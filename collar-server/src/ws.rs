//! WebSocket handling for device connections.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
};
use collar_common::{DaemonMessage, DeviceId, ScriptInfo, ServerMessage};
use std::net::IpAddr;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::state::AppState;

/// WebSocket upgrade handler.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

/// Handle an individual WebSocket connection.
async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    // Wait for auth message
    let (device_id, device_name, scripts, lan_ip) = match wait_for_auth(&mut receiver, &state).await
    {
        Ok(info) => info,
        Err(e) => {
            warn!("Auth failed: {e}");
            let msg = ServerMessage::AuthResult {
                success: false,
                error: Some(e),
            };
            let _ = sender
                .send(Message::Text(serde_json::to_string(&msg).unwrap()))
                .await;
            return;
        }
    };

    info!(device_id = %device_id, name = %device_name, scripts = scripts.len(), "Device connected");

    // Send auth success
    let msg = ServerMessage::AuthResult {
        success: true,
        error: None,
    };
    if sender
        .send(Message::Text(serde_json::to_string(&msg).unwrap()))
        .await
        .is_err()
    {
        return;
    }

    // Create channel for sending messages to this device
    let (tx, mut rx) = mpsc::channel::<ServerMessage>(32);

    // Register device with scripts + last-known LAN IP for WoL.
    let session_id = state
        .register_device(device_id, device_name.clone(), scripts, lan_ip, tx)
        .await;

    // Spawn sender task
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let text = match serde_json::to_string(&msg) {
                Ok(t) => t,
                Err(e) => {
                    error!("Failed to serialize message: {e}");
                    continue;
                }
            };

            if sender.send(Message::Text(text)).await.is_err() {
                break;
            }
        }
    });

    // Handle incoming messages
    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if let Err(e) = handle_message(&text, device_id, &state).await {
                    error!("Error handling message: {e}");
                }
            }
            Ok(Message::Ping(_)) => {
                // Pong is auto-sent by tungstenite; just keep the device fresh.
                state.touch_device(&device_id);
            }
            Ok(Message::Close(_)) => {
                info!(device_id = %device_id, "Device disconnected");
                break;
            }
            Err(e) => {
                error!("WebSocket error: {e}");
                break;
            }
            _ => {}
        }
    }

    // Cleanup — gated on session_id so a late close from a dead socket
    // can't kick a freshly-reconnected daemon.
    state
        .unregister_device_if_session(&device_id, session_id)
        .await;
    send_task.abort();

    info!(device_id = %device_id, "Device session ended");
}

/// Wait for and validate auth message.
async fn wait_for_auth(
    receiver: &mut futures_util::stream::SplitStream<WebSocket>,
    state: &AppState,
) -> Result<(DeviceId, String, Vec<ScriptInfo>, Option<IpAddr>), String> {
    // Timeout for auth
    let timeout = tokio::time::timeout(std::time::Duration::from_secs(10), receiver.next());

    match timeout.await {
        Ok(Some(Ok(Message::Text(text)))) => {
            let msg: DaemonMessage =
                serde_json::from_str(&text).map_err(|e| format!("Invalid message: {e}"))?;

            match msg {
                DaemonMessage::Auth {
                    device_key,
                    scripts,
                    lan_ip,
                } => {
                    let (id, name) = state
                        .validate_device_key(&device_key)
                        .ok_or_else(|| "Invalid device key".to_string())?;
                    let parsed_ip = lan_ip.as_deref().and_then(|s| s.parse::<IpAddr>().ok());
                    Ok((id, name, scripts, parsed_ip))
                }
                _ => Err("Expected auth message".to_string()),
            }
        }
        Ok(Some(Ok(_))) => Err("Expected text message".to_string()),
        Ok(Some(Err(e))) => Err(format!("WebSocket error: {e}")),
        Ok(None) => Err("Connection closed".to_string()),
        Err(_) => Err("Auth timeout".to_string()),
    }
}

/// Handle an incoming daemon message.
async fn handle_message(text: &str, device_id: DeviceId, state: &AppState) -> anyhow::Result<()> {
    let msg: DaemonMessage = serde_json::from_str(text)?;
    debug!(?msg, "Received message from device");

    match msg {
        DaemonMessage::Status { data } => {
            state.update_status(&device_id, data).await;
        }
        DaemonMessage::CommandResult {
            command_id,
            success,
            output,
            error,
        } => {
            // Command results are logged but not forwarded to frontend yet
            debug!(
                command_id = %command_id,
                success,
                output = ?output,
                error = ?error,
                "Command result received"
            );
        }
        DaemonMessage::Ping => {
            state.touch_device(&device_id);
            // Send pong response back to daemon
            let _ = state.send_to_device(&device_id, ServerMessage::Pong).await;
        }
        DaemonMessage::Auth { .. } => {
            warn!("Unexpected auth message after connection");
        }
    }

    Ok(())
}
