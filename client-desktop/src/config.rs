use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use tracing::{error, warn};
use url::Url;

pub const DESKTOP_AGENT_NAME: &str = "client-desktop";
const CONFIG_FILE_NAME: &str = "client-desktop.config.json";

#[cfg(target_os = "macos")]
const DEFAULT_DEVICE_ID: &str = "macos-agent";
#[cfg(target_os = "windows")]
const DEFAULT_DEVICE_ID: &str = "windows-agent";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const DEFAULT_DEVICE_ID: &str = "client-desktop";

#[derive(Debug, Clone)]
pub struct Config {
    pub server_api_base_url: String,
    pub device_id: String,
    pub agent_name: String,
    pub api_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredConfig {
    #[serde(alias = "server_ws_url")]
    server_api_base_url: String,
    device_id: String,
    #[serde(default = "default_agent_name")]
    agent_name: String,
    api_token: String,
}

impl Config {
    pub fn from_prompt() -> Result<Self> {
        let config_path = resolve_config_path();
        let stored_config = load_stored_config(&config_path);
        let no_prompt = env::var("AGENT_NO_PROMPT")
            .ok()
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false);

        #[cfg(target_os = "windows")]
        {
            let stored_config = stored_config.unwrap_or_else(|| {
                let config = StoredConfig {
                    server_api_base_url: default_server_api_base_url(),
                    device_id: default_device_id(),
                    agent_name: default_agent_name(),
                    api_token: default_agent_api_token(),
                };
                save_stored_config(&config_path, &config);
                config
            });

            return Ok(Self {
                server_api_base_url: normalize_server_api_base_url(
                    stored_config.server_api_base_url,
                )?,
                device_id: stored_config.device_id,
                agent_name: stored_config.agent_name,
                api_token: stored_config.api_token,
            });
        }

        if no_prompt {
            let config = stored_config.unwrap_or_else(|| StoredConfig {
                server_api_base_url: default_server_api_base_url(),
                device_id: default_device_id(),
                agent_name: default_agent_name(),
                api_token: default_agent_api_token(),
            });

            save_stored_config(&config_path, &config);
            return Ok(Self {
                server_api_base_url: normalize_server_api_base_url(config.server_api_base_url)?,
                device_id: config.device_id,
                agent_name: config.agent_name,
                api_token: config.api_token,
            });
        }

        let default_server_api_base_url = stored_config
            .as_ref()
            .map(|config| config.server_api_base_url.clone())
            .unwrap_or_else(default_server_api_base_url);
        let server_api_base_url = prompt_server_api_base_url(&default_server_api_base_url)?;

        let default_device_id = stored_config
            .as_ref()
            .map(|config| config.device_id.clone())
            .unwrap_or_else(default_device_id);
        let device_id = prompt_device_id(&default_device_id)?;

        let default_agent_name = stored_config
            .as_ref()
            .map(|config| config.agent_name.clone())
            .unwrap_or_else(default_agent_name);
        let agent_name = prompt_agent_name(&default_agent_name)?;

        let default_api_token = stored_config
            .as_ref()
            .map(|config| config.api_token.clone())
            .unwrap_or_else(default_agent_api_token);
        let api_token = prompt_agent_api_token(&default_api_token)?;

        save_stored_config(
            &config_path,
            &StoredConfig {
                server_api_base_url: server_api_base_url.clone(),
                device_id: device_id.clone(),
                agent_name: agent_name.clone(),
                api_token: api_token.clone(),
            },
        );

        Ok(Self {
            server_api_base_url,
            device_id,
            agent_name,
            api_token,
        })
    }
}

pub fn normalize_server_api_base_url(url: String) -> Result<String> {
    let trimmed = url.trim().trim_end_matches('/').to_string();

    if let Some(prefix) = trimmed.strip_suffix("/api/agent/activity") {
        return validate_server_api_base_url(prefix.to_string());
    }

    if let Some(prefix) = trimmed.strip_suffix("/api/agent/status") {
        return validate_server_api_base_url(prefix.to_string());
    }

    if let Some(prefix) = trimmed.strip_suffix("/ws/agent") {
        let corrected = to_http_base(prefix);
        warn!(
            original = %trimmed,
            corrected = %corrected,
            "agent backend points to websocket endpoint; auto-corrected to HTTP base URL"
        );
        return validate_server_api_base_url(corrected);
    }

    if let Some(prefix) = trimmed.strip_suffix("/ws/dashboard") {
        let corrected = to_http_base(prefix);
        warn!(
            original = %trimmed,
            corrected = %corrected,
            "agent backend points to dashboard websocket; auto-corrected to HTTP base URL"
        );
        return validate_server_api_base_url(corrected);
    }

    if trimmed.starts_with("ws://") || trimmed.starts_with("wss://") {
        return validate_server_api_base_url(to_http_base(&trimmed));
    }

    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return validate_server_api_base_url(trimmed);
    }

    validate_server_api_base_url(format!("http://{trimmed}"))
}

fn to_http_base(value: &str) -> String {
    if let Some(rest) = value.strip_prefix("ws://") {
        return format!("http://{rest}");
    }

    if let Some(rest) = value.strip_prefix("wss://") {
        return format!("https://{rest}");
    }

    value.to_string()
}

fn validate_server_api_base_url(url: String) -> Result<String> {
    let mut parsed = Url::parse(&url).map_err(|err| anyhow!("invalid backend url: {err}"))?;

    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(anyhow!("agent backend must use http or https"));
    }

    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(anyhow!(
            "agent backend base url must not include query or fragment"
        ));
    }

    let normalized_path = parsed.path().trim_end_matches('/');
    if normalized_path == "/api" {
        return Err(anyhow!(
            "agent backend must point to service root, not /api"
        ));
    }

    if normalized_path.is_empty() || normalized_path == "/" {
        parsed.set_path("");
        return Ok(parsed.to_string().trim_end_matches('/').to_string());
    }

    Ok(parsed.to_string().trim_end_matches('/').to_string())
}

fn resolve_config_path() -> PathBuf {
    if let Ok(path) = env::var("AGENT_CONFIG_PATH") {
        let candidate = PathBuf::from(path);
        if candidate.is_file() {
            return candidate;
        }
    }

    let executable_dir_path = executable_dir_config_path();
    if executable_dir_path.is_file() {
        return executable_dir_path;
    }

    let current_dir_path = current_dir_config_path();
    if current_dir_path.is_file() {
        return current_dir_path;
    }

    executable_dir_path
}

fn current_dir_config_path() -> PathBuf {
    env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(CONFIG_FILE_NAME)
}

fn executable_dir_config_path() -> PathBuf {
    env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join(CONFIG_FILE_NAME)
}

fn load_stored_config(path: &Path) -> Option<StoredConfig> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return None,
        Err(err) => {
            warn!(path = %path.display(), %err, "failed to read config file");
            return None;
        }
    };

    match serde_json::from_str::<StoredConfig>(&raw) {
        Ok(config) => Some(config),
        Err(err) => {
            warn!(path = %path.display(), %err, "failed to parse config file");
            None
        }
    }
}

fn save_stored_config(path: &Path, config: &StoredConfig) {
    let raw = match serde_json::to_string_pretty(config) {
        Ok(raw) => raw,
        Err(err) => {
            error!(path = %path.display(), %err, "failed to serialize config file");
            return;
        }
    };

    if let Err(err) = fs::write(path, format!("{raw}\n")) {
        error!(path = %path.display(), %err, "failed to write config file");
    }
}

fn default_server_api_base_url() -> String {
    env::var("AGENT_SERVER_API_BASE_URL")
        .or_else(|_| env::var("AGENT_SERVER_WS_URL"))
        .unwrap_or_else(|_| "http://127.0.0.1:8787".to_string())
}

fn default_device_id() -> String {
    env::var("AGENT_DEVICE_ID").unwrap_or_else(|_| {
        hostname::get()
            .ok()
            .and_then(|host| host.into_string().ok())
            .filter(|host| !host.is_empty())
            .unwrap_or_else(|| DEFAULT_DEVICE_ID.to_string())
    })
}

fn prompt_server_api_base_url(default_value: &str) -> io::Result<String> {
    let mut stdout = io::stdout();
    writeln!(
        stdout,
        "Please enter backend address (example: http://127.0.0.1:8787)"
    )?;
    write!(stdout, "Backend address [{default_value}]: ")?;
    stdout.flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    let raw = if input.trim().is_empty() {
        default_value.to_string()
    } else {
        input
    };

    normalize_server_api_base_url(raw).map_err(io::Error::other)
}

fn prompt_device_id(default_value: &str) -> io::Result<String> {
    let mut stdout = io::stdout();
    writeln!(stdout, "Please enter current device ID")?;
    write!(stdout, "Device ID [{default_value}]: ")?;
    stdout.flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(default_value.to_string());
    }

    Ok(trimmed.to_string())
}

fn default_agent_api_token() -> String {
    env::var("AGENT_API_TOKEN").unwrap_or_else(|_| "dev-agent-token".to_string())
}

fn default_agent_name() -> String {
    env::var("AGENT_NAME").unwrap_or_else(|_| DESKTOP_AGENT_NAME.to_string())
}

fn prompt_agent_name(default_value: &str) -> io::Result<String> {
    let mut stdout = io::stdout();
    writeln!(stdout, "Please enter current agent name")?;
    write!(stdout, "Agent name [{default_value}]: ")?;
    stdout.flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    let trimmed = input.trim();
    if trimmed.is_empty() {
        if default_value.trim().is_empty() {
            return Err(io::Error::other("agent name is required"));
        }

        return Ok(default_value.to_string());
    }

    Ok(trimmed.to_string())
}

fn prompt_agent_api_token(default_value: &str) -> io::Result<String> {
    let mut stdout = io::stdout();
    writeln!(stdout, "Please enter agent API token")?;
    write!(stdout, "Agent API token [{default_value}]: ")?;
    stdout.flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    let trimmed = input.trim();
    if trimmed.is_empty() {
        if default_value.trim().is_empty() {
            return Err(io::Error::other("agent API token is required"));
        }

        return Ok(default_value.to_string());
    }

    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::{normalize_server_api_base_url, validate_server_api_base_url};

    #[test]
    fn allows_http_base_urls() {
        let result = validate_server_api_base_url("http://example.com:8787".to_string());
        assert!(result.is_ok());
    }

    #[test]
    fn converts_websocket_urls() {
        let result = normalize_server_api_base_url("ws://127.0.0.1:8787/ws/agent".to_string());
        assert_eq!(result.unwrap(), "http://127.0.0.1:8787");
    }

    #[test]
    fn rejects_api_paths() {
        let result = validate_server_api_base_url("http://127.0.0.1:8787/api".to_string());
        assert!(result.is_err());
    }
}
