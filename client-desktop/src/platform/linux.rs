use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use amiokay_shared::PresenceState;
use anyhow::{Result, anyhow, bail};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::browser::{BrowserContext, detect_browser_context, page_signature};
use crate::event::{ActivityEnvelope, AppInfo};
use crate::idle;

const SAMPLE_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq)]
struct LastApp {
    window_id: String,
    pid: i32,
    page_signature: Option<String>,
}

#[derive(Debug, Clone)]
struct ForegroundApp {
    window_id: String,
    app: AppInfo,
    window_title: Option<String>,
}

#[derive(Debug, Clone)]
struct LastSentState {
    marker: LastApp,
    app: AppInfo,
    window_title: Option<String>,
    browser: Option<BrowserContext>,
    presence: PresenceState,
    sent_at: Instant,
}

pub fn run_foreground_watcher(
    device_id: String,
    agent_name: String,
    tx: mpsc::UnboundedSender<ActivityEnvelope>,
) -> Result<()> {
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        bail!("client-desktop Linux watcher requires an active desktop session");
    }
    if !command_available("xprop") {
        bail!("client-desktop Linux watcher requires `xprop` in PATH");
    }

    info!("foreground watcher started (Linux xprop polling)");

    let mut last_sent = None::<LastSentState>;
    let mut last_read_error_at = None::<Instant>;

    loop {
        let presence = if idle::is_idle(idle::DEFAULT_IDLE_TIMEOUT_SECS) {
            PresenceState::Idle
        } else {
            PresenceState::Active
        };

        let current = match current_foreground_app() {
            Some(current) => {
                last_read_error_at = None;
                current
            }
            None => {
                let now = Instant::now();
                let should_log = last_read_error_at
                    .map(|at| now.duration_since(at) >= Duration::from_secs(30))
                    .unwrap_or(true);
                if should_log {
                    warn!("cannot read frontmost app on Linux");
                    last_read_error_at = Some(now);
                }
                last_sent
                    .as_ref()
                    .map(previous_as_foreground)
                    .unwrap_or_else(|| synthetic_foreground_app(presence))
            }
        };

        let browser = stabilize_browser_context(
            detect_browser_context(&current.app, current.window_title.as_deref()),
            last_sent.as_ref(),
            &current.app,
            current.window_title.as_deref(),
        );
        let marker = LastApp {
            window_id: current.window_id.clone(),
            pid: current.app.pid,
            page_signature: page_signature(browser.as_ref(), current.window_title.as_deref()),
        };

        let now = Instant::now();
        let marker_changed = last_sent
            .as_ref()
            .map(|state| state.marker != marker)
            .unwrap_or(true);
        let presence_changed = last_sent
            .as_ref()
            .map(|state| state.presence != presence)
            .unwrap_or(true);
        let sample_due = last_sent
            .as_ref()
            .map(|state| now.duration_since(state.sent_at) >= SAMPLE_INTERVAL)
            .unwrap_or(true);

        if marker_changed || presence_changed || sample_due {
            let kind = if marker_changed {
                "foreground_changed"
            } else if presence_changed {
                "presence_changed"
            } else {
                "activity_sample"
            };

            info!(
                app_name = %current.app.name,
                app_id = %current.app.id,
                pid = current.app.pid,
                presence = ?presence,
                kind,
                "activity sampled"
            );

            let event = ActivityEnvelope::activity(
                &device_id,
                &agent_name,
                "linux",
                "xprop",
                kind,
                current.app.clone(),
                current.window_title.clone(),
                browser.clone(),
                presence,
            );
            if let Err(err) = tx.send(event) {
                warn!(error = %err, "event channel closed, dropping event");
            }

            last_sent = Some(LastSentState {
                marker,
                app: current.app,
                window_title: current.window_title,
                browser,
                presence,
                sent_at: now,
            });
        }

        thread::sleep(Duration::from_secs(1));
    }
}

fn stabilize_browser_context(
    browser: Option<BrowserContext>,
    previous: Option<&LastSentState>,
    app: &AppInfo,
    window_title: Option<&str>,
) -> Option<BrowserContext> {
    let same_window = previous
        .map(|state| {
            state.app.id == app.id
                && state.window_title.as_deref() == window_title
                && state.browser.is_some()
        })
        .unwrap_or(false);

    match (browser, previous.and_then(|state| state.browser.clone())) {
        (Some(mut current), Some(prev)) if same_window => {
            if current.url.is_none() {
                current.url = prev.url.clone();
            }
            if current.domain.is_none() {
                current.domain = prev.domain.clone();
            }
            if current.page_title.is_none() {
                current.page_title = prev.page_title.clone();
            }
            Some(current)
        }
        (Some(current), _) => Some(current),
        (None, Some(prev)) if same_window => Some(prev),
        (None, _) => None,
    }
}

fn current_foreground_app() -> Option<ForegroundApp> {
    let window_id = active_window_id()?;
    let pid = window_pid(&window_id)?;
    let window_title = window_title(&window_id);
    let app_id = process_commandline(pid).or_else(|| process_name(pid))?;
    let app_name = process_name(pid).unwrap_or_else(|| fallback_name(&app_id));
    let pid = i32::try_from(pid).unwrap_or(i32::MAX);

    Some(ForegroundApp {
        window_id,
        app: AppInfo {
            id: app_id,
            name: app_name,
            title: window_title.clone(),
            pid,
        },
        window_title,
    })
}

fn previous_as_foreground(previous: &LastSentState) -> ForegroundApp {
    ForegroundApp {
        window_id: previous.marker.window_id.clone(),
        app: previous.app.clone(),
        window_title: previous.window_title.clone(),
    }
}

fn synthetic_foreground_app(presence: PresenceState) -> ForegroundApp {
    let (id, name) = match presence {
        PresenceState::Locked => ("system.locked", "Locked Screen"),
        PresenceState::Idle => ("system.idle", "Idle"),
        PresenceState::Active => ("system.unknown", "Unknown"),
    };

    ForegroundApp {
        window_id: id.to_string(),
        app: AppInfo {
            id: id.to_string(),
            name: name.to_string(),
            title: Some(name.to_string()),
            pid: -1,
        },
        window_title: None,
    }
}

fn active_window_id() -> Option<String> {
    let output = run_command("xprop", &["-root", "_NET_ACTIVE_WINDOW"])?;
    let marker = output.split('#').nth(1)?.trim();
    if marker.is_empty() || marker == "0x0" {
        return None;
    }
    Some(marker.to_string())
}

fn window_pid(window_id: &str) -> Option<u32> {
    let output = run_command("xprop", &["-id", window_id, "_NET_WM_PID"])?;
    output.split('=').nth(1)?.trim().parse::<u32>().ok()
}

fn window_title(window_id: &str) -> Option<String> {
    let output = run_command("xprop", &["-id", window_id, "_NET_WM_NAME", "WM_NAME"])?;

    for line in output.lines() {
        if let Some(title) = quoted_value(line) {
            return Some(title);
        }
    }

    None
}

fn quoted_value(line: &str) -> Option<String> {
    let start = line.find('"')?;
    let end = line.rfind('"')?;
    if end <= start {
        return None;
    }
    let value = &line[start + 1..end];
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.replace("\\\"", "\""))
}

fn process_commandline(pid: u32) -> Option<String> {
    let output = run_command("ps", &["-p", &pid.to_string(), "-o", "args="])?;
    let command = output.lines().next()?.trim();
    if command.is_empty() {
        return None;
    }

    let first = command.split_whitespace().next().unwrap_or(command);
    Some(first.to_string())
}

fn process_name(pid: u32) -> Option<String> {
    let output = run_command("ps", &["-p", &pid.to_string(), "-o", "comm="])?;
    let value = output.lines().next()?.trim();
    if value.is_empty() {
        return None;
    }
    Some(fallback_name(value))
}

fn fallback_name(value: &str) -> String {
    Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string())
        .unwrap_or_else(|| value.to_string())
}

fn command_available(name: &str) -> bool {
    Command::new(name)
        .arg("--help")
        .output()
        .map(|output| {
            output.status.success() || !output.stdout.is_empty() || !output.stderr.is_empty()
        })
        .unwrap_or(false)
}

fn run_command(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }

    String::from_utf8(output.stdout)
        .ok()
        .map(|stdout| stdout.trim().to_string())
        .filter(|stdout| !stdout.is_empty())
}

#[allow(dead_code)]
fn command_error(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program).args(args).output()?;
    if !output.status.success() {
        return Err(anyhow!("command failed: {program} {}", args.join(" ")));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
