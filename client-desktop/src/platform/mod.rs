use std::{thread, time::Duration};

use tokio::sync::mpsc::{self, error::TrySendError};
use tracing::warn;
use url::Url;

use crate::browser::BrowserContext;
use crate::config::{CaptureFilters, PrivacyMode};
use crate::event::{ActivityEnvelope, AppInfo};

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "macos")]
pub(crate) mod macos_native;

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "macos")]
pub use macos::run_foreground_watcher;

#[cfg(target_os = "windows")]
pub use windows::run_foreground_watcher;

#[cfg(target_os = "linux")]
pub use linux::run_foreground_watcher;

#[cfg(not(test))]
const CHANNEL_BACKPRESSURE_RETRY: Duration = Duration::from_millis(200);
#[cfg(test)]
const CHANNEL_BACKPRESSURE_RETRY: Duration = Duration::from_millis(1);

#[cfg(not(test))]
const CHANNEL_BACKPRESSURE_MAX_RETRIES: u32 = 15;
#[cfg(test)]
const CHANNEL_BACKPRESSURE_MAX_RETRIES: u32 = 2;

pub(crate) fn send_activity(
    tx: &mpsc::Sender<ActivityEnvelope>,
    mut event: ActivityEnvelope,
) -> bool {
    for _ in 0..CHANNEL_BACKPRESSURE_MAX_RETRIES {
        match tx.try_send(event) {
            Ok(()) => return true,
            Err(TrySendError::Full(returned)) => {
                event = returned;
                thread::sleep(CHANNEL_BACKPRESSURE_RETRY);
            }
            Err(TrySendError::Closed(_)) => {
                warn!("event channel closed, dropping event");
                return false;
            }
        }
    }

    warn!("event channel full after retries, dropping event");
    false
}

pub(crate) fn accessibility_permission_status() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        if macos_native::accessibility_is_trusted() {
            "granted"
        } else {
            "missing"
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        "not_required"
    }
}

pub(crate) struct FilteredCapture {
    pub app: AppInfo,
    pub window_title: Option<String>,
    pub browser: Option<BrowserContext>,
    pub mode: PrivacyMode,
}

pub(crate) fn apply_capture_filters(
    filters: &CaptureFilters,
    mut app: AppInfo,
    mut window_title: Option<String>,
    browser: Option<BrowserContext>,
) -> FilteredCapture {
    let mode = most_private(
        matching_app_mode(filters, &app).unwrap_or(filters.default_mode),
        matching_domain_mode(filters, browser.as_ref()).unwrap_or(PrivacyMode::Record),
    );
    match mode {
        PrivacyMode::Record => FilteredCapture {
            app,
            window_title,
            browser,
            mode,
        },
        PrivacyMode::Anonymize => {
            let app_name = app.name.clone();
            app.id = format!("privacy.anonymized:{}", normalize_match_name(&app_name));
            app.title = None;
            app.pid = None;
            window_title = Some("Private Activity".to_string());
            FilteredCapture {
                app,
                window_title,
                browser: None,
                mode,
            }
        }
        PrivacyMode::Skip => {
            app.title = None;
            FilteredCapture {
                app,
                window_title: None,
                browser: None,
                mode,
            }
        }
    }
}

pub(crate) fn normalize_app_info(mut app: AppInfo) -> AppInfo {
    let original_name = app.name.trim().to_string();
    let normalized_name = normalize_display_app_name(&original_name);

    if app.title.as_deref().map(str::trim) == Some(original_name.as_str()) {
        app.title = Some(normalized_name.clone());
    }

    app.name = normalized_name;
    app
}

fn matching_app_mode(filters: &CaptureFilters, app: &AppInfo) -> Option<PrivacyMode> {
    let normalized_name = normalize_match_name(&app.name);
    let normalized_id =
        normalize_match_name(app.id.rsplit(['/', '\\']).next().unwrap_or(app.id.as_str()));

    filters
        .app_rules
        .iter()
        .filter_map(|rule| {
            let normalized_rule = normalize_match_name(&rule.pattern);
            (!normalized_rule.is_empty()
                && (normalized_name.contains(&normalized_rule)
                    || normalized_rule.contains(&normalized_name)
                    || normalized_id.contains(&normalized_rule)
                    || normalized_rule.contains(&normalized_id)))
            .then_some(rule.mode)
        })
        .reduce(most_private)
}

fn matching_domain_mode(
    filters: &CaptureFilters,
    browser: Option<&BrowserContext>,
) -> Option<PrivacyMode> {
    let target_domain = browser.and_then(|browser| {
        browser
            .domain
            .as_deref()
            .and_then(normalize_domain_rule)
            .or_else(|| browser.url.as_deref().and_then(normalize_domain_rule))
    });

    let target_domain = target_domain?;

    filters
        .domain_rules
        .iter()
        .filter_map(|rule| {
            let rule_domain = normalize_domain_rule(&rule.pattern)?;
            (target_domain == rule_domain || target_domain.ends_with(&format!(".{rule_domain}")))
                .then_some(rule.mode)
        })
        .reduce(most_private)
}

fn most_private(left: PrivacyMode, right: PrivacyMode) -> PrivacyMode {
    match (left, right) {
        (PrivacyMode::Skip, _) | (_, PrivacyMode::Skip) => PrivacyMode::Skip,
        (PrivacyMode::Anonymize, _) | (_, PrivacyMode::Anonymize) => PrivacyMode::Anonymize,
        _ => PrivacyMode::Record,
    }
}

fn normalize_match_name(value: &str) -> String {
    normalize_display_app_name(value)
        .trim()
        .to_ascii_lowercase()
}

fn normalize_domain_rule(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(parsed) = Url::parse(trimmed) {
        return parsed
            .domain()
            .or_else(|| parsed.host_str())
            .map(|domain| domain.to_ascii_lowercase());
    }

    let without_scheme = trimmed
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let host = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .trim();
    let host = host.rsplit_once('@').map(|(_, host)| host).unwrap_or(host);
    let host = host
        .rsplit_once(':')
        .filter(|(_, port)| port.chars().all(|ch| ch.is_ascii_digit()))
        .map(|(host, _)| host)
        .unwrap_or(host)
        .trim_matches('.');

    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

pub(crate) fn is_system_process(app_name: &str) -> bool {
    let name_lower = app_name.trim().to_lowercase();
    let name_lower = name_lower.trim_end_matches(".exe");

    matches!(
        name_lower,
        "desktop"
            | "lockapp"
            | "logonui"
            | "searchapp"
            | "searchhost"
            | "shellexperiencehost"
            | "startmenuexperiencehost"
            | "textinputhost"
            | "applicationframehost"
            | "dwm"
            | "csrss"
            | "taskmgr"
            | "loginwindow"
            | "screensaverengine"
            | "screensaver"
            | "cinnamon-session"
            | "cinnamon-screensaver"
            | "gnome-shell"
            | "gnome-screensaver"
            | "plasmashell"
            | "kscreenlocker"
            | "xscreensaver"
            | "i3lock"
            | "swaylock"
            | "xfce4-session"
    )
}

pub(crate) fn normalize_display_app_name(app_name: &str) -> String {
    let trimmed = app_name
        .trim()
        .trim_end_matches(".exe")
        .trim_end_matches(".EXE")
        .trim();
    let normalized = trimmed.to_lowercase();

    match normalized.as_str() {
        "chrome" | "google chrome" => "Google Chrome".to_string(),
        "msedge" | "edge" | "microsoft edge" => "Microsoft Edge".to_string(),
        "brave" | "brave browser" => "Brave Browser".to_string(),
        "firefox" | "mozilla firefox" => "Firefox".to_string(),
        "safari" => "Safari".to_string(),
        "opera" | "opera gx" => "Opera".to_string(),
        "vivaldi" => "Vivaldi".to_string(),
        "chromium" => "Chromium".to_string(),
        "arc" => "Arc".to_string(),
        "zen browser" | "zen" => "Zen Browser".to_string(),
        "orion" => "Orion".to_string(),
        "qqbrowser" | "qq browser" | "qq浏览器" => "QQ Browser".to_string(),
        "360se" | "360chrome" | "360 browser" | "360浏览器" => "360 Browser".to_string(),
        "sogouexplorer" | "sogou browser" | "搜狗浏览器" => "Sogou Browser".to_string(),
        "code" | "visual studio code" => "Visual Studio Code".to_string(),
        "cursor" => "Cursor".to_string(),
        "wechat" => "WeChat".to_string(),
        _ => trimmed.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use eyes_on_me_shared::PresenceState;

    use crate::{
        browser::BrowserContext,
        config::{CaptureFilters, PrivacyMode, PrivacyRule},
        event::{ActivityEnvelope, AppInfo},
    };

    use super::{
        apply_capture_filters, is_system_process, normalize_display_app_name, send_activity,
    };

    #[test]
    fn normalizes_common_app_names() {
        assert_eq!(normalize_display_app_name("chrome.exe"), "Google Chrome");
        assert_eq!(normalize_display_app_name("msedge"), "Microsoft Edge");
        assert_eq!(normalize_display_app_name("code"), "Visual Studio Code");
    }

    #[test]
    fn detects_system_processes() {
        assert!(is_system_process("ScreenSaverEngine"));
        assert!(is_system_process("dwm.exe"));
        assert!(!is_system_process("Google Chrome"));
    }

    #[test]
    fn drops_event_when_channel_stays_full() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        tx.try_send(sample_event()).expect("fill channel");

        assert!(!send_activity(&tx, sample_event()));

        let _ = rx.try_recv().expect("original event remains buffered");
    }

    #[test]
    fn filters_ignored_apps_to_generic_activity() {
        let filtered = apply_capture_filters(
            &CaptureFilters {
                app_rules: vec![PrivacyRule {
                    pattern: "WeChat".to_string(),
                    mode: PrivacyMode::Anonymize,
                }],
                ..CaptureFilters::default()
            },
            AppInfo {
                id: "/Applications/WeChat.app".to_string(),
                name: "WeChat".to_string(),
                title: Some("Team Chat".to_string()),
                pid: Some(42),
            },
            Some("Team Chat".to_string()),
            None,
        );

        assert_eq!(filtered.app.name, "WeChat");
        assert_eq!(filtered.window_title.as_deref(), Some("Private Activity"));
        assert!(filtered.browser.is_none());
    }

    #[test]
    fn filters_ignored_domains_without_hiding_browser_app() {
        let filtered = apply_capture_filters(
            &CaptureFilters {
                domain_rules: vec![PrivacyRule {
                    pattern: "github.com".to_string(),
                    mode: PrivacyMode::Anonymize,
                }],
                ..CaptureFilters::default()
            },
            AppInfo {
                id: "com.google.Chrome".to_string(),
                name: "Google Chrome".to_string(),
                title: Some("GitHub".to_string()),
                pid: Some(42),
            },
            Some("GitHub - Pull Requests".to_string()),
            Some(BrowserContext {
                family: "chromium".to_string(),
                name: "Google Chrome".to_string(),
                page_title: Some("GitHub - Pull Requests".to_string()),
                url: Some("https://docs.github.com/en".to_string()),
                domain: Some("docs.github.com".to_string()),
                source: "test".to_string(),
                confidence: 0.9,
            }),
        );

        assert_eq!(filtered.app.name, "Google Chrome");
        assert_eq!(filtered.window_title.as_deref(), Some("Private Activity"));
        assert!(filtered.browser.is_none());
    }

    #[test]
    fn chooses_the_strictest_matching_privacy_rule() {
        let filtered = apply_capture_filters(
            &CaptureFilters {
                app_rules: vec![
                    PrivacyRule {
                        pattern: "Chrome".into(),
                        mode: PrivacyMode::Anonymize,
                    },
                    PrivacyRule {
                        pattern: "Google Chrome".into(),
                        mode: PrivacyMode::Skip,
                    },
                ],
                ..CaptureFilters::default()
            },
            AppInfo {
                id: "com.google.Chrome".into(),
                name: "Google Chrome".into(),
                title: Some("Private".into()),
                pid: Some(42),
            },
            Some("Private".into()),
            None,
        );
        assert_eq!(filtered.mode, PrivacyMode::Skip);
        assert!(filtered.window_title.is_none());
    }

    fn sample_event() -> ActivityEnvelope {
        ActivityEnvelope::activity(
            "device-1",
            "client-desktop",
            "macos",
            "test",
            "activity_sample",
            AppInfo {
                id: "com.google.Chrome".to_string(),
                name: "Google Chrome".to_string(),
                title: Some("GitHub".to_string()),
                pid: Some(42),
            },
            Some("GitHub".to_string()),
            None,
            PresenceState::Active,
        )
    }
}
