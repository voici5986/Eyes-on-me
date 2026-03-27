use serde::Serialize;
use url::Url;

use crate::event::AppInfo;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserContext {
    pub family: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    pub source: String,
    pub confidence: f32,
}

#[derive(Debug, Clone)]
struct BrowserDefinition {
    family: &'static str,
    name: &'static str,
    bundle_ids: &'static [&'static str],
    processes: &'static [&'static str],
    app_names: &'static [&'static str],
    #[cfg(target_os = "macos")]
    apple_script_name: Option<&'static str>,
}

const BROWSERS: &[BrowserDefinition] = &[
    BrowserDefinition {
        family: "chromium",
        name: "Google Chrome",
        bundle_ids: &[
            "com.google.chrome",
            "com.google.chrome.canary",
            "org.chromium.chromium",
        ],
        processes: &["chrome.exe", "chrome", "chromium.exe", "chromium"],
        app_names: &["google chrome", "chrome", "chromium"],
        #[cfg(target_os = "macos")]
        apple_script_name: Some("Google Chrome"),
    },
    BrowserDefinition {
        family: "chromium",
        name: "Microsoft Edge",
        bundle_ids: &["com.microsoft.edgemac", "com.microsoft.edge"],
        processes: &["msedge.exe", "msedge"],
        app_names: &["microsoft edge", "edge"],
        #[cfg(target_os = "macos")]
        apple_script_name: Some("Microsoft Edge"),
    },
    BrowserDefinition {
        family: "chromium",
        name: "Brave",
        bundle_ids: &[
            "com.brave.browser",
            "com.brave.browser.beta",
            "com.brave.browser.nightly",
        ],
        processes: &["brave.exe", "brave"],
        app_names: &["brave browser", "brave"],
        #[cfg(target_os = "macos")]
        apple_script_name: Some("Brave Browser"),
    },
    BrowserDefinition {
        family: "chromium",
        name: "Opera",
        bundle_ids: &[
            "com.operasoftware.opera",
            "com.operasoftware.operanext",
            "com.operasoftware.operagx",
        ],
        processes: &["opera.exe", "launcher.exe", "opera", "launcher"],
        app_names: &["opera", "opera gx"],
        #[cfg(target_os = "macos")]
        apple_script_name: Some("Opera"),
    },
    BrowserDefinition {
        family: "chromium",
        name: "Vivaldi",
        bundle_ids: &["com.vivaldi.vivaldi"],
        processes: &["vivaldi.exe", "vivaldi"],
        app_names: &["vivaldi"],
        #[cfg(target_os = "macos")]
        apple_script_name: Some("Vivaldi"),
    },
    BrowserDefinition {
        family: "chromium",
        name: "Arc",
        bundle_ids: &["company.thebrowser.browser"],
        processes: &["arc.exe", "arc"],
        app_names: &["arc"],
        #[cfg(target_os = "macos")]
        apple_script_name: Some("Arc"),
    },
    BrowserDefinition {
        family: "chromium",
        name: "Zen Browser",
        bundle_ids: &["app.zen-browser.zen"],
        processes: &["zen.exe", "zen"],
        app_names: &["zen browser", "zen"],
        #[cfg(target_os = "macos")]
        apple_script_name: None,
    },
    BrowserDefinition {
        family: "firefox",
        name: "Firefox",
        bundle_ids: &[
            "org.mozilla.firefox",
            "org.mozilla.firefoxdeveloperedition",
            "net.waterfox.waterfox",
        ],
        processes: &["firefox.exe", "firefox", "waterfox.exe", "waterfox"],
        app_names: &["mozilla firefox", "firefox", "waterfox"],
        #[cfg(target_os = "macos")]
        apple_script_name: None,
    },
    BrowserDefinition {
        family: "webkit",
        name: "Safari",
        bundle_ids: &["com.apple.safari"],
        processes: &["safari.exe", "safari"],
        app_names: &["safari"],
        #[cfg(target_os = "macos")]
        apple_script_name: Some("Safari"),
    },
    BrowserDefinition {
        family: "webkit",
        name: "Orion",
        bundle_ids: &["com.kagi.kagimacOS"],
        processes: &["orion.exe", "orion"],
        app_names: &["orion"],
        #[cfg(target_os = "macos")]
        apple_script_name: None,
    },
    BrowserDefinition {
        family: "chromium",
        name: "QQ Browser",
        bundle_ids: &[],
        processes: &["qqbrowser.exe", "qqbrowser"],
        app_names: &["qq browser", "qqbrowser", "qq浏览器"],
        #[cfg(target_os = "macos")]
        apple_script_name: None,
    },
    BrowserDefinition {
        family: "chromium",
        name: "360 Browser",
        bundle_ids: &[],
        processes: &["360se.exe", "360chrome.exe", "360se", "360chrome"],
        app_names: &["360 browser", "360se", "360chrome", "360浏览器"],
        #[cfg(target_os = "macos")]
        apple_script_name: None,
    },
    BrowserDefinition {
        family: "chromium",
        name: "Sogou Browser",
        bundle_ids: &[],
        processes: &["sogouexplorer.exe", "sogouexplorer"],
        app_names: &["sogou browser", "sogouexplorer", "搜狗浏览器"],
        #[cfg(target_os = "macos")]
        apple_script_name: None,
    },
];

pub fn detect_browser_context(app: &AppInfo, window_title: Option<&str>) -> Option<BrowserContext> {
    let browser = match_browser(app)?;

    let mut title = infer_page_title(window_title, browser);
    let mut url = None;
    let mut domain = None;
    let mut source = "window-title".to_string();
    let mut confidence: f32 = if title.is_some() { 0.42 } else { 0.18 };

    #[cfg(target_os = "macos")]
    if let Some(mac_page) = read_macos_browser_page(browser) {
        if let Some(page_title) = mac_page.page_title {
            title = Some(page_title);
        }
        url = mac_page.url;
        domain = mac_page.domain;
        source = mac_page.source;
        confidence = mac_page.confidence;
    }

    if url.is_none() {
        url = window_title.and_then(infer_url_from_title);
        if url.is_some() {
            source = "window-title-url".to_string();
            confidence = confidence.max(0.74);
        }
    }

    if domain.is_none() {
        domain = url.as_deref().and_then(url_domain);
    }

    if domain.is_none() {
        domain = title.as_deref().and_then(infer_domain_from_text);
        if domain.is_some() {
            source = "window-title-domain".to_string();
            confidence = confidence.max(0.56);
        }
    }

    Some(BrowserContext {
        family: browser.family.to_string(),
        name: browser.name.to_string(),
        page_title: title,
        url,
        domain,
        source,
        confidence,
    })
}

pub fn page_signature(
    browser: Option<&BrowserContext>,
    window_title: Option<&str>,
) -> Option<String> {
    let browser = browser?;

    browser
        .url
        .as_deref()
        .map(str::to_string)
        .or_else(|| browser.page_title.as_deref().map(str::to_string))
        .or_else(|| window_title.map(str::to_string))
}

fn match_browser(app: &AppInfo) -> Option<&'static BrowserDefinition> {
    let normalized_id = normalize(&app.id);
    let normalized_name = normalize(&app.name);
    let normalized_title = app.title.as_deref().map(normalize);
    let process_name = process_name(&app.id);

    BROWSERS.iter().find(|browser| {
        browser
            .bundle_ids
            .iter()
            .map(|value| normalize(value))
            .any(|value| normalized_id == value)
            || browser
                .processes
                .iter()
                .map(|value| normalize(value))
                .any(|value| normalized_id.ends_with(&value) || process_name == value)
            || browser
                .app_names
                .iter()
                .map(|value| normalize(value))
                .any(|value| {
                    normalized_name == value
                        || normalized_title
                            .as_deref()
                            .map(|title| title == value.as_str())
                            .unwrap_or(false)
                })
    })
}

fn infer_page_title(window_title: Option<&str>, browser: &BrowserDefinition) -> Option<String> {
    let raw = window_title?.trim();
    if raw.is_empty() {
        return None;
    }

    for separator in [" - ", " — ", " – ", " | ", " · "] {
        if let Some((head, tail)) = raw.rsplit_once(separator) {
            let normalized_tail = normalize(tail);
            if tail_matches_browser(&normalized_tail, browser) {
                return clean_page_title(head);
            }
        }

        if let Some((head, tail)) = raw.split_once(separator) {
            let normalized_head = normalize(head);
            if tail_matches_browser(&normalized_head, browser) {
                return clean_page_title(tail);
            }
        }
    }

    clean_page_title(raw)
}

fn tail_matches_browser(value: &str, browser: &BrowserDefinition) -> bool {
    if value == normalize(browser.name) || value.contains(&normalize(browser.name)) {
        return true;
    }

    browser
        .app_names
        .iter()
        .any(|candidate| value.contains(&normalize(candidate)))
}

fn clean_page_title(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    if normalize_possible_url(trimmed).is_some() {
        return None;
    }

    Some(trimmed.to_string())
}

fn url_domain(value: &str) -> Option<String> {
    let parsed = Url::parse(value).ok()?;
    parsed
        .domain()
        .or_else(|| parsed.host_str())
        .map(str::to_string)
}

fn infer_url_from_title(window_title: &str) -> Option<String> {
    let title = window_title.trim();
    if title.is_empty() {
        return None;
    }

    if let Some(url) = title
        .split_whitespace()
        .next()
        .and_then(normalize_possible_url)
    {
        return Some(url);
    }

    for separator in [" - ", " — ", " – ", " | ", " · "] {
        for part in title.rsplit(separator) {
            if let Some(url) = normalize_possible_url(part) {
                return Some(url);
            }
        }
    }

    infer_domain_from_text(title).and_then(|domain| normalize_possible_url(&domain))
}

fn infer_domain_from_text(value: &str) -> Option<String> {
    value
        .split(|ch: char| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    '|' | '—' | '–' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';'
                )
        })
        .find_map(domain_from_candidate)
}

fn domain_from_candidate(candidate: &str) -> Option<String> {
    let trimmed = trim_url_candidate(candidate)
        .trim_matches(|ch: char| ch.is_control() || matches!(ch, '\u{200b}' | '\u{feff}' | '。'))
        .trim_end_matches('.');
    if trimmed.is_empty() || trimmed.contains(' ') {
        return None;
    }

    normalize_possible_url(trimmed).and_then(|url| url_domain(&url))
}

fn normalize_possible_url(value: &str) -> Option<String> {
    let candidate = trim_url_candidate(value)
        .trim_matches(|ch: char| ch.is_control() || matches!(ch, '\u{200b}' | '\u{feff}'))
        .trim_end_matches('.');

    if candidate.is_empty() || candidate.contains(' ') {
        return None;
    }

    let candidate_lower = candidate.to_ascii_lowercase();
    if candidate_lower.starts_with("http://") || candidate_lower.starts_with("https://") {
        return Some(candidate.to_string());
    }

    if candidate.contains("://")
        || candidate_lower.starts_with("about:")
        || candidate_lower.starts_with("chrome:")
        || candidate_lower.starts_with("edge:")
        || candidate_lower.starts_with("file:")
    {
        return Some(candidate.to_string());
    }

    let (host, _) = split_host_and_rest(candidate);
    if is_probable_host(host) {
        let host_without_port = split_host_port(host).0;
        let scheme = if host_without_port.eq_ignore_ascii_case("localhost")
            || is_probable_ipv4(host_without_port)
        {
            "http://"
        } else {
            "https://"
        };
        return Some(format!("{}{}", scheme, candidate.trim_end_matches('/')));
    }

    None
}

fn trim_url_candidate(value: &str) -> &str {
    value.trim().trim_matches(|ch: char| {
        matches!(
            ch,
            '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | ',' | ';'
        )
    })
}

fn split_host_and_rest(value: &str) -> (&str, &str) {
    if let Some(index) = value.find(|ch| ['/', '?', '#'].contains(&ch)) {
        (&value[..index], &value[index..])
    } else {
        (value, "")
    }
}

fn split_host_port(value: &str) -> (&str, Option<&str>) {
    if let Some(index) = value.rfind(':') {
        let host = &value[..index];
        let port = &value[index + 1..];
        if !host.is_empty() && !port.is_empty() && port.chars().all(|ch| ch.is_ascii_digit()) {
            return (host, Some(port));
        }
    }

    (value, None)
}

fn is_probable_ipv4(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    if parts.len() != 4 {
        return false;
    }

    parts.iter().all(|part| {
        !part.is_empty()
            && part.len() <= 3
            && part.chars().all(|ch| ch.is_ascii_digit())
            && part.parse::<u8>().is_ok()
    })
}

fn is_probable_host(value: &str) -> bool {
    let host = value.trim().trim_end_matches('.');
    if host.is_empty() {
        return false;
    }

    if host.contains("://") || host.contains('@') {
        return false;
    }

    let (host_without_port, _) = split_host_port(host);
    host_without_port.eq_ignore_ascii_case("localhost")
        || is_probable_ipv4(host_without_port)
        || looks_like_domain(host_without_port)
}

fn looks_like_domain(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    if parts.len() < 2 || parts.iter().any(|part| part.is_empty()) {
        return false;
    }

    let suffix = parts
        .last()
        .copied()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(
        suffix.as_str(),
        "rs" | "vue" | "tsx" | "ts" | "js" | "json" | "md" | "txt" | "pdf" | "html" | "css"
    ) {
        return false;
    }

    suffix.len() >= 2 && suffix.chars().all(|ch| ch.is_ascii_alphabetic())
}

fn normalize(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn process_name(value: &str) -> String {
    value
        .rsplit(['/', '\\'])
        .next()
        .map(normalize)
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
struct MacBrowserPage {
    page_title: Option<String>,
    url: Option<String>,
    domain: Option<String>,
    source: String,
    confidence: f32,
}

#[cfg(target_os = "macos")]
fn read_macos_browser_page(browser: &BrowserDefinition) -> Option<MacBrowserPage> {
    let app_name = browser.apple_script_name?;
    let script_lines = if browser.family == "webkit" {
        vec![
            format!("tell application \"{app_name}\""),
            "if it is not running then return \"\"".to_string(),
            "if (count of windows) = 0 then return \"\"".to_string(),
            "set tabTitle to name of current tab of front window".to_string(),
            "set tabUrl to URL of current tab of front window".to_string(),
            "return tabTitle & \"|||AMI|||\" & tabUrl".to_string(),
            "end tell".to_string(),
        ]
    } else {
        vec![
            format!("tell application \"{app_name}\""),
            "if it is not running then return \"\"".to_string(),
            "if (count of windows) = 0 then return \"\"".to_string(),
            "set activeTabRef to active tab of front window".to_string(),
            "set tabTitle to title of activeTabRef".to_string(),
            "set tabUrl to URL of activeTabRef".to_string(),
            "return tabTitle & \"|||AMI|||\" & tabUrl".to_string(),
            "end tell".to_string(),
        ]
    };

    let mut command = std::process::Command::new("osascript");
    for line in script_lines {
        command.arg("-e").arg(line);
    }

    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return None;
    }

    let (page_title, url) = trimmed
        .split_once("|||AMI|||")
        .map(|(title, url)| (clean_page_title(title), clean_url(url)))
        .unwrap_or((clean_page_title(trimmed), None));
    let domain = url.as_deref().and_then(url_domain);
    let has_domain = domain.is_some();

    Some(MacBrowserPage {
        page_title,
        url,
        domain,
        source: "applescript".to_string(),
        confidence: if has_domain { 0.94 } else { 0.78 },
    })
}

#[cfg(target_os = "macos")]
fn clean_url(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    Url::parse(trimmed).ok().map(|url| url.to_string())
}
