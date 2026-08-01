use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::{
    fs,
    sync::{Notify, mpsc},
    time::sleep,
};
use tracing::{info, warn};

use crate::{
    config::{Config, ScreenshotConfig, SpoolConfig},
    event::ActivityEnvelope,
    screenshot::CapturedScreenshot,
};

pub async fn run_diagnostics(
    config: Config,
    recording_enabled: Arc<AtomicBool>,
    control_revision: Arc<AtomicU64>,
) -> Result<()> {
    let client = Client::builder().timeout(Duration::from_secs(10)).build()?;
    let spool = DeliverySpool::new(config.spool_dir.clone(), &config.spool);
    let endpoint = format!("{}/api/agent/diagnostics", config.server_api_base_url);
    loop {
        let payload = eyes_on_me_shared::AgentDiagnostics {
            device_id: config.device_id.clone(),
            agent_name: config.agent_name.clone(),
            platform: std::env::consts::OS.to_string(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            accessibility_permission: crate::platform::accessibility_permission_status()
                .to_string(),
            screen_capture_permission: crate::screenshot::permission_status().to_string(),
            screenshot_enabled: config.screenshots.enabled,
            screenshot_format: format!("{:?}", config.screenshots.format).to_ascii_lowercase(),
            screenshot_display: format!("{:?}", config.screenshots.display).to_ascii_lowercase(),
            privacy_rule_count: config.capture_filters.app_rules.len()
                + config.capture_filters.domain_rules.len(),
            spool_pending: spool.pending_count().await,
            recording_enabled: recording_enabled.load(Ordering::Acquire),
            control_revision: control_revision.load(Ordering::Acquire),
        };
        match client
            .post(&endpoint)
            .bearer_auth(&config.api_token)
            .json(&payload)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {}
            Ok(response) => warn!(status = %response.status(), "agent diagnostics upload failed"),
            Err(error) => warn!(error = %error, "agent diagnostics upload failed"),
        }
        sleep(Duration::from_secs(60)).await;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentControlCache {
    enabled: bool,
    revision: u64,
}

pub fn load_control_cache(config: &Config) -> (bool, u64) {
    let path = control_cache_path(config);
    std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<AgentControlCache>(&bytes).ok())
        .map(|cache| (cache.enabled, cache.revision))
        .unwrap_or((true, 0))
}

pub async fn run_agent_control(
    config: Config,
    recording_enabled: Arc<AtomicBool>,
    control_revision: Arc<AtomicU64>,
) -> Result<()> {
    let client = Client::builder().timeout(Duration::from_secs(10)).build()?;
    let endpoint = format!(
        "{}/api/agent/control/{}",
        config.server_api_base_url,
        url::form_urlencoded::byte_serialize(config.device_id.as_bytes()).collect::<String>()
    );
    loop {
        match client
            .get(&endpoint)
            .bearer_auth(&config.api_token)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                let control = response
                    .json::<eyes_on_me_shared::AgentControlResponse>()
                    .await?;
                let desired = control.recording.desired_enabled;
                let revision = control.recording.revision;
                let changed = recording_enabled.swap(desired, Ordering::AcqRel) != desired
                    || control_revision.swap(revision, Ordering::AcqRel) != revision;
                if changed {
                    info!(enabled = desired, revision, "recording control applied");
                    persist_control_cache(&config, desired, revision).await?;
                }
                if control.recording.applied_enabled != Some(desired)
                    || control.recording.acknowledged_at.is_none()
                {
                    let response = client
                        .post(&endpoint)
                        .bearer_auth(&config.api_token)
                        .json(&serde_json::json!({
                            "enabled": desired,
                            "revision": revision
                        }))
                        .send()
                        .await?;
                    if !response.status().is_success() {
                        warn!(status = %response.status(), "recording control acknowledgement failed");
                    }
                }
            }
            Ok(response) if response.status() == StatusCode::UNAUTHORIZED => {
                bail!("server rejected agent token while polling recording control");
            }
            Ok(response) => {
                warn!(status = %response.status(), "recording control poll failed; retaining last applied state");
            }
            Err(error) => {
                warn!(error = %error, "recording control unavailable; retaining last applied state");
            }
        }
        sleep(Duration::from_secs(5)).await;
    }
}

fn control_cache_path(config: &Config) -> PathBuf {
    config
        .spool_dir
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("client-desktop.control.json")
}

async fn persist_control_cache(config: &Config, enabled: bool, revision: u64) -> Result<()> {
    let path = control_cache_path(config);
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).await?;
    let temporary = path.with_extension("json.tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(&AgentControlCache { enabled, revision })?,
    )
    .await?;
    fs::rename(temporary, path).await?;
    Ok(())
}

const MANIFEST_FILE: &str = "delivery.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SpoolScreenshot {
    file_name: String,
    mime_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SpoolManifest {
    event: ActivityEnvelope,
    screenshot: Option<SpoolScreenshot>,
}

struct SpoolItem {
    path: PathBuf,
    manifest: SpoolManifest,
}

#[derive(Clone)]
pub struct DeliverySpool {
    root: Arc<PathBuf>,
    max_bytes: u64,
    changed: Arc<Notify>,
}

impl DeliverySpool {
    pub fn new(root: PathBuf, config: &SpoolConfig) -> Self {
        Self {
            root: Arc::new(root),
            max_bytes: config.max_bytes,
            changed: Arc::new(Notify::new()),
        }
    }

    async fn persist(
        &self,
        event: ActivityEnvelope,
        screenshot: Option<CapturedScreenshot>,
    ) -> Result<()> {
        fs::create_dir_all(self.root.as_path()).await?;
        let estimated = serde_json::to_vec(&event)?.len() as u64
            + screenshot
                .as_ref()
                .map(|item| item.bytes.len() as u64)
                .unwrap_or(0);
        let used = directory_size(self.root.as_path()).await?;
        if used.saturating_add(estimated) > self.max_bytes {
            bail!(
                "delivery spool is full ({} MiB / {} MiB)",
                used / 1024 / 1024,
                self.max_bytes / 1024 / 1024
            );
        }

        let event_id = &event.payload.event_id;
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        let temporary = self.root.join(format!(".tmp-{event_id}"));
        let destination = self.root.join(format!("{stamp:020}-{event_id}"));
        if fs::try_exists(&temporary).await.unwrap_or(false) {
            fs::remove_dir_all(&temporary).await?;
        }
        fs::create_dir(&temporary).await?;

        let screenshot_manifest = if let Some(screenshot) = screenshot {
            let extension = if screenshot.mime_type == "image/jpeg" {
                "jpg"
            } else {
                "png"
            };
            let file_name = format!("screenshot.{extension}");
            fs::write(temporary.join(&file_name), screenshot.bytes).await?;
            Some(SpoolScreenshot {
                file_name,
                mime_type: screenshot.mime_type,
            })
        } else {
            None
        };
        let manifest = SpoolManifest {
            event,
            screenshot: screenshot_manifest,
        };
        fs::write(
            temporary.join(MANIFEST_FILE),
            serde_json::to_vec_pretty(&manifest)?,
        )
        .await?;
        fs::rename(&temporary, &destination).await?;
        self.changed.notify_one();
        Ok(())
    }

    async fn oldest(&self) -> Result<Option<SpoolItem>> {
        fs::create_dir_all(self.root.as_path()).await?;
        let mut reader = fs::read_dir(self.root.as_path()).await?;
        let mut paths = Vec::new();
        while let Some(entry) = reader.next_entry().await? {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') || !entry.file_type().await?.is_dir() {
                continue;
            }
            paths.push(entry.path());
        }
        paths.sort();
        let Some(path) = paths.into_iter().next() else {
            return Ok(None);
        };
        let bytes = fs::read(path.join(MANIFEST_FILE))
            .await
            .with_context(|| format!("failed to read spool item {}", path.display()))?;
        let manifest = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse spool item {}", path.display()))?;
        Ok(Some(SpoolItem { path, manifest }))
    }

    async fn remove(&self, item: &SpoolItem) -> Result<()> {
        fs::remove_dir_all(&item.path).await?;
        self.changed.notify_waiters();
        Ok(())
    }

    pub async fn pending_count(&self) -> usize {
        let Ok(mut reader) = fs::read_dir(self.root.as_path()).await else {
            return 0;
        };
        let mut count = 0;
        while let Ok(Some(entry)) = reader.next_entry().await {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            if entry
                .file_type()
                .await
                .map(|kind| kind.is_dir())
                .unwrap_or(false)
            {
                count += 1;
            }
        }
        count
    }
}

pub async fn run_transport(
    server_api_base_url: String,
    api_token: String,
    screenshot_config: ScreenshotConfig,
    spool_config: SpoolConfig,
    spool_dir: PathBuf,
    mut rx: mpsc::Receiver<ActivityEnvelope>,
) -> Result<()> {
    let spool = DeliverySpool::new(spool_dir, &spool_config);
    let intake_closed = Arc::new(AtomicBool::new(false));
    let uploader = tokio::spawn(run_uploader(
        server_api_base_url,
        api_token,
        spool.clone(),
        Arc::clone(&intake_closed),
    ));
    let mut last_screenshot_at: Option<Instant> = None;

    while let Some(event) = rx.recv().await {
        let screenshot = if should_capture_screenshot(
            &event,
            &screenshot_config,
            last_screenshot_at,
        ) {
            match crate::screenshot::capture(&screenshot_config, &event).await {
                Ok(captured) => {
                    last_screenshot_at = Some(Instant::now());
                    Some(captured)
                }
                Err(error) => {
                    warn!(event_id = event.payload.event_id, error = %error, "screenshot capture failed; cooldown was not advanced");
                    None
                }
            }
        } else {
            None
        };

        let mut pending = Some((event, screenshot));
        loop {
            let (event, screenshot) = pending.take().expect("pending spool delivery");
            match spool.persist(event.clone(), screenshot.clone()).await {
                Ok(()) => break,
                Err(error) => {
                    warn!(event_id = event.payload.event_id, error = %error, "cannot persist delivery; applying backpressure");
                    pending = Some((event, screenshot));
                    sleep(Duration::from_secs(2)).await;
                }
            }
        }
    }

    intake_closed.store(true, Ordering::Release);
    spool.changed.notify_waiters();
    uploader
        .await
        .map_err(|error| anyhow!("uploader task failed: {error}"))?
}

async fn run_uploader(
    server_api_base_url: String,
    api_token: String,
    spool: DeliverySpool,
    intake_closed: Arc<AtomicBool>,
) -> Result<()> {
    let client = Client::builder().timeout(Duration::from_secs(15)).build()?;
    let max_retry_delay = Duration::from_secs(30);
    let mut retry_delay = Duration::from_secs(2);

    loop {
        let Some(item) = spool.oldest().await? else {
            if intake_closed.load(Ordering::Acquire) {
                return Ok(());
            }
            spool.changed.notified().await;
            continue;
        };
        let event = &item.manifest.event;
        let endpoint = endpoint_for(&server_api_base_url, &event.message_type)?;
        let result = async {
            send_event(&client, &endpoint, &api_token, event).await?;
            if let Some(screenshot) = &item.manifest.screenshot {
                let bytes = fs::read(item.path.join(&screenshot.file_name)).await?;
                upload_screenshot(
                    &client,
                    &server_api_base_url,
                    &api_token,
                    &event.payload.event_id,
                    &screenshot.mime_type,
                    bytes,
                )
                .await?;
            }
            Result::<()>::Ok(())
        }
        .await;

        match result {
            Ok(()) => {
                spool.remove(&item).await?;
                retry_delay = Duration::from_secs(2);
            }
            Err(error) => {
                warn!(event_id = event.payload.event_id, error = %error, delay_secs = retry_delay.as_secs(), "delivery failed; durable spool item retained");
                sleep(retry_delay).await;
                retry_delay = std::cmp::min(max_retry_delay, retry_delay.saturating_mul(2));
            }
        }
    }
}

fn should_capture_screenshot(
    event: &ActivityEnvelope,
    config: &ScreenshotConfig,
    last_capture: Option<Instant>,
) -> bool {
    config.enabled
        && event.message_type == "activity"
        && event.payload.kind == "foreground_changed"
        && event.payload.presence == eyes_on_me_shared::PresenceState::Active
        && !event.payload.app.id.starts_with("privacy.anonymized:")
        && last_capture
            .map(|at| at.elapsed() >= Duration::from_secs(config.cooldown_secs))
            .unwrap_or(true)
}

async fn upload_screenshot(
    client: &Client,
    server_api_base_url: &str,
    api_token: &str,
    event_id: &str,
    mime_type: &str,
    bytes: Vec<u8>,
) -> Result<()> {
    let endpoint = format!("{server_api_base_url}/api/agent/screenshots/{event_id}");
    let response = client
        .post(endpoint)
        .bearer_auth(api_token)
        .header(reqwest::header::CONTENT_TYPE, mime_type)
        .body(bytes.clone())
        .send()
        .await?;
    if response.status().is_success() {
        info!(
            event_id,
            byte_size = bytes.len(),
            mime_type,
            "screenshot uploaded"
        );
        return Ok(());
    }
    let status = response.status();
    let detail = response.text().await.unwrap_or_default();
    Err(anyhow!(
        "screenshot upload returned HTTP {status}: {detail}"
    ))
}

fn endpoint_for(server_api_base_url: &str, message_type: &str) -> Result<String> {
    let path = match message_type {
        "activity" => "/api/agent/activity",
        "status" => "/api/agent/status",
        other => return Err(anyhow!("unsupported agent message type: {other}")),
    };
    Ok(format!("{server_api_base_url}{path}"))
}

async fn send_event(
    client: &Client,
    endpoint: &str,
    api_token: &str,
    event: &ActivityEnvelope,
) -> Result<()> {
    let response = client
        .post(endpoint)
        .bearer_auth(api_token)
        .json(event)
        .send()
        .await?;
    if response.status().is_success() {
        info!(endpoint, app_id = %event.payload.app.id, "event sent");
        return Ok(());
    }
    let status = response.status();
    let detail = response.text().await.unwrap_or_default();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        bail!("server rejected agent token: HTTP {status} {detail}");
    }
    Err(anyhow!("server returned HTTP {status}: {detail}"))
}

async fn directory_size(root: &Path) -> Result<u64> {
    let mut total = 0u64;
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let mut reader = match fs::read_dir(directory).await {
            Ok(reader) => reader,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        while let Some(entry) = reader.next_entry().await? {
            let metadata = entry.metadata().await?;
            if metadata.is_dir() {
                directories.push(entry.path());
            } else {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    Ok(total)
}
