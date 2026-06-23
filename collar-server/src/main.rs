//! Collar Server - API and WebSocket hub for remote control.

mod api;
mod auth;
mod config;
mod homekit;
mod persistence;
mod ratelimit;
mod state;
mod ws;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use axum::{middleware, routing::get, Extension, Router};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use config::Config;
use persistence::StatePersister;
use ratelimit::{rate_limit_middleware, RateLimiter};
use state::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("collar_server=info".parse()?)
                .add_directive("tower_http=debug".parse()?),
        )
        .init();

    info!("Starting Collar Server");

    // Load config
    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "server.toml".to_string());

    let config = Config::load(&config_path)?;
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let homekit_enabled = config.homekit.is_some();
    let state_path = config.server.state_path.clone();

    info!("Loaded configuration from {config_path}");

    // Set up persistence if configured
    let persister = match state_path {
        Some(path) => {
            let p = Arc::new(StatePersister::new(path));
            info!(path = %p.path().display(), "State persistence enabled");
            Some(p)
        }
        None => {
            info!("State persistence disabled (no state_path set)");
            None
        }
    };

    // Build state, restoring from disk if available
    let state = AppState::new(config, persister.clone());
    if let Some(p) = &persister {
        match p.load() {
            Ok(persisted) => state.restore(persisted),
            Err(e) => warn!(error = %e, "Failed to load persisted state — starting fresh"),
        }
    }

    // Rate limiter: 100 requests per minute per IP
    let rate_limiter = RateLimiter::new(100, 60);

    // Spawn cleanup task
    let limiter_cleanup = rate_limiter.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            limiter_cleanup.cleanup();
        }
    });

    // Sweep silent daemons every 5s. Daemon pings every 15s, so any device
    // we haven't heard from in 30s has gone dark — likely shut down without
    // a clean WS close. Without this sweep, HomeKit can take minutes to
    // reflect a shutdown.
    let sweeper_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        interval.tick().await; // skip immediate first tick
        loop {
            interval.tick().await;
            sweeper_state
                .disconnect_stale_devices(std::time::Duration::from_secs(30))
                .await;
        }
    });

    // Build API router. The HomeKit subtree mounts only when configured.
    let mut api_router = api::router();
    if homekit_enabled {
        api_router = api_router.nest("/homekit", homekit::router());
        info!("HomeKit integration enabled at /api/homekit");
    }

    // Build root router (CORS handled by nginx)
    let app = Router::new()
        .route("/health", get(|| async { "OK" }))
        .route("/ws", get(ws::ws_handler))
        .nest("/api", api_router)
        .layer(middleware::from_fn(rate_limit_middleware))
        .layer(Extension(rate_limiter))
        .layer(Extension(state.clone()))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    info!("Listening on {addr}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;

    Ok(())
}
