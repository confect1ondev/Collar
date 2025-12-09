//! WebSocket connection management.

use anyhow::{Context, Result};
use collar_common::{CommandId, DaemonMessage, DeviceStatus, ScriptId, ScriptInfo, ServerMessage};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{interval, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::executor::{self, ExecutionResult};
use crate::scripts::ScriptRegistry;

const RECONNECT_DELAY: Duration = Duration::from_secs(5);
const PING_INTERVAL: Duration = Duration::from_secs(15);
const PONG_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);

/// Events from the connection to the main loop.
#[derive(Debug)]
pub enum ConnectionEvent {
    Connected,
    Disconnected,
    ExecuteScript {
        command_id: CommandId,
        script_id: ScriptId,
        args: Option<Vec<String>>,
    },
    RequestStatus,
}

/// Commands from main loop to connection.
#[derive(Debug)]
pub enum ConnectionCommand {
    SendResult {
        command_id: CommandId,
        success: bool,
        output: Option<String>,
        error: Option<String>,
    },
    SendStatus(DeviceStatus),
}

/// Manages the WebSocket connection to the server.
pub struct ConnectionManager {
    config: Arc<Config>,
    scripts: Vec<ScriptInfo>,
    event_tx: mpsc::Sender<ConnectionEvent>,
    command_rx: mpsc::Receiver<ConnectionCommand>,
}

impl ConnectionManager {
    pub fn new(
        config: Arc<Config>,
        scripts: Vec<ScriptInfo>,
        event_tx: mpsc::Sender<ConnectionEvent>,
        command_rx: mpsc::Receiver<ConnectionCommand>,
    ) -> Self {
        Self {
            config,
            scripts,
            event_tx,
            command_rx,
        }
    }

    /// Run the connection loop with auto-reconnect.
    pub async fn run(mut self) {
        loop {
            match self.connect_and_handle().await {
                Ok(()) => {
                    info!("Connection closed gracefully");
                }
                Err(e) => {
                    error!("Connection error: {e:#}");
                }
            }

            let _ = self.event_tx.send(ConnectionEvent::Disconnected).await;
            info!("Reconnecting in {} seconds...", RECONNECT_DELAY.as_secs());
            tokio::time::sleep(RECONNECT_DELAY).await;
        }
    }

    async fn connect_and_handle(&mut self) -> Result<()> {
        let url = &self.config.server.url;
        info!("Connecting to {url}");

        let (ws_stream, _) = timeout(CONNECTION_TIMEOUT, connect_async(url))
            .await
            .context("Connection timeout - server may be unreachable")?
            .context("Failed to connect to server")?;

        let (mut write, mut read) = ws_stream.split();

        // Send auth message with available scripts
        let auth_msg = DaemonMessage::Auth {
            device_key: self.config.server.device_key.clone(),
            scripts: self.scripts.clone(),
        };
        write
            .send(Message::Text(serde_json::to_string(&auth_msg)?))
            .await?;

        // Wait for auth response
        let auth_response = timeout(Duration::from_secs(10), read.next())
            .await
            .context("Auth timeout")?
            .ok_or_else(|| anyhow::anyhow!("Connection closed during auth"))?
            .context("Failed to receive auth response")?;

        if let Message::Text(text) = auth_response {
            let msg: ServerMessage = serde_json::from_str(&text)?;
            match msg {
                ServerMessage::AuthResult { success, error } => {
                    if !success {
                        anyhow::bail!("Auth failed: {}", error.unwrap_or_default());
                    }
                }
                _ => anyhow::bail!("Unexpected message during auth"),
            }
        }

        info!("Connected and authenticated");
        let _ = self.event_tx.send(ConnectionEvent::Connected).await;

        // Main message loop
        let mut ping_interval = interval(PING_INTERVAL);
        ping_interval.tick().await; // Skip the immediate first tick
        let mut awaiting_pong = false;

        loop {
            tokio::select! {
                // Handle incoming messages
                msg = read.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            // Check if it's a pong before delegating to handle_server_message
                            if let Ok(server_msg) = serde_json::from_str::<ServerMessage>(&text) {
                                if matches!(server_msg, ServerMessage::Pong) {
                                    awaiting_pong = false;
                                    debug!("Received pong from server");
                                    continue;
                                }
                            }
                            self.handle_server_message(&text).await?;
                        }
                        Some(Ok(Message::Ping(data))) => {
                            write.send(Message::Pong(data)).await?;
                        }
                        Some(Ok(Message::Pong(_))) => {
                            awaiting_pong = false;
                        }
                        Some(Ok(Message::Close(_))) => {
                            info!("Server closed connection");
                            return Ok(());
                        }
                        Some(Err(e)) => {
                            return Err(e.into());
                        }
                        None => {
                            return Ok(());
                        }
                        _ => {}
                    }
                }

                // Handle outgoing commands
                cmd = self.command_rx.recv() => {
                    match cmd {
                        Some(ConnectionCommand::SendResult { command_id, success, output, error }) => {
                            let msg = DaemonMessage::CommandResult {
                                command_id,
                                success,
                                output,
                                error,
                            };
                            write.send(Message::Text(serde_json::to_string(&msg)?)).await?;
                        }
                        Some(ConnectionCommand::SendStatus(status)) => {
                            let msg = DaemonMessage::Status { data: status };
                            write.send(Message::Text(serde_json::to_string(&msg)?)).await?;
                        }
                        None => {
                            info!("Command channel closed");
                            return Ok(());
                        }
                    }
                }

                // Periodic ping
                _ = ping_interval.tick() => {
                    if awaiting_pong {
                        // Connection is stale - no pong received for the previous ping
                        // Return error to trigger reconnection
                        anyhow::bail!("Connection stale: no pong received within {} seconds", PING_INTERVAL.as_secs());
                    }
                    let msg = DaemonMessage::Ping;
                    write.send(Message::Text(serde_json::to_string(&msg)?)).await?;
                    awaiting_pong = true;
                }
            }
        }
    }

    async fn handle_server_message(&self, text: &str) -> Result<()> {
        let msg: ServerMessage = serde_json::from_str(text)?;
        debug!(?msg, "Received server message");

        match msg {
            ServerMessage::Execute {
                command_id,
                script_id,
                args,
            } => {
                self.event_tx
                    .send(ConnectionEvent::ExecuteScript {
                        command_id,
                        script_id,
                        args,
                    })
                    .await?;
            }
            ServerMessage::RequestStatus => {
                info!("Received status request from server");
                self.event_tx.send(ConnectionEvent::RequestStatus).await?;
            }
            ServerMessage::Pong => {
                debug!("Received pong");
            }
            ServerMessage::AuthResult { .. } => {
                warn!("Unexpected auth result after connection");
            }
        }

        Ok(())
    }
}

/// Main daemon runner that coordinates connection and script execution.
pub struct Daemon {
    config: Arc<Config>,
    scripts: ScriptRegistry,
}

impl Daemon {
    pub fn new(config: Config, scripts: ScriptRegistry) -> Self {
        Self {
            config: Arc::new(config),
            scripts,
        }
    }

    pub async fn run(self) -> Result<()> {
        let (event_tx, mut event_rx) = mpsc::channel::<ConnectionEvent>(32);
        let (command_tx, command_rx) = mpsc::channel::<ConnectionCommand>(32);

        // Collect script info to send to server
        let script_infos: Vec<ScriptInfo> = self.scripts.all().map(ScriptInfo::from).collect();

        // Spawn connection manager
        let conn_manager = ConnectionManager::new(
            Arc::clone(&self.config),
            script_infos,
            event_tx,
            command_rx,
        );
        tokio::spawn(conn_manager.run());

        // Status polling - only poll when connected
        let poll_interval = Duration::from_secs(self.config.polling.interval_secs);
        let mut poll_timer = interval(poll_interval);
        let mut connected = false;

        loop {
            tokio::select! {
                event = event_rx.recv() => {
                    match event {
                        Some(ConnectionEvent::Connected) => {
                            info!("Connected to server");
                            connected = true;
                            // Immediately send status on connect so the server has current state
                            let status = self.collect_status().await;
                            let _ = command_tx.send(ConnectionCommand::SendStatus(status)).await;
                            // Reset the poll timer so we don't double-poll
                            poll_timer.reset();
                        }
                        Some(ConnectionEvent::Disconnected) => {
                            warn!("Disconnected from server");
                            connected = false;
                        }
                        Some(ConnectionEvent::ExecuteScript { command_id, script_id, args }) => {
                            // Get the command string, then spawn execution in background
                            // so it doesn't block the event loop
                            if let Some(script) = self.scripts.get(&script_id) {
                                let command = script.command.clone();
                                let tx = command_tx.clone();
                                let sid = script_id.clone();
                                tokio::spawn(async move {
                                    info!(script_id = %sid, "Executing script");
                                    let result = match executor::execute(&command, args.as_deref()).await {
                                        Ok(r) => r,
                                        Err(e) => ExecutionResult {
                                            success: false,
                                            exit_code: None,
                                            stdout: String::new(),
                                            stderr: e.to_string(),
                                        },
                                    };
                                    let _ = tx.send(ConnectionCommand::SendResult {
                                        command_id,
                                        success: result.success,
                                        output: Some(result.stdout),
                                        error: if result.stderr.is_empty() { None } else { Some(result.stderr) },
                                    }).await;
                                });
                            } else {
                                error!(script_id, "Script not found");
                                let _ = command_tx.send(ConnectionCommand::SendResult {
                                    command_id,
                                    success: false,
                                    output: None,
                                    error: Some(format!("Script not found: {script_id}")),
                                }).await;
                            }
                        }
                        Some(ConnectionEvent::RequestStatus) => {
                            info!("Collecting and sending status");
                            let status = self.collect_status().await;
                            let _ = command_tx.send(ConnectionCommand::SendStatus(status)).await;
                        }
                        None => break,
                    }
                }

                _ = poll_timer.tick() => {
                    // Only send status updates when connected
                    if connected {
                        let status = self.collect_status().await;
                        let _ = command_tx.send(ConnectionCommand::SendStatus(status)).await;
                    }
                }
            }
        }

        Ok(())
    }

    async fn collect_status(&self) -> DeviceStatus {
        let mut status = DeviceStatus::default();

        for script_id in &self.config.polling.status_scripts {
            if let Some(script) = self.scripts.get(script_id) {
                // Timeout status scripts to prevent blocking the event loop
                // (scripts may hang waiting for D-Bus, display server, etc. at boot)
                let result = timeout(
                    Duration::from_secs(5),
                    executor::execute(&script.command, None),
                )
                .await;

                match result {
                    Ok(Ok(exec_result)) => {
                        let output = exec_result.stdout.trim();
                        status.custom.insert(
                            script_id.to_string(),
                            serde_json::Value::String(output.to_string()),
                        );
                    }
                    Ok(Err(e)) => {
                        warn!(script_id, error = %e, "Status script failed");
                    }
                    Err(_) => {
                        warn!(script_id, "Status script timed out after 5 seconds");
                    }
                }
            }
        }

        status
    }
}
