use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::browser::BrowserContext;
use eyes_on_me_shared::PresenceState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityEnvelope {
    #[serde(rename = "type")]
    pub message_type: String,
    pub payload: ActivityPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityPayload {
    pub event_id: String,
    pub ts: String,
    pub device_id: String,
    pub agent_name: String,
    pub platform: String,
    pub kind: String,
    pub app: AppInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser: Option<BrowserContext>,
    pub presence: PresenceState,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInfo {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
}

impl ActivityEnvelope {
    #[allow(clippy::too_many_arguments)]
    pub fn activity(
        device_id: &str,
        agent_name: &str,
        platform: &'static str,
        source: &'static str,
        kind: &'static str,
        mut app: AppInfo,
        window_title: Option<String>,
        browser: Option<BrowserContext>,
        presence: PresenceState,
    ) -> Self {
        if app.title.is_none() {
            app.title = window_title.clone();
        }

        Self {
            message_type: "activity".to_string(),
            payload: ActivityPayload {
                event_id: Uuid::new_v4().to_string(),
                ts: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
                device_id: device_id.to_string(),
                agent_name: agent_name.to_string(),
                platform: platform.to_string(),
                kind: kind.to_string(),
                app,
                window_title,
                browser,
                presence,
                source: source.to_string(),
            },
        }
    }
}
