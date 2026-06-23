//! REST API routes.

use axum::{
    extract::{Path, State},
    http::{header::SET_COOKIE, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use collar_common::{
    Device, DeviceId, ExecuteRequest, ExecuteResponse, LoginRequest, LoginResponse, ScriptInfo,
    ServerMessage,
};
use tracing::info;
use uuid::Uuid;

use crate::auth::{clear_auth_cookie, create_auth_cookie, create_token, verify_password, AuthUser};
use crate::state::AppState;

/// Build the API router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/devices", get(list_devices))
        .route("/devices/:id", get(get_device))
        .route("/devices/:id/command", post(execute_command))
        .route("/devices/:id/status", get(get_status))
        .route("/devices/:id/refresh", post(refresh_status))
        .route("/devices/:id/scripts", get(get_scripts))
}

/// Login endpoint.
async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    // Verify credentials
    if req.username != state.config.auth.admin_username {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Invalid credentials" })),
        ));
    }

    if !verify_password(&req.password, &state.config.auth.admin_password_hash) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Invalid credentials" })),
        ));
    }

    // Create token
    let token = create_token(
        &req.username,
        &state.config.auth.jwt_secret,
        state.config.auth.jwt_expiry_hours,
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
    })?;

    let expires_at = chrono::Utc::now()
        + chrono::TimeDelta::hours(state.config.auth.jwt_expiry_hours as i64);

    // Create httpOnly cookie
    let cookie = create_auth_cookie(&token, state.config.auth.jwt_expiry_hours);

    Ok((
        [(SET_COOKIE, cookie)],
        Json(LoginResponse { token, expires_at }),
    ))
}

/// Logout endpoint - clears the auth cookie.
async fn logout() -> impl IntoResponse {
    let cookie = clear_auth_cookie();
    ([(SET_COOKIE, cookie)], Json(serde_json::json!({ "success": true })))
}

/// List all connected devices.
async fn list_devices(
    _user: AuthUser,
    State(state): State<AppState>,
) -> Json<Vec<Device>> {
    Json(state.list_devices())
}

/// Get a specific device.
async fn get_device(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Device>, (StatusCode, Json<serde_json::Value>)> {
    let device_id = parse_device_id(&id)?;

    state.get_device(&device_id).map(Json).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Device not found" })),
        )
    })
}

/// Execute a command on a device.
async fn execute_command(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ExecuteRequest>,
) -> Result<Json<ExecuteResponse>, (StatusCode, Json<serde_json::Value>)> {
    let device_id = parse_device_id(&id)?;

    // Check device exists
    if state.get_device(&device_id).is_none() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Device not found" })),
        ));
    }

    let command_id = Uuid::new_v4();

    // Send command to device
    let message = ServerMessage::Execute {
        command_id,
        script_id: req.script_id,
        args: req.args,
    };

    state
        .send_to_device(&device_id, message)
        .await
        .map_err(|e| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": e })),
            )
        })?;

    Ok(Json(ExecuteResponse { command_id }))
}

/// Get device status.
async fn get_status(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let device_id = parse_device_id(&id)?;

    let device = state.get_device(&device_id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Device not found" })),
        )
    })?;

    Ok(Json(device.status))
}

/// Request immediate status refresh from daemon.
async fn refresh_status(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    info!("Refresh status request for device: {}", id);
    let device_id = parse_device_id(&id)?;

    // Check device exists
    if state.get_device(&device_id).is_none() {
        info!("Device not found: {}", id);
        return Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Device not found" })),
        ));
    }

    // Send RequestStatus to daemon
    info!("Sending RequestStatus to daemon for device: {}", id);
    state
        .send_to_device(&device_id, ServerMessage::RequestStatus)
        .await
        .map_err(|e| {
            info!("Failed to send to device: {}", e);
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": e })),
            )
        })?;

    info!("RequestStatus sent successfully");
    Ok(Json(serde_json::json!({ "success": true })))
}

/// Get device scripts.
async fn get_scripts(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<ScriptInfo>>, (StatusCode, Json<serde_json::Value>)> {
    let device_id = parse_device_id(&id)?;

    state.get_device_scripts(&device_id).map(Json).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Device not found" })),
        )
    })
}

fn parse_device_id(s: &str) -> Result<DeviceId, (StatusCode, Json<serde_json::Value>)> {
    Uuid::parse_str(s).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid device ID" })),
        )
    })
}
