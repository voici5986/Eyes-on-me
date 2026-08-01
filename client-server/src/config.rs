use std::{env, net::Ipv4Addr, path::PathBuf};

#[derive(Debug, Clone)]
pub struct AiConfig {
    pub base_url: String,
    pub api_key: String,
    pub chat_model: Option<String>,
    pub embedding_model: Option<String>,
}

#[derive(Debug, Clone)]
pub enum RemoteStorageConfig {
    WebDav {
        url: String,
        username: String,
        password: String,
        prefix: String,
    },
    S3 {
        endpoint: String,
        bucket: String,
        region: String,
        access_key: String,
        secret_key: String,
        prefix: String,
    },
}

#[derive(Debug, Clone)]
pub struct Config {
    pub host: Ipv4Addr,
    pub port: u16,
    pub web_dist_dir: Option<PathBuf>,
    pub database_url: String,
    pub agent_api_token: String,
    pub dashboard_token: Option<String>,
    pub integration_token: Option<String>,
    pub secure_cookie: bool,
    pub media_dir: PathBuf,
    pub media_max_bytes: usize,
    pub media_total_max_bytes: u64,
    pub media_retention_days: u32,
    pub ocr_command: Option<String>,
    pub ocr_language: String,
    pub ocr_concurrency: usize,
    pub ocr_redact_terms: Vec<String>,
    pub ai: Option<AiConfig>,
    pub remote_storage: Option<RemoteStorageConfig>,
}

impl Config {
    pub fn from_env() -> Self {
        let host = env::var("EYES_ON_ME_HOST")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(Ipv4Addr::LOCALHOST);

        let port = env::var("EYES_ON_ME_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(8787);

        let web_dist_dir = env::var("EYES_ON_ME_WEB_DIST")
            .ok()
            .map(PathBuf::from)
            .or_else(|| Some(PathBuf::from("../web/dist")));
        let database_url = env::var("EYES_ON_ME_DATABASE_URL")
            .unwrap_or_else(|_| "sqlite://DB/eyes-on-me.db".to_string());
        let agent_api_token = env::var("EYES_ON_ME_AGENT_API_TOKEN")
            .or_else(|_| env::var("AGENT_API_TOKEN"))
            .unwrap_or_else(|_| "dev-agent-token".to_string());
        let dashboard_token = non_empty_env("EYES_ON_ME_DASHBOARD_TOKEN")
            .or_else(|| non_empty_env("DASHBOARD_TOKEN"));
        let integration_token = non_empty_env("EYES_ON_ME_INTEGRATION_TOKEN");
        let secure_cookie = env_flag("EYES_ON_ME_SECURE_COOKIE");
        let media_dir = env::var("EYES_ON_ME_MEDIA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("DB/media"));
        let media_max_bytes = env::var("EYES_ON_ME_MEDIA_MAX_BYTES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(8 * 1024 * 1024)
            .clamp(256 * 1024, 25 * 1024 * 1024);
        let media_total_max_bytes = env::var("EYES_ON_ME_MEDIA_TOTAL_MAX_BYTES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(2 * 1024 * 1024 * 1024u64)
            .clamp(64 * 1024 * 1024, 1024 * 1024 * 1024 * 1024u64);
        let media_retention_days = env::var("EYES_ON_ME_MEDIA_RETENTION_DAYS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(7)
            .clamp(1, 3650);
        let ocr_command = match env::var("EYES_ON_ME_OCR_COMMAND") {
            Ok(value) => {
                let value = value.trim().to_string();
                if value.is_empty() || matches!(value.as_str(), "off" | "none" | "disabled") {
                    None
                } else {
                    Some(value)
                }
            }
            Err(_) => Some("tesseract".to_string()),
        };
        let ocr_language =
            non_empty_env("EYES_ON_ME_OCR_LANGUAGE").unwrap_or_else(|| "eng".to_string());
        let ocr_concurrency = env::var("EYES_ON_ME_OCR_CONCURRENCY")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1)
            .clamp(1, 4);
        let ocr_redact_terms = env::var("EYES_ON_ME_OCR_REDACT_TERMS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
            .collect();
        let ai = load_ai_config();
        let remote_storage = load_remote_storage_config();

        Self {
            host,
            port,
            web_dist_dir,
            database_url,
            agent_api_token,
            dashboard_token,
            integration_token,
            secure_cookie,
            media_dir,
            media_max_bytes,
            media_total_max_bytes,
            media_retention_days,
            ocr_command,
            ocr_language,
            ocr_concurrency,
            ocr_redact_terms,
            ai,
            remote_storage,
        }
    }

    pub fn web_assets_mode(&self) -> &'static str {
        match &self.web_dist_dir {
            Some(path) if path.join("index.html").is_file() => "filesystem",
            _ => "embedded",
        }
    }
}

fn load_ai_config() -> Option<AiConfig> {
    let base_url = non_empty_env("EYES_ON_ME_AI_BASE_URL")?;
    let api_key = non_empty_env("EYES_ON_ME_AI_API_KEY")?;
    let chat_model = non_empty_env("EYES_ON_ME_AI_MODEL");
    let embedding_model = non_empty_env("EYES_ON_ME_EMBEDDING_MODEL");
    if chat_model.is_none() && embedding_model.is_none() {
        return None;
    }

    Some(AiConfig {
        base_url: base_url.trim_end_matches('/').to_string(),
        api_key,
        chat_model,
        embedding_model,
    })
}

fn load_remote_storage_config() -> Option<RemoteStorageConfig> {
    match non_empty_env("EYES_ON_ME_REMOTE_PROVIDER")?
        .to_ascii_lowercase()
        .as_str()
    {
        "webdav" => Some(RemoteStorageConfig::WebDav {
            url: non_empty_env("EYES_ON_ME_WEBDAV_URL")?
                .trim_end_matches('/')
                .to_string(),
            username: non_empty_env("EYES_ON_ME_WEBDAV_USERNAME").unwrap_or_default(),
            password: non_empty_env("EYES_ON_ME_WEBDAV_PASSWORD").unwrap_or_default(),
            prefix: non_empty_env("EYES_ON_ME_REMOTE_PREFIX")
                .unwrap_or_else(|| "eyes-on-me".to_string()),
        }),
        "s3" => Some(RemoteStorageConfig::S3 {
            endpoint: non_empty_env("EYES_ON_ME_S3_ENDPOINT")?
                .trim_end_matches('/')
                .to_string(),
            bucket: non_empty_env("EYES_ON_ME_S3_BUCKET")?,
            region: non_empty_env("EYES_ON_ME_S3_REGION")
                .unwrap_or_else(|| "us-east-1".to_string()),
            access_key: non_empty_env("EYES_ON_ME_S3_ACCESS_KEY")?,
            secret_key: non_empty_env("EYES_ON_ME_S3_SECRET_KEY")?,
            prefix: non_empty_env("EYES_ON_ME_REMOTE_PREFIX")
                .unwrap_or_else(|| "eyes-on-me".to_string()),
        }),
        _ => None,
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_flag(key: &str) -> bool {
    env::var(key)
        .ok()
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}
