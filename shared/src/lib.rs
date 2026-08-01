use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Macos,
    Windows,
    Linux,
    Android,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PresenceState {
    Active,
    Idle,
    Locked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityApp {
    pub id: String,
    pub name: String,
    pub title: Option<String>,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserContext {
    pub family: String,
    pub name: String,
    pub page_title: Option<String>,
    pub url: Option<String>,
    pub domain: Option<String>,
    pub source: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    ForegroundChanged,
    ActivitySample,
    PresenceChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEvent {
    pub event_id: String,
    #[serde(with = "time::serde::rfc3339")]
    pub ts: OffsetDateTime,
    pub device_id: String,
    pub agent_name: String,
    pub platform: Platform,
    pub kind: ActivityKind,
    pub app: ActivityApp,
    pub window_title: Option<String>,
    pub browser: Option<BrowserContext>,
    pub presence: PresenceState,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStatus {
    #[serde(with = "time::serde::rfc3339")]
    pub ts: OffsetDateTime,
    pub device_id: String,
    pub agent_name: String,
    pub platform: Platform,
    pub status_text: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSnapshot {
    pub devices: Vec<ActivityEvent>,
    pub latest_status: Option<DeviceStatus>,
    pub recent_activities: Vec<ActivityEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceOverview {
    pub device: ActivityEvent,
    pub latest_status: Option<DeviceStatus>,
    pub recording: DeviceRecordingState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevicesResponse {
    pub devices: Vec<DeviceOverview>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceDetailResponse {
    pub device: ActivityEvent,
    pub latest_status: Option<DeviceStatus>,
    pub recent_activities: Vec<ActivityEvent>,
    pub recording: DeviceRecordingState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRecordingState {
    pub device_id: String,
    pub desired_enabled: bool,
    pub applied_enabled: Option<bool>,
    pub revision: u64,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    pub acknowledged_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentControlResponse {
    pub recording: DeviceRecordingState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageBucket {
    pub key: String,
    pub label: String,
    pub sublabel: Option<String>,
    pub total_tracked_ms: u64,
    pub sessions: u32,
    #[serde(with = "time::serde::rfc3339")]
    pub last_seen: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUsageBucket {
    pub key: String,
    pub label: String,
    pub sublabel: Option<String>,
    pub total_tracked_ms: u64,
    pub sessions: u32,
    #[serde(with = "time::serde::rfc3339")]
    pub last_seen: OffsetDateTime,
    pub windows: Vec<UsageBucket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryUsageBucket {
    pub key: String,
    pub label: String,
    pub total_tracked_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HourlyUsageBucket {
    pub hour: u8,
    pub total_tracked_ms: u64,
    pub apps: Vec<UsageBucket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyUsageBucket {
    pub date: String,
    pub total_tracked_ms: u64,
    pub work_tracked_ms: u64,
    pub browser_tracked_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageUsageBucket {
    pub key: String,
    pub label: String,
    pub url: Option<String>,
    pub total_tracked_ms: u64,
    pub sessions: u32,
    #[serde(with = "time::serde::rfc3339")]
    pub last_seen: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainUsageBucket {
    pub key: String,
    pub label: String,
    pub total_tracked_ms: u64,
    pub sessions: u32,
    #[serde(with = "time::serde::rfc3339")]
    pub last_seen: OffsetDateTime,
    pub pages: Vec<PageUsageBucket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserUsageBucket {
    pub key: String,
    pub label: String,
    pub family: String,
    pub total_tracked_ms: u64,
    pub sessions: u32,
    #[serde(with = "time::serde::rfc3339")]
    pub last_seen: OffsetDateTime,
    pub domains: Vec<DomainUsageBucket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAnalysisSummary {
    pub device_id: String,
    pub platform: Platform,
    pub current_label: String,
    pub latest_status_text: Option<String>,
    pub total_tracked_ms: u64,
    pub event_count: usize,
    #[serde(with = "time::serde::rfc3339")]
    pub last_seen: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisOverviewResponse {
    #[serde(with = "time::serde::rfc3339")]
    pub generated_at: OffsetDateTime,
    pub device_count: usize,
    pub total_tracked_ms: u64,
    pub work_tracked_ms: u64,
    pub after_hours_tracked_ms: u64,
    pub browser_tracked_ms: u64,
    pub idle_tracked_ms: u64,
    pub locked_tracked_ms: u64,
    pub app_count: usize,
    pub devices: Vec<DeviceAnalysisSummary>,
    pub top_app_usage: Vec<AppUsageBucket>,
    pub top_domain_usage: Vec<UsageBucket>,
    pub top_browser_usage: Vec<BrowserUsageBucket>,
    pub category_usage: Vec<CategoryUsageBucket>,
    pub hourly_usage: Vec<HourlyUsageBucket>,
    pub daily_usage: Vec<DailyUsageBucket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAnalysisResponse {
    pub device_id: String,
    #[serde(with = "time::serde::rfc3339")]
    pub generated_at: OffsetDateTime,
    pub total_tracked_ms: u64,
    pub work_tracked_ms: u64,
    pub after_hours_tracked_ms: u64,
    pub browser_tracked_ms: u64,
    pub idle_tracked_ms: u64,
    pub locked_tracked_ms: u64,
    pub app_count: usize,
    pub event_count: usize,
    pub current_label: Option<String>,
    pub latest_status: Option<DeviceStatus>,
    pub app_usage: Vec<AppUsageBucket>,
    pub domain_usage: Vec<UsageBucket>,
    pub browser_usage: Vec<BrowserUsageBucket>,
    pub category_usage: Vec<CategoryUsageBucket>,
    pub hourly_usage: Vec<HourlyUsageBucket>,
    pub daily_usage: Vec<DailyUsageBucket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivitySearchHit {
    pub activity: ActivityEvent,
    pub snippet: Option<String>,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivitySearchResponse {
    pub query: String,
    pub device_id: Option<String>,
    pub total: usize,
    pub results: Vec<ActivitySearchHit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthSession {
    pub authenticated: bool,
    pub auth_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotRecord {
    pub id: String,
    pub event_id: String,
    pub device_id: String,
    #[serde(with = "time::serde::rfc3339")]
    pub captured_at: OffsetDateTime,
    pub mime_type: String,
    pub byte_size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub sha256: String,
    pub ocr_status: String,
    pub ocr_text: Option<String>,
    pub ocr_error: Option<String>,
    pub ocr_attempts: u32,
    pub duplicate_of: Option<String>,
    pub content_url: String,
    pub thumbnail_url: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaStatus {
    pub screenshot_count: u64,
    pub total_bytes: u64,
    pub thumbnail_bytes: u64,
    pub total_limit_bytes: u64,
    pub retention_days: u32,
    pub pending_ocr: u64,
    pub failed_ocr: u64,
    pub oldest_capture: Option<String>,
    pub newest_capture: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaCleanupResponse {
    pub deleted_screenshots: u64,
    pub freed_bytes: u64,
    pub status: MediaStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteMirrorStatus {
    pub configured: bool,
    pub provider: Option<String>,
    pub pending: u64,
    pub complete: u64,
    pub failed: u64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDiagnostics {
    pub device_id: String,
    pub agent_name: String,
    pub platform: String,
    pub updated_at: String,
    pub accessibility_permission: String,
    pub screen_capture_permission: String,
    pub screenshot_enabled: bool,
    pub screenshot_format: String,
    pub screenshot_display: String,
    pub privacy_rule_count: usize,
    pub spool_pending: usize,
    #[serde(default = "default_true")]
    pub recording_enabled: bool,
    #[serde(default)]
    pub control_revision: u64,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDiagnosticsResponse {
    pub agents: Vec<AgentDiagnostics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotListResponse {
    pub screenshots: Vec<ScreenshotRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CategoryTarget {
    App,
    Domain,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryRule {
    pub id: String,
    pub name: String,
    pub color: String,
    pub target: CategoryTarget,
    pub pattern: String,
    pub priority: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkScheduleSegment {
    pub id: String,
    /// ISO weekday, Monday = 1 and Sunday = 7.
    pub weekday: u8,
    pub start_minute: u16,
    pub end_minute: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportPreferences {
    pub pinned_blocks: Vec<String>,
    pub hidden_blocks: Vec<String>,
    pub block_order: Vec<String>,
    pub auto_export_enabled: bool,
    pub auto_export_directory: Option<String>,
}

impl Default for ReportPreferences {
    fn default() -> Self {
        Self {
            pinned_blocks: Vec::new(),
            hidden_blocks: Vec::new(),
            block_order: vec![
                "overview".to_string(),
                "applications".to_string(),
                "websites".to_string(),
                "screenshots".to_string(),
            ],
            auto_export_enabled: false,
            auto_export_directory: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSettings {
    pub category_rules: Vec<CategoryRule>,
    pub work_schedule: Vec<WorkScheduleSegment>,
    pub report_preferences: ReportPreferences,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineItem {
    pub activity: ActivityEvent,
    pub screenshot: Option<ScreenshotRecord>,
    pub duration_ms: u64,
    pub category_key: String,
    pub category_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineResponse {
    pub items: Vec<TimelineItem>,
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletionSummary {
    pub deleted_activities: u64,
    pub deleted_screenshots: u64,
    pub affected_dates: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PotentialTodo {
    pub text: String,
    pub source_event_id: String,
    #[serde(with = "time::serde::rfc3339")]
    pub observed_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkSession {
    pub id: String,
    pub device_id: String,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub ended_at: OffsetDateTime,
    pub total_tracked_ms: u64,
    pub app_names: Vec<String>,
    pub summary: String,
    pub potential_todos: Vec<PotentialTodo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkSessionResponse {
    pub sessions: Vec<WorkSession>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantConversation {
    pub id: String,
    pub title: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessage {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub mode: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantConversationDetail {
    pub conversation: AssistantConversation,
    pub messages: Vec<AssistantMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantReply {
    pub conversation: AssistantConversation,
    pub message: AssistantMessage,
    pub starter_prompts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyReport {
    pub date: String,
    pub content: String,
    pub generation_mode: String,
    pub model_name: Option<String>,
    pub fallback_reason: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEntry {
    pub id: String,
    pub date: String,
    pub source_type: String,
    pub title: String,
    pub content: String,
    pub tags: Vec<String>,
    pub score: Option<f32>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySearchResponse {
    pub query: String,
    pub mode: String,
    pub entries: Vec<MemoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryReindexResponse {
    pub indexed_entries: usize,
    pub embedded_entries: usize,
    pub embedding_failures: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum StreamMessage {
    Snapshot(DashboardSnapshot),
    Ping {
        #[serde(with = "time::serde::rfc3339")]
        ts: OffsetDateTime,
    },
}

impl DashboardSnapshot {
    pub fn demo() -> Self {
        let now = OffsetDateTime::now_utc();
        let activity = ActivityEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            ts: now,
            device_id: "my-mac".to_string(),
            agent_name: "client-desktop".to_string(),
            platform: Platform::Macos,
            kind: ActivityKind::ForegroundChanged,
            app: ActivityApp {
                id: "com.apple.Safari".to_string(),
                name: "Safari".to_string(),
                title: Some("eyes-on-me".to_string()),
                pid: Some(4242),
            },
            window_title: Some("am-i-okay dashboard".to_string()),
            browser: Some(BrowserContext {
                family: "webkit".to_string(),
                name: "Safari".to_string(),
                page_title: Some("eyes-on-me".to_string()),
                url: Some("https://example.com/eyes-on-me".to_string()),
                domain: Some("example.com".to_string()),
                source: "demo".to_string(),
                confidence: 0.9,
            }),
            presence: PresenceState::Active,
            source: "demo".to_string(),
        };

        let status = DeviceStatus {
            ts: now,
            device_id: "my-mac".to_string(),
            agent_name: "client-desktop".to_string(),
            platform: Platform::Macos,
            status_text: "building Eyes on Me".to_string(),
            source: "demo".to_string(),
        };

        Self {
            devices: vec![activity.clone()],
            latest_status: Some(status),
            recent_activities: vec![activity],
        }
    }
}
