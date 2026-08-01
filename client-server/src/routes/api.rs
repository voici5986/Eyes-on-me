use std::{convert::Infallible, sync::Arc, time::Duration};

use anyhow::Context;
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Path, Query, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE, SET_COOKIE},
    },
    middleware,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{delete, get, post},
};
use eyes_on_me_shared::{
    ActivityEvent, AgentControlResponse, AgentDiagnostics, AgentDiagnosticsResponse, AuthSession,
    DeviceStatus, ReviewSettings, ScreenshotListResponse, StreamMessage,
};
use serde::Deserialize;
use tokio_stream::{StreamExt, wrappers::BroadcastStream};
use tracing::error;

use crate::app_state::{AnalysisRange, AppState};

pub fn router(state: Arc<AppState>) -> Router {
    let public = Router::new()
        .route("/health", get(health))
        .route("/api/auth/session", get(auth_session))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/agent/activity", post(post_activity))
        .route("/api/agent/status", post(post_status))
        .route(
            "/api/agent/control/{device_id}",
            get(get_agent_control).post(ack_agent_control),
        )
        .route("/api/agent/diagnostics", post(post_agent_diagnostics))
        .route("/api/agent/screenshots/{event_id}", post(post_screenshot));
    let public = public.route("/mcp", post(mcp_handler));

    let protected = Router::new()
        .route("/api/current", get(get_current))
        .route("/api/devices", get(get_devices))
        .route("/api/search/activities", get(search_activities))
        .route("/api/timeline", get(get_timeline))
        .route("/api/sessions", get(get_work_sessions))
        .route("/api/activities/bulk-delete", post(bulk_delete_activities))
        .route("/api/devices/{device_id}", get(get_device_detail))
        .route(
            "/api/devices/{device_id}/screenshots",
            get(get_device_screenshots),
        )
        .route(
            "/api/screenshots/{screenshot_id}/content",
            get(get_screenshot_content),
        )
        .route("/api/screenshots", get(get_screenshots))
        .route(
            "/api/screenshots/{screenshot_id}",
            delete(delete_screenshot_handler),
        )
        .route(
            "/api/screenshots/{screenshot_id}/thumbnail",
            get(get_screenshot_thumbnail),
        )
        .route(
            "/api/screenshots/{screenshot_id}/ocr",
            post(retry_screenshot_ocr),
        )
        .route(
            "/api/activities/{event_id}",
            delete(delete_activity_handler),
        )
        .route("/api/media/status", get(get_media_status))
        .route("/api/media/remote", get(get_remote_media_status))
        .route("/api/media/remote/retry", post(retry_remote_media))
        .route("/api/media/cleanup", post(cleanup_media))
        .route("/api/agent/diagnostics", get(get_agent_diagnostics))
        .route("/api/export/activities", get(export_activities))
        .route("/api/analysis", get(get_analysis_overview))
        .route(
            "/api/devices/{device_id}/analysis",
            get(get_device_analysis),
        )
        .route(
            "/api/devices/{device_id}/recording",
            axum::routing::put(set_device_recording),
        )
        .route(
            "/api/settings/review",
            get(get_review_settings).put(put_review_settings),
        )
        .route("/api/settings/review/export", get(export_review_settings))
        .route("/api/settings/review/import", post(import_review_settings))
        .route("/api/stream", get(stream))
        .route("/api/reports", get(list_reports))
        .route("/api/reports/export", get(export_report_range))
        .route("/api/reports/{date}", get(get_report).put(put_report))
        .route("/api/reports/{date}/generate", post(generate_report))
        .route("/api/reports/{date}/export", get(export_report))
        .route("/api/memory", get(search_memory))
        .route("/api/memory/reindex", post(reindex_memory))
        .route(
            "/api/assistant/conversations",
            get(list_assistant_conversations),
        )
        .route(
            "/api/assistant/conversations/{conversation_id}",
            get(get_assistant_conversation).delete(delete_assistant_conversation),
        )
        .route("/api/assistant/prompts", get(get_assistant_prompts))
        .route("/api/assistant/stream", post(stream_assistant_reply))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            crate::auth::require_dashboard_auth,
        ));

    public.merge(protected).with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ActivityInput {
    Raw(ActivityEvent),
    Envelope(Envelope<ActivityEvent>),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StatusInput {
    Raw(DeviceStatus),
    Envelope(Envelope<DeviceStatus>),
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    #[serde(rename = "type")]
    _message_type: String,
    payload: T,
}

#[derive(Debug, Deserialize, Default)]
struct AnalysisQuery {
    range: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ActivitySearchQuery {
    q: Option<String>,
    device_id: Option<String>,
    limit: Option<u16>,
}

#[derive(Debug, Deserialize)]
struct LoginInput {
    token: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ScreenshotQuery {
    limit: Option<u16>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GenerateReportQuery {
    use_ai: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ReportInput {
    content: String,
}

#[derive(Debug, Deserialize)]
struct ReportRangeQuery {
    start: String,
    end: String,
}

#[derive(Debug, Deserialize, Default)]
struct MemoryQuery {
    q: Option<String>,
    limit: Option<u16>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ActivityExportQuery {
    format: Option<String>,
    device_id: Option<String>,
    date: Option<String>,
    start: Option<String>,
    end: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct TimelineQuery {
    date: Option<String>,
    start: Option<String>,
    end: Option<String>,
    device_id: Option<String>,
    app: Option<String>,
    domain: Option<String>,
    category: Option<String>,
    q: Option<String>,
    limit: Option<u16>,
    offset: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BulkDeleteInput {
    event_id: Option<String>,
    date: Option<String>,
    start: Option<String>,
    end: Option<String>,
    device_id: Option<String>,
    app: Option<String>,
    domain: Option<String>,
    category: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RecordingInput {
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct RecordingAckInput {
    enabled: bool,
    revision: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssistantInput {
    conversation_id: Option<String>,
    prompt: String,
    use_ai: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct McpRequest {
    #[serde(default)]
    jsonrpc: String,
    id: Option<serde_json::Value>,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true }))
}

async fn mcp_handler(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(request): Json<McpRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !is_integration_authorized(&headers, &state) {
        return Err(if state.integrations_enabled() {
            StatusCode::UNAUTHORIZED
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        });
    }
    let id = request.id.unwrap_or(serde_json::Value::Null);
    if request.jsonrpc != "2.0" {
        return Ok(Json(mcp_error(id, -32600, "invalid JSON-RPC version")));
    }
    let result = match request.method.as_str() {
        "initialize" => Ok(serde_json::json!({
            "protocolVersion": "2025-06-18",
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "eyes-on-me", "version": env!("CARGO_PKG_VERSION") }
        })),
        "notifications/initialized" => Ok(serde_json::Value::Null),
        "ping" => Ok(serde_json::json!({})),
        "tools/list" => Ok(serde_json::json!({ "tools": mcp_tools() })),
        "tools/call" => mcp_call_tool(&state, request.params).await,
        _ => Err(anyhow::anyhow!("method not found")),
    };
    Ok(Json(match result {
        Ok(result) => serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => mcp_error(id, -32602, &error.to_string()),
    }))
}

fn mcp_tools() -> serde_json::Value {
    serde_json::json!([
        {"name":"current_context","description":"Current foreground context for all devices","inputSchema":{"type":"object","properties":{}}},
        {"name":"timeline","description":"Activity timeline for a local date","inputSchema":{"type":"object","properties":{"date":{"type":"string"},"deviceId":{"type":"string"},"limit":{"type":"integer"}},"required":["date"]}},
        {"name":"work_sessions","description":"Continuous work sessions and potential follow-up items","inputSchema":{"type":"object","properties":{"date":{"type":"string"},"deviceId":{"type":"string"}},"required":["date"]}},
        {"name":"search_memory","description":"Search local semantic memory","inputSchema":{"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer"}},"required":["query"]}},
        {"name":"get_report","description":"Read a saved daily report","inputSchema":{"type":"object","properties":{"date":{"type":"string"}},"required":["date"]}},
        {"name":"generate_report","description":"Generate a daily report from local records","inputSchema":{"type":"object","properties":{"date":{"type":"string"},"useAi":{"type":"boolean"}},"required":["date"]}},
        {"name":"media_status","description":"Screenshot storage and OCR queue status","inputSchema":{"type":"object","properties":{}}}
    ])
}

async fn mcp_call_tool(
    state: &AppState,
    params: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let name = params
        .get("name")
        .and_then(serde_json::Value::as_str)
        .context("tool name is required")?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let value = match name {
        "current_context" => serde_json::to_value(state.snapshot())?,
        "timeline" => {
            let date = arguments.get("date").and_then(serde_json::Value::as_str);
            let (start, end) = resolve_time_range(date, None, None)?;
            serde_json::to_value(
                crate::governance::load_timeline(
                    state,
                    crate::governance::TimelineFilter {
                        start,
                        end,
                        device_id: arguments
                            .get("deviceId")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string),
                        event_id: None,
                        app: None,
                        domain: None,
                        category: None,
                        query: None,
                        limit: arguments
                            .get("limit")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(100)
                            .clamp(1, 500) as usize,
                        offset: 0,
                    },
                )
                .await?,
            )?
        }
        "work_sessions" => {
            let date = arguments.get("date").and_then(serde_json::Value::as_str);
            let (start, end) = resolve_time_range(date, None, None)?;
            serde_json::to_value(
                crate::governance::load_work_sessions(
                    state,
                    start,
                    end,
                    arguments
                        .get("deviceId")
                        .and_then(serde_json::Value::as_str),
                )
                .await?,
            )?
        }
        "search_memory" => serde_json::to_value(
            crate::memory::search(
                state,
                arguments
                    .get("query")
                    .and_then(serde_json::Value::as_str)
                    .context("query is required")?,
                arguments
                    .get("limit")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(20)
                    .clamp(1, 100),
            )
            .await?,
        )?,
        "get_report" => {
            let date = arguments
                .get("date")
                .and_then(serde_json::Value::as_str)
                .context("date is required")?;
            serde_json::to_value(crate::db::load_daily_report(&state.pool(), date).await?)?
        }
        "generate_report" => {
            let date = arguments
                .get("date")
                .and_then(serde_json::Value::as_str)
                .context("date is required")?;
            serde_json::to_value(
                crate::reports::generate(
                    state,
                    date,
                    arguments
                        .get("useAi")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                )
                .await?,
            )?
        }
        "media_status" => serde_json::to_value(crate::media::status(state).await?)?,
        _ => anyhow::bail!("unknown tool: {name}"),
    };
    Ok(serde_json::json!({
        "content": [{"type":"text","text":serde_json::to_string_pretty(&value)?}],
        "structuredContent": value,
        "isError": false
    }))
}

fn mcp_error(id: serde_json::Value, code: i64, message: &str) -> serde_json::Value {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

async fn auth_session(headers: HeaderMap, State(state): State<Arc<AppState>>) -> Json<AuthSession> {
    let auth_required = state.dashboard_auth_required();
    let authenticated = !auth_required
        || crate::auth::session_cookie_value(&headers)
            .map(|value| state.is_dashboard_session_valid(value))
            .unwrap_or(false);
    Json(AuthSession {
        authenticated,
        auth_required,
    })
}

async fn login(
    State(state): State<Arc<AppState>>,
    Json(input): Json<LoginInput>,
) -> Result<Response, StatusCode> {
    if !state.verify_dashboard_token(input.token.trim()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let mut response = Json(AuthSession {
        authenticated: true,
        auth_required: state.dashboard_auth_required(),
    })
    .into_response();
    if let Some(session) = state.dashboard_session_token() {
        let cookie = crate::auth::login_cookie(session, state.secure_cookie());
        response.headers_mut().insert(
            SET_COOKIE,
            HeaderValue::from_str(&cookie).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        );
    }
    Ok(response)
}

async fn logout(State(state): State<Arc<AppState>>) -> Result<Response, StatusCode> {
    let mut response = Json(AuthSession {
        authenticated: !state.dashboard_auth_required(),
        auth_required: state.dashboard_auth_required(),
    })
    .into_response();
    let cookie = crate::auth::clear_cookie(state.secure_cookie());
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    Ok(response)
}

async fn get_current(
    State(state): State<Arc<AppState>>,
) -> Json<eyes_on_me_shared::DashboardSnapshot> {
    Json(state.snapshot())
}

async fn get_devices(
    State(state): State<Arc<AppState>>,
) -> Result<Json<eyes_on_me_shared::DevicesResponse>, StatusCode> {
    state.devices_response().await.map(Json).map_err(|err| {
        error!(error = %err, "failed to load devices response");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

async fn get_device_detail(
    Path(device_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<eyes_on_me_shared::DeviceDetailResponse>, StatusCode> {
    match state.device_detail(&device_id).await {
        Ok(Some(device)) => Ok(Json(device)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(err) => {
            error!(error = %err, device_id, "failed to load device detail");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn get_analysis_overview(
    Query(query): Query<AnalysisQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<eyes_on_me_shared::AnalysisOverviewResponse>, StatusCode> {
    let range = AnalysisRange::from_query(query.range.as_deref()).ok_or(StatusCode::BAD_REQUEST)?;

    state
        .analysis_overview(range)
        .await
        .map(Json)
        .map_err(|err| {
            error!(error = %err, "failed to load analysis overview");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn get_device_analysis(
    Path(device_id): Path<String>,
    Query(query): Query<AnalysisQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<eyes_on_me_shared::DeviceAnalysisResponse>, StatusCode> {
    let range = AnalysisRange::from_query(query.range.as_deref()).ok_or(StatusCode::BAD_REQUEST)?;

    match state.device_analysis(&device_id, range).await {
        Ok(Some(analysis)) => Ok(Json(analysis)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(err) => {
            error!(error = %err, device_id, "failed to load device analysis");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn search_activities(
    Query(query): Query<ActivitySearchQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<eyes_on_me_shared::ActivitySearchResponse>, StatusCode> {
    let search_query = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(StatusCode::BAD_REQUEST)?;
    if !search_query.chars().any(char::is_alphanumeric) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let device_id = query
        .device_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let limit = query.limit.unwrap_or(25).clamp(1, 100) as i64;

    state
        .search_activities(search_query, device_id, limit)
        .await
        .map(Json)
        .map_err(|err| {
            error!(error = %err, query = search_query, device_id, "failed to search activities");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn get_timeline(
    Query(query): Query<TimelineQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<eyes_on_me_shared::TimelineResponse>, StatusCode> {
    let filter = timeline_filter(query).map_err(|error| {
        error!(error = %error, "invalid timeline query");
        StatusCode::BAD_REQUEST
    })?;
    crate::governance::load_timeline(&state, filter)
        .await
        .map(Json)
        .map_err(internal_error)
}

async fn get_work_sessions(
    Query(query): Query<TimelineQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<eyes_on_me_shared::WorkSessionResponse>, StatusCode> {
    let (start, end) = resolve_time_range(
        query.date.as_deref(),
        query.start.as_deref(),
        query.end.as_deref(),
    )
    .map_err(|_| StatusCode::BAD_REQUEST)?;
    crate::governance::load_work_sessions(&state, start, end, query.device_id.as_deref())
        .await
        .map(Json)
        .map_err(internal_error)
}

async fn bulk_delete_activities(
    State(state): State<Arc<AppState>>,
    Json(input): Json<BulkDeleteInput>,
) -> Result<Json<eyes_on_me_shared::DeletionSummary>, StatusCode> {
    let filter = bulk_delete_filter(&state, input).await.map_err(|error| {
        error!(error = %error, "invalid bulk delete request");
        StatusCode::BAD_REQUEST
    })?;
    crate::governance::delete_matching_activities(&state, filter)
        .await
        .map(Json)
        .map_err(internal_error)
}

async fn post_activity(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ActivityInput>,
) -> Result<Json<eyes_on_me_shared::DashboardSnapshot>, StatusCode> {
    if !is_agent_authorized(&headers, state.agent_api_token()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let payload = match payload {
        ActivityInput::Raw(payload) => payload,
        ActivityInput::Envelope(message) => message.payload,
    };

    if let Err(err) = state.upsert_activity(payload).await {
        error!(error = %err, "failed to persist activity");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    Ok(Json(state.snapshot()))
}

async fn post_status(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(payload): Json<StatusInput>,
) -> Result<Json<eyes_on_me_shared::DashboardSnapshot>, StatusCode> {
    if !is_agent_authorized(&headers, state.agent_api_token()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let payload = match payload {
        StatusInput::Raw(payload) => payload,
        StatusInput::Envelope(message) => message.payload,
    };

    if let Err(err) = state.update_status(payload).await {
        error!(error = %err, "failed to persist status");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    Ok(Json(state.snapshot()))
}

async fn get_agent_control(
    Path(device_id): Path<String>,
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<AgentControlResponse>, StatusCode> {
    if !is_agent_authorized(&headers, state.agent_api_token()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    crate::db::load_recording_state(&state.pool(), &device_id)
        .await
        .map(|recording| Json(AgentControlResponse { recording }))
        .map_err(internal_error)
}

async fn ack_agent_control(
    Path(device_id): Path<String>,
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(input): Json<RecordingAckInput>,
) -> Result<Json<AgentControlResponse>, StatusCode> {
    if !is_agent_authorized(&headers, state.agent_api_token()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    crate::db::acknowledge_recording_state(&state.pool(), &device_id, input.enabled, input.revision)
        .await
        .map(|recording| Json(AgentControlResponse { recording }))
        .map_err(internal_error)
}

async fn set_device_recording(
    Path(device_id): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(input): Json<RecordingInput>,
) -> Result<Json<eyes_on_me_shared::DeviceRecordingState>, StatusCode> {
    crate::db::set_recording_desired(&state.pool(), &device_id, input.enabled)
        .await
        .map(Json)
        .map_err(internal_error)
}

async fn get_review_settings(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ReviewSettings>, StatusCode> {
    crate::governance::effective_settings(&state.pool())
        .await
        .map(Json)
        .map_err(internal_error)
}

async fn put_review_settings(
    State(state): State<Arc<AppState>>,
    Json(settings): Json<ReviewSettings>,
) -> Result<Json<ReviewSettings>, StatusCode> {
    crate::governance::save_settings(&state.pool(), settings)
        .await
        .map(Json)
        .map_err(|error| {
            error!(error = %error, "failed to save review settings");
            StatusCode::BAD_REQUEST
        })
}

async fn export_review_settings(
    State(state): State<Arc<AppState>>,
) -> Result<Response, StatusCode> {
    let settings = crate::governance::effective_settings(&state.pool())
        .await
        .map_err(internal_error)?;
    let bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "format": "eyes-on-me-review-settings",
        "version": 1,
        "exportedAt": time::OffsetDateTime::now_utc().to_string(),
        "settings": settings
    }))
    .map_err(|error| internal_error(error.into()))?;
    let mut response = bytes.into_response();
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response.headers_mut().insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"eyes-on-me-settings.json\""),
    );
    Ok(response)
}

#[derive(Debug, Deserialize)]
struct ReviewSettingsBackupInput {
    format: String,
    version: u32,
    settings: ReviewSettings,
}

async fn import_review_settings(
    State(state): State<Arc<AppState>>,
    Json(input): Json<ReviewSettingsBackupInput>,
) -> Result<Json<ReviewSettings>, StatusCode> {
    if input.format != "eyes-on-me-review-settings" || input.version != 1 {
        return Err(StatusCode::BAD_REQUEST);
    }
    crate::governance::save_settings(&state.pool(), input.settings)
        .await
        .map(Json)
        .map_err(|error| {
            error!(error = %error, "failed to import review settings");
            StatusCode::BAD_REQUEST
        })
}

async fn post_agent_diagnostics(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AgentDiagnostics>,
) -> Result<StatusCode, StatusCode> {
    if !is_agent_authorized(&headers, state.agent_api_token()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    crate::db::persist_agent_diagnostics(&state.pool(), &payload)
        .await
        .map_err(internal_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn post_screenshot(
    Path(event_id): Path<String>,
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    body: Body,
) -> Result<Json<eyes_on_me_shared::ScreenshotRecord>, StatusCode> {
    if !is_agent_authorized(&headers, state.agent_api_token()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let declared_mime = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    let bytes = to_bytes(body, state.media_max_bytes() + 1)
        .await
        .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?;
    crate::media::store_screenshot(&state, &event_id, declared_mime, &bytes)
        .await
        .map(Json)
        .map_err(|err| {
            error!(error = %err, event_id, "failed to store screenshot");
            if err.to_string().contains("does not exist") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_REQUEST
            }
        })
}

async fn get_device_screenshots(
    Path(device_id): Path<String>,
    Query(query): Query<ScreenshotQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<ScreenshotListResponse>, StatusCode> {
    let limit = query.limit.unwrap_or(60).clamp(1, 200) as i64;
    crate::db::load_screenshots_for_device(&state.pool(), &device_id, limit)
        .await
        .map(|screenshots| Json(ScreenshotListResponse { screenshots }))
        .map_err(internal_error)
}

async fn get_screenshots(
    Query(query): Query<ScreenshotQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<ScreenshotListResponse>, StatusCode> {
    let limit = query.limit.unwrap_or(100).clamp(1, 200) as i64;
    crate::db::load_recent_screenshots(&state.pool(), limit)
        .await
        .map(|screenshots| Json(ScreenshotListResponse { screenshots }))
        .map_err(internal_error)
}

async fn get_screenshot_content(
    Path(screenshot_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Response, StatusCode> {
    match crate::media::read_screenshot(&state, &screenshot_id)
        .await
        .map_err(internal_error)?
    {
        Some((mime, bytes)) => {
            let mut response = bytes.into_response();
            response.headers_mut().insert(
                CONTENT_TYPE,
                HeaderValue::from_str(&mime).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
            );
            response.headers_mut().insert(
                CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=86400"),
            );
            Ok(response)
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn get_screenshot_thumbnail(
    Path(screenshot_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Response, StatusCode> {
    match crate::media::read_thumbnail(&state, &screenshot_id)
        .await
        .map_err(internal_error)?
    {
        Some(bytes) => {
            let mut response = bytes.into_response();
            response
                .headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
            response.headers_mut().insert(
                CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=86400"),
            );
            Ok(response)
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn delete_screenshot_handler(
    Path(screenshot_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<StatusCode, StatusCode> {
    match crate::media::delete_screenshot(&state, &screenshot_id)
        .await
        .map_err(internal_error)?
    {
        true => Ok(StatusCode::NO_CONTENT),
        false => Err(StatusCode::NOT_FOUND),
    }
}

async fn retry_screenshot_ocr(
    Path(screenshot_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<eyes_on_me_shared::ScreenshotRecord>, StatusCode> {
    crate::media::retry_ocr(&state, &screenshot_id)
        .await
        .map(Json)
        .map_err(|error| {
            error!(error = %error, screenshot_id, "failed to retry screenshot OCR");
            if error.to_string().contains("does not exist") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_REQUEST
            }
        })
}

async fn delete_activity_handler(
    Path(event_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<StatusCode, StatusCode> {
    match crate::media::delete_activity(&state, &event_id)
        .await
        .map_err(internal_error)?
    {
        true => Ok(StatusCode::NO_CONTENT),
        false => Err(StatusCode::NOT_FOUND),
    }
}

async fn get_media_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<eyes_on_me_shared::MediaStatus>, StatusCode> {
    crate::media::status(&state)
        .await
        .map(Json)
        .map_err(internal_error)
}

async fn get_remote_media_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<eyes_on_me_shared::RemoteMirrorStatus>, StatusCode> {
    crate::remote::status(&state)
        .await
        .map(Json)
        .map_err(internal_error)
}

async fn retry_remote_media(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    crate::remote::retry_pending(&state)
        .await
        .map(|queued| Json(serde_json::json!({ "queued": queued })))
        .map_err(internal_error)
}

async fn cleanup_media(
    State(state): State<Arc<AppState>>,
) -> Result<Json<eyes_on_me_shared::MediaCleanupResponse>, StatusCode> {
    crate::media::cleanup(&state)
        .await
        .map(Json)
        .map_err(internal_error)
}

async fn get_agent_diagnostics(
    State(state): State<Arc<AppState>>,
) -> Result<Json<AgentDiagnosticsResponse>, StatusCode> {
    crate::db::load_agent_diagnostics(&state.pool())
        .await
        .map(|agents| Json(AgentDiagnosticsResponse { agents }))
        .map_err(internal_error)
}

async fn export_activities(
    Query(query): Query<ActivityExportQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Response, StatusCode> {
    let device_id = query
        .device_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mut activities = if query.date.is_some() || query.start.is_some() || query.end.is_some() {
        let (start, end) = resolve_time_range(
            query.date.as_deref(),
            query.start.as_deref(),
            query.end.as_deref(),
        )
        .map_err(|_| StatusCode::BAD_REQUEST)?;
        crate::db::load_activities_between(&state.pool(), start, end).await
    } else {
        match device_id {
            Some(device_id) => {
                crate::db::load_all_activities_for_device(&state.pool(), device_id).await
            }
            None => crate::db::load_all_activities(&state.pool()).await,
        }
    }
    .map_err(internal_error)?;
    if let Some(device_id) = device_id {
        activities.retain(|item| item.device_id == device_id);
    }
    let format = query.format.as_deref().unwrap_or("csv");
    let (mime, extension, bytes) = match format {
        "json" => (
            "application/json",
            "json",
            serde_json::to_vec_pretty(&activities).map_err(|error| internal_error(error.into()))?,
        ),
        "csv" => {
            let mut writer = csv::Writer::from_writer(Vec::new());
            writer
                .write_record([
                    "event_id",
                    "timestamp",
                    "device_id",
                    "app",
                    "window_or_tab",
                    "browser_url",
                    "domain",
                    "presence",
                    "source",
                ])
                .map_err(|error| internal_error(error.into()))?;
            for item in &activities {
                writer
                    .write_record([
                        item.event_id.as_str(),
                        &item.ts.to_string(),
                        item.device_id.as_str(),
                        item.app.name.as_str(),
                        item.browser
                            .as_ref()
                            .and_then(|value| value.page_title.as_deref())
                            .or(item.window_title.as_deref())
                            .unwrap_or(""),
                        item.browser
                            .as_ref()
                            .and_then(|value| value.url.as_deref())
                            .unwrap_or(""),
                        item.browser
                            .as_ref()
                            .and_then(|value| value.domain.as_deref())
                            .unwrap_or(""),
                        match item.presence {
                            eyes_on_me_shared::PresenceState::Active => "active",
                            eyes_on_me_shared::PresenceState::Idle => "idle",
                            eyes_on_me_shared::PresenceState::Locked => "locked",
                        },
                        item.source.as_str(),
                    ])
                    .map_err(|error| internal_error(error.into()))?;
            }
            (
                "text/csv; charset=utf-8",
                "csv",
                writer
                    .into_inner()
                    .map_err(|error| internal_error(error.into_error().into()))?,
            )
        }
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let filename = format!("eyes-on-me-activities.{extension}");
    let mut response = bytes.into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_str(mime).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    response.headers_mut().insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    Ok(response)
}

async fn get_report(
    Path(date): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<eyes_on_me_shared::DailyReport>, StatusCode> {
    crate::reports::date_bounds(&date).map_err(|_| StatusCode::BAD_REQUEST)?;
    crate::db::load_daily_report(&state.pool(), &date)
        .await
        .map_err(internal_error)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn list_reports(
    Query(query): Query<ReportRangeQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<eyes_on_me_shared::DailyReport>>, StatusCode> {
    validate_report_range(&query.start, &query.end)?;
    crate::db::load_daily_reports_between(&state.pool(), &query.start, &query.end)
        .await
        .map(Json)
        .map_err(internal_error)
}

async fn export_report_range(
    Query(query): Query<ReportRangeQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Response, StatusCode> {
    validate_report_range(&query.start, &query.end)?;
    let reports = crate::db::load_daily_reports_between(&state.pool(), &query.start, &query.end)
        .await
        .map_err(internal_error)?;
    let body = reports
        .into_iter()
        .map(|report| report.content)
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");
    let mut response = body.into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/markdown; charset=utf-8"),
    );
    response.headers_mut().insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "attachment; filename=\"eyes-on-me-reports-{}-{}.md\"",
            query.start, query.end
        ))
        .map_err(|_| StatusCode::BAD_REQUEST)?,
    );
    Ok(response)
}

async fn generate_report(
    Path(date): Path<String>,
    Query(query): Query<GenerateReportQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<eyes_on_me_shared::DailyReport>, StatusCode> {
    crate::reports::generate(&state, &date, query.use_ai.unwrap_or(false))
        .await
        .map(Json)
        .map_err(|err| {
            error!(error = %err, date, "failed to generate report");
            StatusCode::BAD_REQUEST
        })
}

async fn put_report(
    Path(date): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(input): Json<ReportInput>,
) -> Result<Json<eyes_on_me_shared::DailyReport>, StatusCode> {
    crate::reports::save_manual(&state, &date, input.content)
        .await
        .map(Json)
        .map_err(|err| {
            error!(error = %err, date, "failed to save report");
            StatusCode::BAD_REQUEST
        })
}

async fn export_report(
    Path(date): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Response, StatusCode> {
    let report = crate::db::load_daily_report(&state.pool(), &date)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok((
        [(CONTENT_TYPE, "text/markdown; charset=utf-8")],
        report.content,
    )
        .into_response())
}

async fn search_memory(
    Query(query): Query<MemoryQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<eyes_on_me_shared::MemorySearchResponse>, StatusCode> {
    let search_query = query.q.as_deref().unwrap_or_default();
    if !search_query.trim().is_empty() && !search_query.chars().any(char::is_alphanumeric) {
        return Err(StatusCode::BAD_REQUEST);
    }
    crate::memory::search(
        &state,
        search_query,
        query.limit.unwrap_or(30).clamp(1, 100) as i64,
    )
    .await
    .map(Json)
    .map_err(internal_error)
}

async fn reindex_memory(
    State(state): State<Arc<AppState>>,
) -> Result<Json<eyes_on_me_shared::MemoryReindexResponse>, StatusCode> {
    crate::memory::reindex(&state)
        .await
        .map(Json)
        .map_err(internal_error)
}

async fn list_assistant_conversations(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<eyes_on_me_shared::AssistantConversation>>, StatusCode> {
    crate::assistant::list(&state)
        .await
        .map(Json)
        .map_err(internal_error)
}

async fn get_assistant_conversation(
    Path(conversation_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<eyes_on_me_shared::AssistantConversationDetail>, StatusCode> {
    crate::assistant::detail(&state, &conversation_id)
        .await
        .map_err(internal_error)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn delete_assistant_conversation(
    Path(conversation_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<StatusCode, StatusCode> {
    match crate::assistant::delete(&state, &conversation_id)
        .await
        .map_err(internal_error)?
    {
        true => Ok(StatusCode::NO_CONTENT),
        false => Err(StatusCode::NOT_FOUND),
    }
}

async fn get_assistant_prompts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<String>>, StatusCode> {
    crate::assistant::starter_prompts(&state)
        .await
        .map(Json)
        .map_err(internal_error)
}

async fn stream_assistant_reply(
    State(state): State<Arc<AppState>>,
    Json(input): Json<AssistantInput>,
) -> Result<Response, StatusCode> {
    if input.prompt.trim().is_empty() || input.prompt.chars().count() > 8_000 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<axum::body::Bytes, Infallible>>(16);
    tokio::spawn(async move {
        let result = crate::assistant::ask(
            &state,
            input.conversation_id.as_deref(),
            &input.prompt,
            input.use_ai.unwrap_or(false),
        )
        .await;
        match result {
            Ok(reply) => {
                for characters in reply.message.content.chars().collect::<Vec<_>>().chunks(32) {
                    let token = characters.iter().collect::<String>();
                    let line =
                        serde_json::json!({ "type": "token", "token": token }).to_string() + "\n";
                    if tx.send(Ok(axum::body::Bytes::from(line))).await.is_err() {
                        return;
                    }
                }
                let line = serde_json::json!({ "type": "done", "reply": reply }).to_string() + "\n";
                let _ = tx.send(Ok(axum::body::Bytes::from(line))).await;
            }
            Err(error) => {
                let line = serde_json::json!({ "type": "error", "error": error.to_string() })
                    .to_string()
                    + "\n";
                let _ = tx.send(Ok(axum::body::Bytes::from(line))).await;
            }
        }
    });
    let mut response =
        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)).into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/x-ndjson; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

fn timeline_filter(query: TimelineQuery) -> anyhow::Result<crate::governance::TimelineFilter> {
    let (start, end) = resolve_time_range(
        query.date.as_deref(),
        query.start.as_deref(),
        query.end.as_deref(),
    )?;
    Ok(crate::governance::TimelineFilter {
        start,
        end,
        device_id: clean_optional(query.device_id),
        event_id: None,
        app: clean_optional(query.app),
        domain: clean_optional(query.domain),
        category: clean_optional(query.category),
        query: clean_optional(query.q),
        limit: usize::from(query.limit.unwrap_or(100).clamp(1, 500)),
        offset: usize::try_from(query.offset.unwrap_or_default()).unwrap_or_default(),
    })
}

async fn bulk_delete_filter(
    state: &AppState,
    input: BulkDeleteInput,
) -> anyhow::Result<crate::governance::TimelineFilter> {
    let event_id = clean_optional(input.event_id);
    let (start, end) = if let Some(event_id) = event_id.as_deref() {
        let identity = crate::db::load_activity_identity(&state.pool(), event_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("activity does not exist"))?;
        (identity.ts, identity.ts + time::Duration::nanoseconds(1))
    } else {
        if input.date.is_none() && (input.start.is_none() || input.end.is_none()) {
            anyhow::bail!("bulk deletion requires an event, date, or explicit time range");
        }
        resolve_time_range(
            input.date.as_deref(),
            input.start.as_deref(),
            input.end.as_deref(),
        )?
    };
    Ok(crate::governance::TimelineFilter {
        start,
        end,
        device_id: clean_optional(input.device_id),
        event_id,
        app: clean_optional(input.app),
        domain: clean_optional(input.domain),
        category: clean_optional(input.category),
        query: None,
        limit: 1,
        offset: 0,
    })
}

fn resolve_time_range(
    date: Option<&str>,
    start: Option<&str>,
    end: Option<&str>,
) -> anyhow::Result<(time::OffsetDateTime, time::OffsetDateTime)> {
    if let Some(date) = date.map(str::trim).filter(|value| !value.is_empty()) {
        return crate::reports::date_bounds(date);
    }
    match (start, end) {
        (Some(start), Some(end)) => Ok((parse_range_start(start)?, parse_range_end(end)?)),
        (None, None) => {
            let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
            let date = time::OffsetDateTime::now_utc()
                .to_offset(offset)
                .date()
                .to_string();
            crate::reports::date_bounds(&date)
        }
        _ => anyhow::bail!("start and end must be supplied together"),
    }
}

fn validate_report_range(start: &str, end: &str) -> Result<(), StatusCode> {
    let start_bounds = crate::reports::date_bounds(start).map_err(|_| StatusCode::BAD_REQUEST)?;
    let end_bounds = crate::reports::date_bounds(end).map_err(|_| StatusCode::BAD_REQUEST)?;
    if end_bounds.0 < start_bounds.0 || end_bounds.1 - start_bounds.0 > time::Duration::days(3660) {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(())
}

fn parse_range_start(value: &str) -> anyhow::Result<time::OffsetDateTime> {
    let value = value.trim();
    if value.len() == 10 {
        return crate::reports::date_bounds(value).map(|bounds| bounds.0);
    }
    time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .map_err(Into::into)
}

fn parse_range_end(value: &str) -> anyhow::Result<time::OffsetDateTime> {
    let value = value.trim();
    if value.len() == 10 {
        return crate::reports::date_bounds(value).map(|bounds| bounds.1);
    }
    time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .map_err(Into::into)
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn is_agent_authorized(headers: &HeaderMap, expected_token: &str) -> bool {
    let Some(value) = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let Some(token) = value.strip_prefix("Bearer ") else {
        return false;
    };
    !expected_token.is_empty() && crate::auth::constant_time_eq(token, expected_token)
}

fn is_integration_authorized(headers: &HeaderMap, state: &AppState) -> bool {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|token| state.verify_integration_token(token))
}

fn internal_error(err: anyhow::Error) -> StatusCode {
    error!(error = %err, "request failed");
    StatusCode::INTERNAL_SERVER_ERROR
}

async fn stream(
    State(state): State<Arc<AppState>>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let initial_state = state.clone();
    let initial = futures_util::stream::once(async move {
        Ok(Event::default()
            .event("message")
            .json_data(StreamMessage::Snapshot(initial_state.snapshot()))
            .expect("serialize stream snapshot"))
    });

    let updates = BroadcastStream::new(state.subscribe())
        .filter_map(|result| result.ok())
        .map(|message| {
            Ok(Event::default()
                .event("message")
                .json_data(message)
                .expect("serialize stream message"))
        });

    let stream = initial.chain(updates);

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue, header::AUTHORIZATION};

    use super::is_agent_authorized;

    #[test]
    fn validates_agent_bearer_token() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer token-1"));

        assert!(is_agent_authorized(&headers, "token-1"));
        assert!(!is_agent_authorized(&headers, "token-2"));
        assert!(!is_agent_authorized(&HeaderMap::new(), "token-1"));
    }
}
