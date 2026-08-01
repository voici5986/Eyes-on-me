use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use block2::RcBlock;
use eyes_on_me_shared::PresenceState;
use objc2::rc::autoreleasepool;
use objc2_app_kit::{
    NSRunningApplication, NSWorkspace, NSWorkspaceDidActivateApplicationNotification,
};
use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSNotification, NSRunLoop};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::browser::{
    BrowserContext, NativeBrowserPage, detect_browser_context_for_macos, is_browser_app,
    page_signature,
};
use crate::config::{CaptureFilters, PrivacyMode};
use crate::event::{ActivityEnvelope, AppInfo};
use crate::platform::macos_native::{
    AccessibilityEventMonitor, log_accessibility_status, read_window_snapshot,
    take_accessibility_event,
};
use crate::platform::{
    apply_capture_filters, is_system_process, normalize_app_info, send_activity,
};
use crate::{idle, screen_lock};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const TAB_FALLBACK_INTERVAL: Duration = Duration::from_secs(5);
const PRESENCE_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const AX_EVENT_COALESCE_INTERVAL: Duration = Duration::from_millis(500);
const RUN_LOOP_SLICE_SECS: f64 = 0.2;
const LOG_THROTTLE_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
struct LastApp {
    bundle_id: String,
    pid: Option<u32>,
    page_signature: Option<String>,
}

#[derive(Debug, Clone)]
struct ForegroundApp {
    app: AppInfo,
    window_title: Option<String>,
    native_browser_page: Option<NativeBrowserPage>,
    source: &'static str,
}

#[derive(Debug, Clone)]
struct LastSentState {
    marker: LastApp,
    app: AppInfo,
    window_title: Option<String>,
    browser: Option<BrowserContext>,
    presence: PresenceState,
    sent_at: Instant,
    source: &'static str,
}

pub fn run_foreground_watcher(
    device_id: String,
    agent_name: String,
    capture_filters: CaptureFilters,
    recording_enabled: Arc<AtomicBool>,
    tx: mpsc::Sender<ActivityEnvelope>,
) -> Result<()> {
    let mut last_sent = None::<LastSentState>;
    let mut last_read_error_at = None::<Instant>;
    let mut accessibility_monitor = AccessibilityEventMonitor::new();
    let workspace_event_pending = Arc::new(AtomicBool::new(true));

    log_accessibility_status();

    autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let notification_center = workspace.notificationCenter();

        let block_pending = Arc::clone(&workspace_event_pending);
        let observer = RcBlock::new(move |_notification: NonNull<NSNotification>| {
            block_pending.store(true, Ordering::Release);
        });
        let _observer_token = unsafe {
            notification_center.addObserverForName_object_queue_usingBlock(
                Some(NSWorkspaceDidActivateApplicationNotification),
                None,
                None,
                &observer,
            )
        };

        info!(
            tab_fallback_secs = TAB_FALLBACK_INTERVAL.as_secs(),
            heartbeat_secs = HEARTBEAT_INTERVAL.as_secs(),
            "foreground watcher started (macOS native notifications + low-frequency fallback)"
        );

        let run_loop = NSRunLoop::currentRunLoop();
        let mut presence = current_presence();
        let mut next_presence_check = Instant::now();
        let mut next_fallback = Instant::now();
        let mut accessibility_event_deferred = false;
        let mut last_accessibility_sample_at = None::<Instant>;
        let mut was_recording = recording_enabled.load(Ordering::Acquire);

        loop {
            let until = NSDate::dateWithTimeIntervalSinceNow(RUN_LOOP_SLICE_SECS);
            let mode = unsafe { NSDefaultRunLoopMode };
            let _ = run_loop.runMode_beforeDate(mode, &until);

            let is_recording = recording_enabled.load(Ordering::Acquire);
            if !is_recording {
                if was_recording {
                    last_sent = None;
                    accessibility_monitor.bind(None);
                }
                was_recording = false;
                continue;
            }
            if !was_recording {
                workspace_event_pending.store(true, Ordering::Release);
            }
            was_recording = true;

            let now = Instant::now();
            let workspace_changed = workspace_event_pending.swap(false, Ordering::AcqRel);
            accessibility_event_deferred |= take_accessibility_event();
            let accessibility_event_due = accessibility_event_deferred
                && last_accessibility_sample_at
                    .map(|sampled_at| now.duration_since(sampled_at) >= AX_EVENT_COALESCE_INTERVAL)
                    .unwrap_or(true);
            let mut should_sample = workspace_changed || accessibility_event_due;

            if now >= next_presence_check {
                let next_presence = current_presence();
                should_sample |= next_presence != presence;
                presence = next_presence;
                next_presence_check = now + PRESENCE_CHECK_INTERVAL;
            }

            let heartbeat_due = last_sent
                .as_ref()
                .map(|state| now.duration_since(state.sent_at) >= HEARTBEAT_INTERVAL)
                .unwrap_or(true);
            let fallback_due = now >= next_fallback;
            should_sample |= heartbeat_due || fallback_due;

            if !should_sample {
                continue;
            }

            if accessibility_event_due {
                accessibility_event_deferred = false;
                last_accessibility_sample_at = Some(now);
            }

            let foreground_pid = emit_sample(
                &device_id,
                &agent_name,
                &capture_filters,
                &tx,
                &mut last_sent,
                &mut last_read_error_at,
                presence,
            );
            accessibility_monitor.bind(foreground_pid);

            let fallback_interval = if last_sent
                .as_ref()
                .map(|state| needs_tab_fallback(&state.app))
                .unwrap_or(false)
            {
                TAB_FALLBACK_INTERVAL
            } else {
                HEARTBEAT_INTERVAL
            };
            next_fallback = Instant::now() + fallback_interval;
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn emit_sample(
    device_id: &str,
    agent_name: &str,
    capture_filters: &CaptureFilters,
    tx: &mpsc::Sender<ActivityEnvelope>,
    last_sent: &mut Option<LastSentState>,
    last_read_error_at: &mut Option<Instant>,
    presence: PresenceState,
) -> Option<i32> {
    let previous = last_sent.clone();
    let now = Instant::now();

    let mut current = match current_foreground_app() {
        Some(current) => {
            *last_read_error_at = None;
            current
        }
        None => {
            throttle_read_error(last_read_error_at);
            previous
                .as_ref()
                .map(previous_as_foreground)
                .unwrap_or_else(|| synthetic_foreground_app(presence))
        }
    };
    let observer_pid = current.app.pid.and_then(|pid| i32::try_from(pid).ok());

    if is_system_process(&current.app.name) {
        current = synthetic_foreground_app(presence);
    }

    let app_changed = previous
        .as_ref()
        .map(|state| state.app.id != current.app.id || state.app.pid != current.app.pid)
        .unwrap_or(true);
    let browser = if app_changed || is_browser_app(&current.app) {
        stabilize_browser_context(
            detect_browser_context_for_macos(
                &current.app,
                current.window_title.as_deref(),
                current.native_browser_page.clone(),
            ),
            previous.as_ref(),
            &current.app,
            current.window_title.as_deref(),
        )
    } else {
        previous.as_ref().and_then(|state| state.browser.clone())
    };

    let filtered = apply_capture_filters(
        capture_filters,
        current.app.clone(),
        current.window_title.clone(),
        browser,
    );
    let marker = LastApp {
        bundle_id: filtered.app.id.clone(),
        pid: filtered.app.pid,
        page_signature: page_signature(filtered.browser.as_ref(), filtered.window_title.as_deref()),
    };

    if filtered.mode == PrivacyMode::Skip {
        *last_sent = Some(LastSentState {
            marker,
            app: filtered.app,
            window_title: None,
            browser: None,
            presence,
            sent_at: now,
            source: current.source,
        });
        return observer_pid;
    }

    let marker_changed = previous
        .as_ref()
        .map(|state| state.marker != marker)
        .unwrap_or(true);
    let presence_changed = previous
        .as_ref()
        .map(|state| state.presence != presence)
        .unwrap_or(true);
    let heartbeat_due = previous
        .as_ref()
        .map(|state| now.duration_since(state.sent_at) >= HEARTBEAT_INTERVAL)
        .unwrap_or(true);
    if !(marker_changed || presence_changed || heartbeat_due) {
        return observer_pid;
    }

    let kind = if marker_changed {
        "foreground_changed"
    } else if presence_changed {
        "presence_changed"
    } else {
        "activity_sample"
    };
    if marker_changed || presence_changed {
        info!(
            app_name = %filtered.app.name,
            bundle_id = %filtered.app.id,
            pid = ?filtered.app.pid,
            window_title = filtered.window_title.as_deref().unwrap_or("n/a"),
            presence = ?presence,
            source = current.source,
            kind,
            "activity sampled"
        );
    }

    let event = ActivityEnvelope::activity(
        device_id,
        agent_name,
        "macos",
        current.source,
        kind,
        filtered.app.clone(),
        filtered.window_title.clone(),
        filtered.browser.clone(),
        presence,
    );
    if !send_activity(tx, event) {
        return observer_pid;
    }

    *last_sent = Some(LastSentState {
        marker,
        app: filtered.app,
        window_title: filtered.window_title,
        browser: filtered.browser,
        presence,
        sent_at: now,
        source: current.source,
    });
    observer_pid
}

fn current_presence() -> PresenceState {
    if screen_lock::is_locked() {
        PresenceState::Locked
    } else if idle::is_idle(idle::DEFAULT_IDLE_TIMEOUT_SECS) {
        PresenceState::Idle
    } else {
        PresenceState::Active
    }
}

fn throttle_read_error(last_read_error_at: &mut Option<Instant>) {
    let now = Instant::now();
    let should_log = last_read_error_at
        .map(|at| now.duration_since(at) >= LOG_THROTTLE_INTERVAL)
        .unwrap_or(true);
    if should_log {
        warn!("cannot read frontmost app on macOS");
        *last_read_error_at = Some(now);
    }
}

fn current_foreground_app() -> Option<ForegroundApp> {
    autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let app = workspace.frontmostApplication()?;
        let pid = app.processIdentifier();
        if pid <= 0 {
            return None;
        }

        let app_info = app_from_running_app(&app, pid);
        let is_browser = is_browser_app(&app_info);
        let native = read_window_snapshot(pid, is_browser);
        let window_title = normalize_window_title(&app_info, native.window_title);
        let native_browser_page = is_browser.then(|| NativeBrowserPage {
            page_title: None,
            url: native.browser_url.clone(),
            source: native.source,
            confidence: if native.browser_url.is_some() {
                0.96
            } else {
                0.42
            },
        });

        Some(ForegroundApp {
            app: app_info,
            window_title,
            native_browser_page,
            source: native.source,
        })
    })
}

fn app_from_running_app(app: &NSRunningApplication, pid: i32) -> AppInfo {
    let bundle_id = app
        .bundleIdentifier()
        .map(|id| id.to_string())
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| format!("pid:{pid}"));
    let name = app
        .localizedName()
        .map(|name| name.to_string())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| bundle_id.clone());

    normalize_app_info(AppInfo {
        id: bundle_id,
        name: name.clone(),
        title: Some(name),
        pid: u32::try_from(pid).ok(),
    })
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

fn needs_tab_fallback(app: &AppInfo) -> bool {
    if is_browser_app(app) {
        return true;
    }
    is_terminal_app(app)
}

fn is_terminal_app(app: &AppInfo) -> bool {
    let id = app.id.to_ascii_lowercase();
    if [
        "com.apple.terminal",
        "com.googlecode.iterm2",
        "dev.warp.warp-stable",
        "com.github.wez.wezterm",
        "net.kovidgoyal.kitty",
        "org.alacritty",
        "org.tabby",
    ]
    .iter()
    .any(|candidate| id == *candidate)
    {
        return true;
    }

    let name = app.name.to_ascii_lowercase();
    [
        "terminal",
        "iterm",
        "warp",
        "wezterm",
        "kitty",
        "alacritty",
        "tabby",
        "终端",
    ]
    .iter()
    .any(|candidate| name.contains(candidate))
}

fn normalize_window_title(app: &AppInfo, title: Option<String>) -> Option<String> {
    let title = title?;
    if !is_terminal_app(app) {
        return Some(title);
    }

    let without_spinner = title
        .chars()
        .filter(|character| !matches!(*character, '\u{2800}'..='\u{28ff}'))
        .collect::<String>();
    let collapsed = without_spinner
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let without_dimensions = [" — ", " - ", " | "]
        .into_iter()
        .find_map(|separator| {
            let (head, suffix) = collapsed.rsplit_once(separator)?;
            terminal_dimensions(suffix).then(|| head.to_string())
        })
        .unwrap_or(collapsed);
    let normalized = without_dimensions.trim();
    (!normalized.is_empty()).then(|| normalized.to_string())
}

fn terminal_dimensions(value: &str) -> bool {
    let value = value.trim();
    let Some((columns, rows)) = value.split_once('×').or_else(|| value.split_once('x')) else {
        return false;
    };
    !columns.is_empty()
        && !rows.is_empty()
        && columns.chars().all(|character| character.is_ascii_digit())
        && rows.chars().all(|character| character.is_ascii_digit())
}

fn previous_as_foreground(previous: &LastSentState) -> ForegroundApp {
    ForegroundApp {
        app: previous.app.clone(),
        window_title: previous.window_title.clone(),
        native_browser_page: previous.browser.as_ref().map(|browser| NativeBrowserPage {
            page_title: browser.page_title.clone(),
            url: browser.url.clone(),
            source: "cached",
            confidence: browser.confidence,
        }),
        source: previous.source,
    }
}

fn synthetic_foreground_app(presence: PresenceState) -> ForegroundApp {
    let (id, name) = match presence {
        PresenceState::Locked => ("system.locked", "Locked Screen"),
        PresenceState::Idle => ("system.idle", "Idle"),
        PresenceState::Active => ("system.unknown", "Unknown"),
    };

    ForegroundApp {
        app: AppInfo {
            id: id.to_string(),
            name: name.to_string(),
            title: Some(name.to_string()),
            pid: None,
        },
        window_title: None,
        native_browser_page: None,
        source: "macos-native",
    }
}

#[cfg(test)]
mod tests {
    use super::{needs_tab_fallback, normalize_window_title};
    use crate::event::AppInfo;

    fn app(name: &str, id: &str) -> AppInfo {
        AppInfo {
            id: id.to_string(),
            name: name.to_string(),
            title: None,
            pid: Some(1),
        }
    }

    #[test]
    fn browser_and_terminal_apps_keep_low_frequency_fallback() {
        assert!(needs_tab_fallback(&app(
            "Google Chrome",
            "com.google.Chrome"
        )));
        assert!(needs_tab_fallback(&app("Terminal", "com.apple.Terminal")));
        assert!(needs_tab_fallback(&app("iTerm2", "com.googlecode.iterm2")));
        assert!(!needs_tab_fallback(&app("Finder", "com.apple.finder")));
    }

    #[test]
    fn terminal_titles_ignore_spinner_frames_and_window_dimensions() {
        let terminal = app("终端", "com.apple.Terminal");
        let first = normalize_window_title(
            &terminal,
            Some("Eyes_on_me — ⠧ Eyes_on_me — codex — 224×66".to_string()),
        );
        let second = normalize_window_title(
            &terminal,
            Some("Eyes_on_me — ⠙ Eyes_on_me — codex — 224×66".to_string()),
        );

        assert_eq!(first, second);
        assert_eq!(first.as_deref(), Some("Eyes_on_me — Eyes_on_me — codex"));
    }
}
