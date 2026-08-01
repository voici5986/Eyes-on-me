#![cfg_attr(
    not(any(target_os = "macos", target_os = "windows")),
    allow(dead_code, unused_imports)
)]

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64},
};

use anyhow::Result;
use tokio::sync::mpsc;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

mod browser;
mod config;
mod event;
mod idle;
mod platform;
mod screen_lock;
mod screenshot;
mod transport;

const EVENT_CHANNEL_CAPACITY: usize = 256;

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

    let (tx, rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
    let (cached_recording, cached_revision) = transport::load_control_cache(&cfg);
    let recording_enabled = Arc::new(AtomicBool::new(cached_recording));
    let control_revision = Arc::new(AtomicU64::new(cached_revision));

    let _diagnostics_task = tokio::spawn(transport::run_diagnostics(
        cfg.clone(),
        Arc::clone(&recording_enabled),
        Arc::clone(&control_revision),
    ));
    let _control_task = tokio::spawn(transport::run_agent_control(
        cfg.clone(),
        Arc::clone(&recording_enabled),
        Arc::clone(&control_revision),
    ));

    let transport_config = cfg.clone();
    let _transport_task = tokio::spawn(async move {
        if let Err(error) = transport::run_transport(
            transport_config.server_api_base_url,
            transport_config.api_token,
            transport_config.screenshots,
            transport_config.spool,
            transport_config.spool_dir,
            rx,
        )
        .await
        {
            error!(error = %error, "transport task stopped");
        }
    });

    platform::run_foreground_watcher(
        cfg.device_id.clone(),
        cfg.agent_name.clone(),
        cfg.capture_filters.clone(),
        recording_enabled,
        tx,
    )?;
    Ok(())
}
