//! Collar Daemon - Background service for remote computer control.

mod config;
mod connection;
mod executor;
mod scripts;

use anyhow::Result;
use tracing::info;
use tracing_subscriber::EnvFilter;

use config::Config;
use connection::Daemon;
use scripts::ScriptRegistry;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("collar_daemon=info".parse()?)
        )
        .init();

    info!("Starting Collar Daemon");

    // Load configuration
    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "collar.toml".to_string());

    let config = Config::load(&config_path)?;
    info!("Loaded configuration from {config_path}");

    // Build script registry
    let mut registry = ScriptRegistry::new();
    for script_cfg in &config.scripts {
        registry.register(script_cfg.clone().into());
    }
    info!("Registered {} scripts", config.scripts.len());

    // Run daemon
    let daemon = Daemon::new(config, registry);
    daemon.run().await
}
