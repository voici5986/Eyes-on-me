#![cfg_attr(
    not(any(target_os = "macos", target_os = "windows")),
    allow(dead_code, unused_imports)
)]

use anyhow::Result;
use tokio::sync::mpsc;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod browser;
mod config;
mod event;
mod idle;
mod platform;
mod screen_lock;
mod transport;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[allow(dead_code)]
const UNSUPPORTED_PLATFORM_NOTICE: &str =
    "client-desktop foreground watcher is not implemented for this platform yet";

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();

    let cfg = config::Config::from_prompt()?;
    info!(
        server_api_base_url = %cfg.server_api_base_url,
        device_id = %cfg.device_id,
        agent_name = %cfg.agent_name,
        "agent starting"
    );

    let (tx, rx) = mpsc::unbounded_channel();

    let _transport_task = tokio::spawn(transport::run_transport(
        cfg.server_api_base_url.clone(),
        cfg.api_token.clone(),
        rx,
    ));

    platform::run_foreground_watcher(cfg.device_id.clone(), cfg.agent_name.clone(), tx)?;
    Ok(())
}
