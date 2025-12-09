//! Collar Server - API and WebSocket hub for remote control.

mod api;
mod auth;
mod config;
mod ratelimit;
mod state;
mod ws;

use std::net::SocketAddr;

use anyhow::Result;
use axum::{middleware, routing::get, Extension, Router};
use tower_http::trace::TraceLayer;
use tracing::info;
use tracing_subscriber::EnvFilter;

use config::Config;
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

    info!("Loaded configuration from {config_path}");

    // Build state
    let state = AppState::new(config);

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

    // Build router (CORS handled by nginx)
    let app = Router::new()
        .route("/health", get(|| async { "OK" }))
        .route("/ws", get(ws::ws_handler))
        .nest("/api", api::router())
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
