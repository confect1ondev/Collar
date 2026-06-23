//! HomeKit / Homebridge integration surface.
//!
//! Exposes a small REST + SSE API under `/api/homekit` for a Homebridge
//! plugin to enumerate configured switches, toggle them, and subscribe to
//! live state changes. Authentication is via a dedicated API key in the
//! `Authorization: Bearer …` header — never the user JWT.
//!
//! Switches are declared in `server.toml` under `[[homekit.switches]]` and
//! reference scripts the daemon already declares. The server never invents
//! commands; if the daemon's script registry doesn't contain the referenced
//! script id, the command fails at the daemon (same as the web UI path).

use std::convert::Infallible;
use std::time::Duration;

use axum::{
    async_trait,
    extract::{FromRequestParts, Path, State},
    http::{header::AUTHORIZATION, request::Parts, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use collar_common::{
    HomeKitEvent, HomeKitSetRequest, HomeKitSetResponse, HomeKitSwitchState, ServerMessage,
};
use futures::Stream;
use serde_json::Value;
use subtle::ConstantTimeEq;
use tokio_stream::{wrappers::BroadcastStream, StreamExt};
use tracing::{info, warn};
use uuid::Uuid;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/switches", get(list_switches))
        .route("/switches/:id", get(get_switch))
        .route("/switches/:id/set", post(set_switch))
        .route("/events", get(events))
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

/// Extractor that requires the configured HomeKit API key in the
/// `Authorization: Bearer …` header. Fails closed if `[homekit]` isn't
/// configured at all.
pub struct HomeKitAuth;

#[async_trait]
impl FromRequestParts<AppState> for HomeKitAuth {
    type Rejection = HomeKitAuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let expected = state
            .config
            .homekit
            .as_ref()
            .map(|c| c.api_key.as_str())
            .ok_or(HomeKitAuthError::NotConfigured)?;

        let header = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .ok_or(HomeKitAuthError::Missing)?;

        let presented = header
            .strip_prefix("Bearer ")
            .ok_or(HomeKitAuthError::Missing)?;

        // Constant-time comparison so we don't leak key prefix length via
        // timing. ConstantTimeEq diverges only on the length check itself,
        // which is acceptable — the configured key length is not a secret.
        if presented.as_bytes().ct_eq(expected.as_bytes()).into() {
            Ok(HomeKitAuth)
        } else {
            Err(HomeKitAuthError::Invalid)
        }
    }
}

pub enum HomeKitAuthError {
    NotConfigured,
    Missing,
    Invalid,
}

impl IntoResponse for HomeKitAuthError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::NotConfigured => (
                StatusCode::NOT_FOUND,
                "HomeKit integration is not configured on this server",
            ),
            Self::Missing => (StatusCode::UNAUTHORIZED, "Missing API key"),
            Self::Invalid => (StatusCode::UNAUTHORIZED, "Invalid API key"),
        };
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn list_switches(
    _auth: HomeKitAuth,
    State(state): State<AppState>,
) -> Json<Vec<HomeKitSwitchState>> {
    let out: Vec<HomeKitSwitchState> = state
        .all_switches()
        .iter()
        .filter_map(|cfg| state.build_switch_state(cfg))
        .collect();
    Json(out)
}

async fn get_switch(
    _auth: HomeKitAuth,
    State(state): State<AppState>,
    Path(switch_id): Path<String>,
) -> Result<Json<HomeKitSwitchState>, (StatusCode, Json<Value>)> {
    let cfg = state
        .find_switch(&switch_id)
        .ok_or_else(|| not_found("Switch not found"))?;

    state
        .build_switch_state(&cfg)
        .map(Json)
        .ok_or_else(|| not_found("Switch references an unknown device"))
}

async fn set_switch(
    _auth: HomeKitAuth,
    State(state): State<AppState>,
    Path(switch_id): Path<String>,
    Json(req): Json<HomeKitSetRequest>,
) -> Result<Json<HomeKitSetResponse>, (StatusCode, Json<Value>)> {
    let cfg = state
        .find_switch(&switch_id)
        .ok_or_else(|| not_found("Switch not found"))?;

    let device_id = Uuid::parse_str(&cfg.device_id).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "Switch has invalid device_id" })),
        )
    })?;

    let script_id = if req.on {
        cfg.on_script.clone()
    } else {
        cfg.off_script.clone()
    };

    let command_id = Uuid::new_v4();
    let message = ServerMessage::Execute {
        command_id,
        script_id: script_id.clone(),
        args: None,
    };

    info!(
        switch = %switch_id,
        device = %device_id,
        script = %script_id,
        on = req.on,
        "HomeKit switch set"
    );

    state.send_to_device(&device_id, message).await.map_err(|e| {
        warn!(switch = %switch_id, error = %e, "HomeKit set failed");
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": e })),
        )
    })?;

    Ok(Json(HomeKitSetResponse {
        command_id,
        dispatched_script: script_id,
    }))
}

/// Server-Sent Events stream of `HomeKitEvent`s. The plugin uses this to push
/// switch updates into HomeKit without polling. Per-message format:
/// `event: <type>\ndata: <json>\n\n` where `<type>` is e.g. `switch_updated`.
async fn events(
    _auth: HomeKitAuth,
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.subscribe_homekit();
    let stream = BroadcastStream::new(rx).filter_map(|res| match res {
        Ok(event) => Some(Ok(serialize_event(&event))),
        // Slow consumer fell behind. The plugin will resync via the next
        // /switches poll, so we just drop the event silently here.
        Err(_lagged) => None,
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    )
}

fn serialize_event(event: &HomeKitEvent) -> Event {
    let (kind, data) = match event {
        HomeKitEvent::SwitchUpdated { state } => (
            "switch_updated",
            serde_json::to_string(state).unwrap_or_else(|_| "{}".to_string()),
        ),
        HomeKitEvent::Heartbeat => ("heartbeat", "{}".to_string()),
    };
    Event::default().event(kind).data(data)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn not_found(msg: &str) -> (StatusCode, Json<Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": msg })),
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AuthConfig, Config, HomeKitConfig, HomeKitSwitchConfig, ServerConfig};
    use axum::body::Body;
    use axum::http::{Method, Request};
    use collar_common::DeviceStatus;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    const TEST_DEVICE_ID: &str = "550e8400-e29b-41d4-a716-446655440000";
    const TEST_API_KEY: &str = "test-homekit-key";

    fn test_config() -> Config {
        Config {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 0,
                state_path: None,
            },
            auth: AuthConfig {
                jwt_secret: "test".to_string(),
                jwt_expiry_hours: 1,
                admin_username: "admin".to_string(),
                admin_password_hash: "$argon2id$v=19$m=19456,t=2,p=1$YWJj$YWJj".to_string(),
            },
            devices: vec![crate::config::DeviceKeyConfig {
                device_id: TEST_DEVICE_ID.to_string(),
                name: "Test Device".to_string(),
                api_key: "device-key".to_string(),
                wol_mac: None,
            }],
            homekit: Some(HomeKitConfig {
                api_key: TEST_API_KEY.to_string(),
                switches: vec![HomeKitSwitchConfig {
                    id: "test_lock".to_string(),
                    name: "Test Lock".to_string(),
                    device_id: TEST_DEVICE_ID.to_string(),
                    on_script: "lock".to_string(),
                    off_script: "unlock".to_string(),
                    accessory_type: Default::default(),
                    state_source: Default::default(),
                    state_script: Some("is_locked".to_string()),
                    state_on_value: Some("yes".to_string()),
                }],
            }),
        }
    }

    fn test_app() -> (Router, AppState) {
        let state = AppState::new(test_config(), None);
        let app = Router::new()
            .nest("/api/homekit", router())
            .with_state(state.clone());
        (app, state)
    }

    fn auth_header(value: &str) -> (axum::http::HeaderName, axum::http::HeaderValue) {
        (
            AUTHORIZATION,
            axum::http::HeaderValue::from_str(&format!("Bearer {value}")).unwrap(),
        )
    }

    #[tokio::test]
    async fn unauthorized_without_key() {
        let (app, _) = test_app();
        let req = Request::builder()
            .uri("/api/homekit/switches")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn unauthorized_with_wrong_key() {
        let (app, _) = test_app();
        let (hk, hv) = auth_header("wrong-key");
        let req = Request::builder()
            .uri("/api/homekit/switches")
            .header(hk, hv)
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn list_switches_returns_configured_switch() {
        let (app, _) = test_app();
        let (hk, hv) = auth_header(TEST_API_KEY);
        let req = Request::builder()
            .uri("/api/homekit/switches")
            .header(hk, hv)
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let body = res.into_body().collect().await.unwrap().to_bytes();
        let switches: Vec<HomeKitSwitchState> = serde_json::from_slice(&body).unwrap();
        assert_eq!(switches.len(), 1);
        assert_eq!(switches[0].id, "test_lock");
        assert_eq!(switches[0].name, "Test Lock");
        assert!(!switches[0].device_online, "no daemon connected in test");
        assert_eq!(switches[0].on, None);
        assert_eq!(switches[0].last_observed, None);
    }

    #[tokio::test]
    async fn get_switch_reflects_persisted_status() {
        // Build state with a persisted offline device that has reported is_locked=yes.
        let state = AppState::new(test_config(), None);
        let device_id = Uuid::parse_str(TEST_DEVICE_ID).unwrap();
        let mut status = DeviceStatus::default();
        status.custom.insert(
            "is_locked".to_string(),
            serde_json::Value::String("yes".to_string()),
        );
        let observed = chrono::Utc::now();
        state.offline_devices.insert(
            device_id,
            crate::state::OfflineDevice {
                id: device_id,
                name: "Test Device".to_string(),
                disconnected_at: observed,
                last_status: status,
                status_observed_at: Some(observed),
                scripts: vec![],
                lan_ip: None,
            },
        );

        let app = Router::new()
            .nest("/api/homekit", router())
            .with_state(state.clone());

        let (hk, hv) = auth_header(TEST_API_KEY);
        let req = Request::builder()
            .uri("/api/homekit/switches/test_lock")
            .header(hk, hv)
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let body = res.into_body().collect().await.unwrap().to_bytes();
        let sw: HomeKitSwitchState = serde_json::from_slice(&body).unwrap();
        assert_eq!(sw.on, Some(true), "is_locked=yes should be ON");
        assert!(!sw.device_online);
        assert!(sw.last_observed.is_some());
    }

    #[tokio::test]
    async fn get_unknown_switch_is_404() {
        let (app, _) = test_app();
        let (hk, hv) = auth_header(TEST_API_KEY);
        let req = Request::builder()
            .uri("/api/homekit/switches/does_not_exist")
            .header(hk, hv)
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn set_switch_returns_503_when_device_offline() {
        let (app, _) = test_app();
        let (hk, hv) = auth_header(TEST_API_KEY);
        let req = Request::builder()
            .method(Method::POST)
            .uri("/api/homekit/switches/test_lock/set")
            .header(hk, hv)
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"on": true}"#))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn missing_homekit_config_returns_404() {
        // Same config but homekit absent.
        let mut cfg = test_config();
        cfg.homekit = None;
        let state = AppState::new(cfg, None);
        let app = Router::new()
            .nest("/api/homekit", router())
            .with_state(state);
        let (hk, hv) = auth_header("anything");
        let req = Request::builder()
            .uri("/api/homekit/switches")
            .header(hk, hv)
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn accessory_uuid_is_device_level_and_stable() {
        use collar_common::{homekit_device_accessory_uuid, homekit_service_subtype};
        let device_id = Uuid::parse_str(TEST_DEVICE_ID).unwrap();
        let a = homekit_device_accessory_uuid(&device_id);
        let b = homekit_device_accessory_uuid(&device_id);
        assert_eq!(a, b, "same device must produce same accessory UUID");

        // Service subtypes differ even when accessory UUID is shared.
        let svc_lock = homekit_service_subtype("lock", "unlock", "is_locked");
        let svc_power = homekit_service_subtype("noop", "shutdown", "@online");
        assert_ne!(svc_lock, svc_power);
        assert_eq!(
            svc_lock,
            homekit_service_subtype("lock", "unlock", "is_locked"),
            "same behaviour must produce stable subtype"
        );
    }
}
