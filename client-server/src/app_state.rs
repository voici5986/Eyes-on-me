use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, RwLock},
};

use eyes_on_me_shared::{
    ActivityEvent, ActivityKind, ActivitySearchResponse, AnalysisOverviewResponse, AppUsageBucket,
    BrowserUsageBucket, CategoryRule, CategoryUsageBucket, DailyUsageBucket, DashboardSnapshot,
    DeviceAnalysisResponse, DeviceAnalysisSummary, DeviceDetailResponse, DeviceOverview,
    DeviceStatus, DevicesResponse, DomainUsageBucket, HourlyUsageBucket, PageUsageBucket,
    PresenceState, StreamMessage, UsageBucket, WorkScheduleSegment,
};
use sqlx::SqlitePool;
use time::{Duration as TimeDuration, OffsetDateTime, UtcOffset};
use tokio::sync::{Semaphore, broadcast};

const RECENT_ACTIVITY_LIMIT: usize = 20;
const DEVICE_ACTIVITY_LIMIT: i64 = 50;
const ANALYSIS_TOP_LIMIT: usize = 50;
const MAX_ACTIVITY_CONTINUITY: TimeDuration = TimeDuration::seconds(120);
const WORKDAY_START_HOUR: i64 = 9;
const WORKDAY_END_HOUR: i64 = 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisRange {
    Last3Hours,
    Last6Hours,
    Today,
    LastDay,
    LastWeek,
    LastMonth,
    All,
}

impl AnalysisRange {
    pub fn from_query(value: Option<&str>) -> Option<Self> {
        match value.unwrap_or("today") {
            "3h" => Some(Self::Last3Hours),
            "6h" => Some(Self::Last6Hours),
            "today" => Some(Self::Today),
            "1d" => Some(Self::LastDay),
            "1w" => Some(Self::LastWeek),
            "1m" => Some(Self::LastMonth),
            "all" => Some(Self::All),
            _ => None,
        }
    }

    fn window_start(self, now: OffsetDateTime) -> Option<OffsetDateTime> {
        self.window_start_with_offset(now, UtcOffset::current_local_offset().ok())
    }

    fn window_start_with_offset(
        self,
        now: OffsetDateTime,
        local_offset: Option<UtcOffset>,
    ) -> Option<OffsetDateTime> {
        match self {
            Self::Last3Hours => Some(now - TimeDuration::hours(3)),
            Self::Last6Hours => Some(now - TimeDuration::hours(6)),
            Self::Today => Some(start_of_local_day(
                now,
                local_offset.unwrap_or(UtcOffset::UTC),
            )),
            Self::LastDay => Some(now - TimeDuration::days(1)),
            Self::LastWeek => Some(now - TimeDuration::days(7)),
            Self::LastMonth => Some(now - TimeDuration::days(30)),
            Self::All => None,
        }
    }
}

fn start_of_local_day(now: OffsetDateTime, local_offset: UtcOffset) -> OffsetDateTime {
    now.to_offset(local_offset)
        .date()
        .midnight()
        .assume_offset(local_offset)
}

#[derive(Debug)]
struct SnapshotState {
    snapshot: DashboardSnapshot,
}

#[derive(Clone)]
pub struct AppState {
    inner: Arc<RwLock<SnapshotState>>,
    pool: SqlitePool,
    tx: broadcast::Sender<StreamMessage>,
    agent_api_token: Arc<str>,
    dashboard_token: Option<Arc<str>>,
    dashboard_session_token: Option<Arc<str>>,
    integration_token: Option<Arc<str>>,
    secure_cookie: bool,
    media_dir: Arc<std::path::PathBuf>,
    media_max_bytes: usize,
    media_total_max_bytes: u64,
    media_retention_days: u32,
    ocr_command: Option<Arc<str>>,
    ocr_language: Arc<str>,
    ocr_semaphore: Arc<Semaphore>,
    ocr_redact_terms: Arc<Vec<String>>,
    ai: Option<crate::config::AiConfig>,
    remote_storage: Option<crate::config::RemoteStorageConfig>,
}

impl AppState {
    pub fn new(
        snapshot: DashboardSnapshot,
        pool: SqlitePool,
        config: &crate::config::Config,
    ) -> Self {
        let (tx, _) = broadcast::channel(128);
        let dashboard_session_token = config
            .dashboard_token
            .as_deref()
            .map(crate::auth::derive_session_token)
            .map(Arc::from);
        Self {
            inner: Arc::new(RwLock::new(SnapshotState { snapshot })),
            pool,
            tx,
            agent_api_token: Arc::from(config.agent_api_token.clone()),
            dashboard_token: config.dashboard_token.clone().map(Arc::from),
            dashboard_session_token,
            integration_token: config.integration_token.clone().map(Arc::from),
            secure_cookie: config.secure_cookie,
            media_dir: Arc::new(config.media_dir.clone()),
            media_max_bytes: config.media_max_bytes,
            media_total_max_bytes: config.media_total_max_bytes,
            media_retention_days: config.media_retention_days,
            ocr_command: config.ocr_command.clone().map(Arc::from),
            ocr_language: Arc::from(config.ocr_language.clone()),
            ocr_semaphore: Arc::new(Semaphore::new(config.ocr_concurrency)),
            ocr_redact_terms: Arc::new(config.ocr_redact_terms.clone()),
            ai: config.ai.clone(),
            remote_storage: config.remote_storage.clone(),
        }
    }

    pub fn agent_api_token(&self) -> &str {
        &self.agent_api_token
    }

    pub fn dashboard_auth_required(&self) -> bool {
        self.dashboard_token.is_some()
    }

    pub fn verify_dashboard_token(&self, token: &str) -> bool {
        self.dashboard_token
            .as_deref()
            .map(|expected| crate::auth::constant_time_eq(expected, token))
            .unwrap_or(true)
    }

    pub fn dashboard_session_token(&self) -> Option<&str> {
        self.dashboard_session_token.as_deref()
    }

    pub fn is_dashboard_session_valid(&self, token: &str) -> bool {
        self.dashboard_session_token
            .as_deref()
            .map(|expected| crate::auth::constant_time_eq(expected, token))
            .unwrap_or(true)
    }

    pub fn secure_cookie(&self) -> bool {
        self.secure_cookie
    }

    pub fn verify_integration_token(&self, token: &str) -> bool {
        self.integration_token
            .as_deref()
            .map(|expected| crate::auth::constant_time_eq(expected, token))
            .unwrap_or(false)
    }

    pub fn integrations_enabled(&self) -> bool {
        self.integration_token.is_some()
    }

    pub fn pool(&self) -> SqlitePool {
        self.pool.clone()
    }

    pub fn media_dir(&self) -> &std::path::Path {
        self.media_dir.as_path()
    }

    pub fn media_max_bytes(&self) -> usize {
        self.media_max_bytes
    }

    pub fn media_total_max_bytes(&self) -> u64 {
        self.media_total_max_bytes
    }

    pub fn media_retention_days(&self) -> u32 {
        self.media_retention_days
    }

    pub fn ocr_command(&self) -> Option<&str> {
        self.ocr_command.as_deref()
    }

    pub fn ocr_language(&self) -> &str {
        &self.ocr_language
    }

    pub fn ocr_semaphore(&self) -> Arc<Semaphore> {
        Arc::clone(&self.ocr_semaphore)
    }

    pub fn ocr_redact_terms(&self) -> &[String] {
        self.ocr_redact_terms.as_slice()
    }

    pub fn ai_config(&self) -> Option<&crate::config::AiConfig> {
        self.ai.as_ref()
    }

    pub fn remote_storage_config(&self) -> Option<&crate::config::RemoteStorageConfig> {
        self.remote_storage.as_ref()
    }

    pub fn snapshot(&self) -> DashboardSnapshot {
        self.inner
            .read()
            .expect("snapshot lock poisoned")
            .snapshot
            .clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<StreamMessage> {
        self.tx.subscribe()
    }

    pub async fn reload_snapshot(&self) -> anyhow::Result<()> {
        let snapshot = crate::db::load_snapshot(&self.pool).await?;
        {
            let mut guard = self.inner.write().expect("snapshot lock poisoned");
            guard.snapshot = snapshot.clone();
        }
        self.broadcast_snapshot(snapshot);
        Ok(())
    }

    pub async fn upsert_activity(&self, event: ActivityEvent) -> anyhow::Result<()> {
        crate::db::persist_activity(&self.pool, &event).await?;
        let derived_status = derive_status_from_activity(&event);
        crate::db::persist_status(&self.pool, &derived_status).await?;
        let snapshot = {
            let mut guard = self.inner.write().expect("snapshot lock poisoned");
            let devices = upsert_device(&guard.snapshot.devices, &event);
            let recent_activities = push_recent_activity(&guard.snapshot.recent_activities, event);

            guard.snapshot.devices = devices;
            guard.snapshot.recent_activities = recent_activities;
            guard.snapshot.latest_status = Some(derived_status);
            guard.snapshot.clone()
        };

        self.broadcast_snapshot(snapshot);
        Ok(())
    }

    pub async fn update_status(&self, status: DeviceStatus) -> anyhow::Result<()> {
        crate::db::persist_status(&self.pool, &status).await?;
        let snapshot = {
            let mut guard = self.inner.write().expect("snapshot lock poisoned");
            guard.snapshot.latest_status = Some(status);
            guard.snapshot.clone()
        };

        self.broadcast_snapshot(snapshot);
        Ok(())
    }

    pub async fn devices_response(&self) -> anyhow::Result<DevicesResponse> {
        let snapshot = self.snapshot();
        let status_by_device = crate::db::load_device_statuses(&self.pool)
            .await?
            .into_iter()
            .map(|status| (status.device_id.clone(), status))
            .collect::<HashMap<_, _>>();

        let mut devices = Vec::new();
        for device in snapshot.devices {
            let recording = crate::db::load_recording_state(&self.pool, &device.device_id).await?;
            devices.push(DeviceOverview {
                latest_status: status_by_device.get(&device.device_id).cloned(),
                recording,
                device,
            });
        }

        Ok(DevicesResponse { devices })
    }

    pub async fn device_detail(
        &self,
        device_id: &str,
    ) -> anyhow::Result<Option<DeviceDetailResponse>> {
        let current_device = self
            .snapshot()
            .devices
            .into_iter()
            .find(|device| device.device_id == device_id)
            .or(crate::db::load_latest_activity_for_device(&self.pool, device_id).await?);

        let Some(device) = current_device else {
            return Ok(None);
        };

        let latest_status = crate::db::load_device_status(&self.pool, device_id).await?;
        let recent_activities = crate::db::load_recent_activities_for_device(
            &self.pool,
            device_id,
            DEVICE_ACTIVITY_LIMIT,
        )
        .await?;
        let recording = crate::db::load_recording_state(&self.pool, device_id).await?;

        Ok(Some(DeviceDetailResponse {
            device,
            latest_status,
            recent_activities,
            recording,
        }))
    }

    pub async fn analysis_overview(
        &self,
        range: AnalysisRange,
    ) -> anyhow::Result<AnalysisOverviewResponse> {
        let now = OffsetDateTime::now_utc();
        let snapshot = self.snapshot();
        let latest_statuses = crate::db::load_device_statuses(&self.pool)
            .await?
            .into_iter()
            .map(|status| (status.device_id.clone(), status))
            .collect::<HashMap<_, _>>();
        let settings = crate::governance::effective_settings(&self.pool).await?;

        let mut devices = Vec::new();
        let mut app_usage = HashMap::new();
        let mut domain_usage = HashMap::new();
        let mut browser_usage = HashMap::new();
        let mut total_tracked_ms = 0_u64;
        let mut work_tracked_ms = 0_u64;
        let mut after_hours_tracked_ms = 0_u64;
        let mut browser_tracked_ms = 0_u64;
        let mut idle_tracked_ms = 0_u64;
        let mut locked_tracked_ms = 0_u64;
        let mut app_keys = HashSet::new();
        let mut category_usage = HashMap::new();
        let mut hourly_usage = hourly_accumulators();
        let mut daily_usage = HashMap::new();

        for current_device in snapshot.devices {
            let activities = crate::db::load_analysis_activities_for_device(
                &self.pool,
                &current_device.device_id,
                range.window_start(now),
            )
            .await?;
            if activities.is_empty() {
                continue;
            }

            let analysis = build_device_analysis_payload(
                current_device.device_id.clone(),
                latest_statuses.get(&current_device.device_id).cloned(),
                activities,
                now,
                range,
                &settings.category_rules,
                &settings.work_schedule,
            );

            if analysis.total_tracked_ms == 0 {
                continue;
            }

            total_tracked_ms += analysis.total_tracked_ms;
            work_tracked_ms += analysis.work_tracked_ms;
            after_hours_tracked_ms += analysis.after_hours_tracked_ms;
            browser_tracked_ms += analysis.browser_tracked_ms;
            idle_tracked_ms += analysis.idle_tracked_ms;
            locked_tracked_ms += analysis.locked_tracked_ms;
            merge_app_usage_vec(&mut app_usage, &analysis.app_usage);
            merge_usage_vec(&mut domain_usage, &analysis.domain_usage);
            merge_browser_usage_vec(&mut browser_usage, &analysis.browser_usage);
            merge_category_usage_vec(&mut category_usage, &analysis.category_usage);
            merge_hourly_usage_vec(&mut hourly_usage, &analysis.hourly_usage);
            merge_daily_usage_vec(&mut daily_usage, &analysis.daily_usage);
            app_keys.extend(analysis.app_usage.iter().map(|bucket| bucket.key.clone()));

            devices.push(DeviceAnalysisSummary {
                device_id: current_device.device_id.clone(),
                platform: current_device.platform.clone(),
                current_label: analysis
                    .current_label
                    .clone()
                    .unwrap_or_else(|| current_activity_label(&current_device)),
                latest_status_text: analysis
                    .latest_status
                    .as_ref()
                    .map(|status| status.status_text.clone()),
                total_tracked_ms: analysis.total_tracked_ms,
                event_count: analysis.event_count,
                last_seen: current_device.ts,
            });
        }

        devices.sort_by(|a, b| {
            b.total_tracked_ms
                .cmp(&a.total_tracked_ms)
                .then_with(|| b.last_seen.cmp(&a.last_seen))
        });

        Ok(AnalysisOverviewResponse {
            generated_at: now,
            device_count: devices.len(),
            total_tracked_ms,
            work_tracked_ms,
            after_hours_tracked_ms,
            browser_tracked_ms,
            idle_tracked_ms,
            locked_tracked_ms,
            app_count: app_keys.len(),
            devices,
            top_app_usage: finalize_app_usage_map(app_usage),
            top_domain_usage: finalize_usage_map(domain_usage),
            top_browser_usage: finalize_browser_usage_map(browser_usage),
            category_usage: finalize_category_usage_map(category_usage),
            hourly_usage: finalize_hourly_usage(hourly_usage),
            daily_usage: finalize_daily_usage_map(daily_usage),
        })
    }

    pub async fn device_analysis(
        &self,
        device_id: &str,
        range: AnalysisRange,
    ) -> anyhow::Result<Option<DeviceAnalysisResponse>> {
        let now = OffsetDateTime::now_utc();
        let activities = crate::db::load_analysis_activities_for_device(
            &self.pool,
            device_id,
            range.window_start(now),
        )
        .await?;
        if activities.is_empty() {
            return Ok(None);
        }

        let latest_status = crate::db::load_device_status(&self.pool, device_id).await?;
        let settings = crate::governance::effective_settings(&self.pool).await?;
        Ok(Some(build_device_analysis_payload(
            device_id.to_string(),
            latest_status,
            activities,
            now,
            range,
            &settings.category_rules,
            &settings.work_schedule,
        )))
    }

    pub async fn search_activities(
        &self,
        query: &str,
        device_id: Option<&str>,
        limit: i64,
    ) -> anyhow::Result<ActivitySearchResponse> {
        let results = crate::db::search_activities(&self.pool, query, device_id, limit).await?;
        let total = crate::db::count_activity_search_results(&self.pool, query, device_id).await?;

        Ok(ActivitySearchResponse {
            query: query.to_string(),
            device_id: device_id.map(str::to_string),
            total: total.max(0) as usize,
            results,
        })
    }

    fn broadcast_snapshot(&self, snapshot: DashboardSnapshot) {
        let _ = self.tx.send(StreamMessage::Snapshot(snapshot));
    }
}

fn upsert_device(devices: &[ActivityEvent], event: &ActivityEvent) -> Vec<ActivityEvent> {
    let mut next = devices.to_vec();

    if let Some(existing) = next
        .iter_mut()
        .find(|item| item.device_id == event.device_id)
    {
        *existing = event.clone();
    } else {
        next.push(event.clone());
    }

    next.sort_by(|a, b| b.ts.cmp(&a.ts));
    next
}

fn push_recent_activity(current: &[ActivityEvent], event: ActivityEvent) -> Vec<ActivityEvent> {
    if matches!(event.kind, ActivityKind::ActivitySample) {
        return current.to_vec();
    }

    let mut queue: VecDeque<ActivityEvent> = current.iter().cloned().collect();
    queue.push_front(event);

    while queue.len() > RECENT_ACTIVITY_LIMIT {
        queue.pop_back();
    }

    queue.into_iter().collect()
}

fn derive_status_from_activity(event: &ActivityEvent) -> DeviceStatus {
    let status_text = if event.presence == PresenceState::Locked {
        "屏幕已锁定".to_string()
    } else if event.presence == PresenceState::Idle {
        "当前空闲中".to_string()
    } else if let Some(context) = event
        .browser
        .as_ref()
        .and_then(|browser| browser.page_title.as_deref())
        .or(event.window_title.as_deref())
        .or(event.app.title.as_deref())
    {
        if let Some(domain) = event
            .browser
            .as_ref()
            .and_then(|browser| browser.domain.as_deref())
        {
            format!("正在使用 {} · {}（{}）", event.app.name, context, domain)
        } else {
            format!("正在使用 {} · {}", event.app.name, context)
        }
    } else if let Some(domain) = event
        .browser
        .as_ref()
        .and_then(|browser| browser.domain.as_deref())
    {
        format!("正在使用 {} 浏览 {}", event.app.name, domain)
    } else {
        format!("正在使用 {}", event.app.name)
    };

    DeviceStatus {
        ts: event.ts,
        device_id: event.device_id.clone(),
        agent_name: event.agent_name.clone(),
        platform: event.platform.clone(),
        status_text,
        source: "auto-activity".to_string(),
    }
}

fn build_device_analysis_payload(
    device_id: String,
    latest_status: Option<DeviceStatus>,
    activities: Vec<ActivityEvent>,
    now: OffsetDateTime,
    range: AnalysisRange,
    category_rules: &[CategoryRule],
    work_schedule: &[WorkScheduleSegment],
) -> DeviceAnalysisResponse {
    let mut app_usage = HashMap::new();
    let mut domain_usage = HashMap::new();
    let mut browser_usage = HashMap::new();
    let mut total_tracked_ms = 0_u64;
    let mut work_tracked_ms = 0_u64;
    let mut browser_tracked_ms = 0_u64;
    let mut idle_tracked_ms = 0_u64;
    let mut locked_tracked_ms = 0_u64;
    let mut event_count = 0_usize;
    let window_start = range.window_start(now);
    let local_offset = UtcOffset::current_local_offset()
        .ok()
        .unwrap_or(UtcOffset::UTC);
    let mut app_keys = HashSet::new();
    let mut category_usage = HashMap::new();
    let mut hourly_usage = hourly_accumulators();
    let mut daily_usage = HashMap::new();
    let mut previous_app_key: Option<String> = None;
    let mut previous_window_key: Option<String> = None;
    let mut previous_domain_key: Option<String> = None;
    let mut previous_browser_key: Option<String> = None;
    let mut previous_page_key: Option<String> = None;

    for (index, activity) in activities.iter().enumerate() {
        let next_ts = activities.get(index + 1).map(|next| next.ts).unwrap_or(now);
        let Some((effective_start, effective_end)) =
            effective_segment_bounds(activity.ts, next_ts, window_start, now)
        else {
            continue;
        };
        let tracked_ms = duration_between(effective_start, effective_end);
        if tracked_ms == 0 {
            continue;
        }

        if matches!(activity.kind, ActivityKind::ForegroundChanged)
            && activity.presence == PresenceState::Active
        {
            event_count += 1;
        }

        if activity.presence != PresenceState::Active {
            match activity.presence {
                PresenceState::Idle => idle_tracked_ms += tracked_ms,
                PresenceState::Locked => locked_tracked_ms += tracked_ms,
                PresenceState::Active => {}
            }
            previous_app_key = None;
            previous_window_key = None;
            previous_domain_key = None;
            previous_browser_key = None;
            previous_page_key = None;
            continue;
        }

        let app_key = app_usage_key(activity);
        let window_key = window_usage_key(activity);
        let browser_key = browser_usage_key(activity);
        let domain_key = domain_usage_key(activity);
        let page_key = page_usage_key(activity);
        let app_session_started = previous_app_key.as_deref() != Some(app_key.as_str());
        let window_session_started = previous_window_key.as_deref() != Some(window_key.as_str());
        let browser_session_started = browser_key
            .as_deref()
            .is_some_and(|key| previous_browser_key.as_deref() != Some(key));
        let domain_session_started = domain_key
            .as_deref()
            .is_some_and(|key| previous_domain_key.as_deref() != Some(key));
        let page_session_started = page_key
            .as_deref()
            .is_some_and(|key| previous_page_key.as_deref() != Some(key));

        total_tracked_ms += tracked_ms;
        work_tracked_ms += crate::governance::duration_within_schedule(
            effective_start,
            effective_end,
            local_offset,
            work_schedule,
        );
        if activity.browser.is_some() {
            browser_tracked_ms += tracked_ms;
        }
        accumulate_app_usage(
            &mut app_usage,
            activity,
            tracked_ms,
            app_session_started,
            window_session_started,
        );
        accumulate_domain_usage(
            &mut domain_usage,
            activity,
            tracked_ms,
            domain_session_started,
        );
        accumulate_browser_usage(
            &mut browser_usage,
            activity,
            tracked_ms,
            browser_session_started,
            domain_session_started,
            page_session_started,
        );
        accumulate_category_usage(&mut category_usage, activity, tracked_ms, category_rules);
        accumulate_time_usage(
            &mut hourly_usage,
            &mut daily_usage,
            activity,
            effective_start,
            effective_end,
            local_offset,
            app_session_started,
        );
        app_keys.insert(app_key.clone());
        previous_app_key = Some(app_key);
        previous_window_key = Some(window_key);
        previous_browser_key = browser_key;
        previous_domain_key = domain_key;
        previous_page_key = page_key;
    }

    let current_label = activities
        .iter()
        .rev()
        .find(|activity| activity.presence == PresenceState::Active)
        .map(current_activity_label)
        .or_else(|| activities.last().map(current_activity_label));

    DeviceAnalysisResponse {
        device_id,
        generated_at: now,
        total_tracked_ms,
        work_tracked_ms,
        after_hours_tracked_ms: total_tracked_ms.saturating_sub(work_tracked_ms),
        browser_tracked_ms,
        idle_tracked_ms,
        locked_tracked_ms,
        app_count: app_keys.len(),
        event_count,
        current_label,
        latest_status,
        app_usage: finalize_app_usage_map(app_usage),
        domain_usage: finalize_usage_map(domain_usage),
        browser_usage: finalize_browser_usage_map(browser_usage),
        category_usage: finalize_category_usage_map(category_usage),
        hourly_usage: finalize_hourly_usage(hourly_usage),
        daily_usage: finalize_daily_usage_map(daily_usage),
    }
}

#[cfg(test)]
fn duration_within_window(
    start: OffsetDateTime,
    end: OffsetDateTime,
    window_start: Option<OffsetDateTime>,
    now: OffsetDateTime,
) -> u64 {
    let Some((effective_start, effective_end)) =
        effective_segment_bounds(start, end, window_start, now)
    else {
        return 0;
    };
    duration_between(effective_start, effective_end)
}

fn effective_segment_bounds(
    start: OffsetDateTime,
    end: OffsetDateTime,
    window_start: Option<OffsetDateTime>,
    now: OffsetDateTime,
) -> Option<(OffsetDateTime, OffsetDateTime)> {
    let continuity_end = start + MAX_ACTIVITY_CONTINUITY;
    let effective_start = match window_start {
        Some(cutoff) if start < cutoff => cutoff,
        _ => start,
    };
    let effective_end = if end > now { now } else { end }.min(continuity_end);

    if effective_end <= effective_start {
        return None;
    }

    Some((effective_start, effective_end))
}

fn duration_between(start: OffsetDateTime, end: OffsetDateTime) -> u64 {
    (end - start)
        .whole_milliseconds()
        .max(0)
        .try_into()
        .unwrap_or(0)
}

fn duration_within_workday(
    start: OffsetDateTime,
    end: OffsetDateTime,
    local_offset: UtcOffset,
) -> u64 {
    let start_local = start.to_offset(local_offset);
    let end_local = end.to_offset(local_offset);

    let mut total = overlap_with_workday_for_date(start_local.date(), start_local, end_local);
    if end_local.date() != start_local.date() {
        total += overlap_with_workday_for_date(end_local.date(), start_local, end_local);
    }
    total
}

fn overlap_with_workday_for_date(
    date: time::Date,
    start_local: OffsetDateTime,
    end_local: OffsetDateTime,
) -> u64 {
    let work_start = date.midnight().assume_offset(start_local.offset())
        + TimeDuration::hours(WORKDAY_START_HOUR);
    let work_end =
        date.midnight().assume_offset(start_local.offset()) + TimeDuration::hours(WORKDAY_END_HOUR);
    let effective_start = start_local.max(work_start);
    let effective_end = end_local.min(work_end);

    if effective_end <= effective_start {
        return 0;
    }

    duration_between(effective_start, effective_end)
}

fn app_usage_key(activity: &ActivityEvent) -> String {
    let identity = if activity.app.id.trim().is_empty() {
        activity.app.name.trim()
    } else {
        activity.app.id.trim()
    };
    format!("app:{}", identity.to_ascii_lowercase())
}

fn window_usage_key(activity: &ActivityEvent) -> String {
    let identity = activity
        .browser
        .as_ref()
        .and_then(|browser| browser.url.as_deref())
        .or_else(|| {
            activity
                .browser
                .as_ref()
                .and_then(|browser| browser.page_title.as_deref())
        })
        .or(activity.window_title.as_deref())
        .or(activity.app.title.as_deref())
        .unwrap_or(activity.app.name.as_str());
    format!(
        "{}:window:{}",
        app_usage_key(activity),
        identity.to_lowercase()
    )
}

#[derive(Debug, Clone)]
struct AppUsageAccumulator {
    key: String,
    label: String,
    sublabel: Option<String>,
    total_tracked_ms: u64,
    sessions: u32,
    last_seen: OffsetDateTime,
    windows: HashMap<String, UsageBucket>,
}

fn accumulate_app_usage(
    target: &mut HashMap<String, AppUsageAccumulator>,
    activity: &ActivityEvent,
    tracked_ms: u64,
    app_session_started: bool,
    window_session_started: bool,
) {
    let key = app_usage_key(activity);
    let app_entry = target
        .entry(key.clone())
        .or_insert_with(|| AppUsageAccumulator {
            key,
            label: activity.app.name.clone(),
            sublabel: Some(activity.app.id.clone()),
            total_tracked_ms: 0,
            sessions: 0,
            last_seen: activity.ts,
            windows: HashMap::new(),
        });
    app_entry.total_tracked_ms += tracked_ms;
    app_entry.sessions += u32::from(app_session_started);
    if activity.ts >= app_entry.last_seen {
        app_entry.last_seen = activity.ts;
        app_entry.label = activity.app.name.clone();
        app_entry.sublabel = Some(activity.app.id.clone());
    }

    let window_key = window_usage_key(activity);
    let window_label = activity
        .browser
        .as_ref()
        .and_then(|browser| browser.page_title.clone())
        .or_else(|| activity.window_title.clone())
        .or_else(|| activity.app.title.clone())
        .unwrap_or_else(|| "未命名窗口".to_string());
    let window_sublabel = activity
        .browser
        .as_ref()
        .and_then(|browser| browser.domain.clone().or_else(|| browser.url.clone()));

    merge_usage_entry(
        &mut app_entry.windows,
        UsageBucket {
            key: window_key,
            label: window_label,
            sublabel: window_sublabel,
            total_tracked_ms: tracked_ms,
            sessions: u32::from(window_session_started),
            last_seen: activity.ts,
        },
    );
}

fn accumulate_domain_usage(
    target: &mut HashMap<String, UsageBucket>,
    activity: &ActivityEvent,
    tracked_ms: u64,
    session_started: bool,
) {
    let Some(browser) = &activity.browser else {
        return;
    };
    let Some(domain) = browser.domain.as_ref() else {
        return;
    };

    let sublabel = browser
        .page_title
        .clone()
        .or_else(|| Some(activity.app.name.clone()));

    merge_usage_entry(
        target,
        UsageBucket {
            key: domain.to_lowercase(),
            label: domain.clone(),
            sublabel,
            total_tracked_ms: tracked_ms,
            sessions: u32::from(session_started),
            last_seen: activity.ts,
        },
    );
}

fn browser_usage_key(activity: &ActivityEvent) -> Option<String> {
    let browser = activity.browser.as_ref()?;
    Some(format!(
        "browser:{}:{}",
        browser.family.to_ascii_lowercase(),
        activity.app.id.to_ascii_lowercase()
    ))
}

fn domain_usage_key(activity: &ActivityEvent) -> Option<String> {
    activity
        .browser
        .as_ref()?
        .domain
        .as_ref()
        .map(|domain| domain.to_ascii_lowercase())
}

fn page_usage_key(activity: &ActivityEvent) -> Option<String> {
    let browser = activity.browser.as_ref()?;
    browser
        .url
        .clone()
        .filter(|url| !url.is_empty())
        .or_else(|| browser.page_title.clone().filter(|title| !title.is_empty()))
        .or_else(|| browser.domain.clone().filter(|domain| !domain.is_empty()))
}

#[derive(Debug, Clone)]
struct PageUsageAccumulator {
    key: String,
    label: String,
    url: Option<String>,
    total_tracked_ms: u64,
    sessions: u32,
    last_seen: OffsetDateTime,
}

#[derive(Debug, Clone)]
struct DomainUsageAccumulator {
    key: String,
    label: String,
    total_tracked_ms: u64,
    sessions: u32,
    last_seen: OffsetDateTime,
    pages: HashMap<String, PageUsageAccumulator>,
}

#[derive(Debug, Clone)]
struct BrowserUsageAccumulator {
    key: String,
    label: String,
    family: String,
    total_tracked_ms: u64,
    sessions: u32,
    last_seen: OffsetDateTime,
    domains: HashMap<String, DomainUsageAccumulator>,
}

fn accumulate_browser_usage(
    target: &mut HashMap<String, BrowserUsageAccumulator>,
    activity: &ActivityEvent,
    tracked_ms: u64,
    browser_session_started: bool,
    domain_session_started: bool,
    page_session_started: bool,
) {
    let Some(browser) = activity.browser.as_ref() else {
        return;
    };

    let browser_key = browser_usage_key(activity).expect("browser activity has a key");
    let browser_entry =
        target
            .entry(browser_key.clone())
            .or_insert_with(|| BrowserUsageAccumulator {
                key: browser_key,
                label: activity.app.name.clone(),
                family: browser.family.clone(),
                total_tracked_ms: 0,
                sessions: 0,
                last_seen: activity.ts,
                domains: HashMap::new(),
            });

    browser_entry.total_tracked_ms += tracked_ms;
    browser_entry.sessions += u32::from(browser_session_started);
    if activity.ts >= browser_entry.last_seen {
        browser_entry.last_seen = activity.ts;
        browser_entry.label = activity.app.name.clone();
    }

    let Some(domain) = browser.domain.as_ref() else {
        return;
    };

    let domain_key = domain.to_ascii_lowercase();
    let domain_entry = browser_entry
        .domains
        .entry(domain_key.clone())
        .or_insert_with(|| DomainUsageAccumulator {
            key: domain_key,
            label: domain.clone(),
            total_tracked_ms: 0,
            sessions: 0,
            last_seen: activity.ts,
            pages: HashMap::new(),
        });

    domain_entry.total_tracked_ms += tracked_ms;
    domain_entry.sessions += u32::from(domain_session_started);
    if activity.ts >= domain_entry.last_seen {
        domain_entry.last_seen = activity.ts;
        domain_entry.label = domain.clone();
    }

    let page_key = browser
        .url
        .clone()
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| {
            browser
                .page_title
                .clone()
                .filter(|title| !title.is_empty())
                .unwrap_or_else(|| domain.clone())
        });
    let page_label = browser
        .page_title
        .clone()
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| domain.clone());

    let page_entry = domain_entry
        .pages
        .entry(page_key.clone())
        .or_insert_with(|| PageUsageAccumulator {
            key: page_key,
            label: page_label.clone(),
            url: browser.url.clone(),
            total_tracked_ms: 0,
            sessions: 0,
            last_seen: activity.ts,
        });

    page_entry.total_tracked_ms += tracked_ms;
    page_entry.sessions += u32::from(page_session_started);
    if activity.ts >= page_entry.last_seen {
        page_entry.last_seen = activity.ts;
        page_entry.label = page_label;
        page_entry.url = browser.url.clone();
    }
}

#[derive(Debug, Clone, Default)]
struct HourlyUsageAccumulator {
    total_tracked_ms: u64,
    apps: HashMap<String, UsageBucket>,
}

#[derive(Debug, Clone, Default)]
struct DailyUsageAccumulator {
    total_tracked_ms: u64,
    work_tracked_ms: u64,
    browser_tracked_ms: u64,
}

fn hourly_accumulators() -> Vec<HourlyUsageAccumulator> {
    (0..24).map(|_| HourlyUsageAccumulator::default()).collect()
}

fn accumulate_category_usage(
    target: &mut HashMap<String, CategoryUsageBucket>,
    activity: &ActivityEvent,
    tracked_ms: u64,
    category_rules: &[CategoryRule],
) {
    let (key, label) = crate::governance::classify_activity(activity, category_rules);
    let entry = target
        .entry(key.clone())
        .or_insert_with(|| CategoryUsageBucket {
            key,
            label,
            total_tracked_ms: 0,
        });
    entry.total_tracked_ms += tracked_ms;
}

fn accumulate_time_usage(
    hourly: &mut [HourlyUsageAccumulator],
    daily: &mut HashMap<String, DailyUsageAccumulator>,
    activity: &ActivityEvent,
    start: OffsetDateTime,
    end: OffsetDateTime,
    local_offset: UtcOffset,
    session_started: bool,
) {
    let mut cursor = start;
    while cursor < end {
        let local_cursor = cursor.to_offset(local_offset);
        let bucket_end = next_local_hour_boundary(cursor, local_offset).min(end);
        let chunk_ms = duration_between(cursor, bucket_end);
        if chunk_ms == 0 {
            break;
        }

        let hour = local_cursor.hour() as usize;
        if let Some(hour_bucket) = hourly.get_mut(hour) {
            hour_bucket.total_tracked_ms += chunk_ms;
            merge_usage_entry(
                &mut hour_bucket.apps,
                UsageBucket {
                    key: app_usage_key(activity),
                    label: activity.app.name.clone(),
                    sublabel: None,
                    total_tracked_ms: chunk_ms,
                    sessions: u32::from(session_started || cursor > start),
                    last_seen: activity.ts,
                },
            );
        }

        let day = daily.entry(local_cursor.date().to_string()).or_default();
        day.total_tracked_ms += chunk_ms;
        day.work_tracked_ms += duration_within_workday(cursor, bucket_end, local_offset);
        if activity.browser.is_some() {
            day.browser_tracked_ms += chunk_ms;
        }

        cursor = bucket_end;
    }
}

fn next_local_hour_boundary(value: OffsetDateTime, local_offset: UtcOffset) -> OffsetDateTime {
    let local = value.to_offset(local_offset);
    let next = if local.hour() == 23 {
        local.date().next_day().unwrap_or(local.date()).midnight()
    } else {
        local
            .date()
            .with_hms(local.hour() + 1, 0, 0)
            .expect("valid next local hour")
    };
    next.assume_offset(local_offset).to_offset(UtcOffset::UTC)
}

fn merge_app_usage_vec(
    target: &mut HashMap<String, AppUsageAccumulator>,
    source: &[AppUsageBucket],
) {
    for app in source {
        let entry = target
            .entry(app.key.clone())
            .or_insert_with(|| AppUsageAccumulator {
                key: app.key.clone(),
                label: app.label.clone(),
                sublabel: app.sublabel.clone(),
                total_tracked_ms: 0,
                sessions: 0,
                last_seen: app.last_seen,
                windows: HashMap::new(),
            });
        entry.total_tracked_ms += app.total_tracked_ms;
        entry.sessions += app.sessions;
        if app.last_seen >= entry.last_seen {
            entry.last_seen = app.last_seen;
            entry.label = app.label.clone();
            entry.sublabel = app.sublabel.clone();
        }
        for window in &app.windows {
            merge_usage_entry(&mut entry.windows, window.clone());
        }
    }
}

fn merge_category_usage_vec(
    target: &mut HashMap<String, CategoryUsageBucket>,
    source: &[CategoryUsageBucket],
) {
    for category in source {
        match target.get_mut(&category.key) {
            Some(entry) => {
                entry.total_tracked_ms += category.total_tracked_ms;
                entry.label = category.label.clone();
            }
            None => {
                target.insert(category.key.clone(), category.clone());
            }
        }
    }
}

fn merge_hourly_usage_vec(target: &mut [HourlyUsageAccumulator], source: &[HourlyUsageBucket]) {
    for bucket in source {
        let Some(entry) = target.get_mut(bucket.hour as usize) else {
            continue;
        };
        entry.total_tracked_ms += bucket.total_tracked_ms;
        for app in &bucket.apps {
            merge_usage_entry(&mut entry.apps, app.clone());
        }
    }
}

fn merge_daily_usage_vec(
    target: &mut HashMap<String, DailyUsageAccumulator>,
    source: &[DailyUsageBucket],
) {
    for bucket in source {
        let entry = target.entry(bucket.date.clone()).or_default();
        entry.total_tracked_ms += bucket.total_tracked_ms;
        entry.work_tracked_ms += bucket.work_tracked_ms;
        entry.browser_tracked_ms += bucket.browser_tracked_ms;
    }
}

fn merge_usage_vec(target: &mut HashMap<String, UsageBucket>, source: &[UsageBucket]) {
    for bucket in source {
        merge_usage_entry(target, bucket.clone());
    }
}

fn merge_browser_usage_vec(
    target: &mut HashMap<String, BrowserUsageAccumulator>,
    source: &[BrowserUsageBucket],
) {
    for browser in source {
        let browser_entry =
            target
                .entry(browser.key.clone())
                .or_insert_with(|| BrowserUsageAccumulator {
                    key: browser.key.clone(),
                    label: browser.label.clone(),
                    family: browser.family.clone(),
                    total_tracked_ms: 0,
                    sessions: 0,
                    last_seen: browser.last_seen,
                    domains: HashMap::new(),
                });

        browser_entry.total_tracked_ms += browser.total_tracked_ms;
        browser_entry.sessions += browser.sessions;
        if browser.last_seen >= browser_entry.last_seen {
            browser_entry.last_seen = browser.last_seen;
            browser_entry.label = browser.label.clone();
            browser_entry.family = browser.family.clone();
        }

        for domain in &browser.domains {
            let domain_entry = browser_entry
                .domains
                .entry(domain.key.clone())
                .or_insert_with(|| DomainUsageAccumulator {
                    key: domain.key.clone(),
                    label: domain.label.clone(),
                    total_tracked_ms: 0,
                    sessions: 0,
                    last_seen: domain.last_seen,
                    pages: HashMap::new(),
                });

            domain_entry.total_tracked_ms += domain.total_tracked_ms;
            domain_entry.sessions += domain.sessions;
            if domain.last_seen >= domain_entry.last_seen {
                domain_entry.last_seen = domain.last_seen;
                domain_entry.label = domain.label.clone();
            }

            for page in &domain.pages {
                let page_entry = domain_entry
                    .pages
                    .entry(page.key.clone())
                    .or_insert_with(|| PageUsageAccumulator {
                        key: page.key.clone(),
                        label: page.label.clone(),
                        url: page.url.clone(),
                        total_tracked_ms: 0,
                        sessions: 0,
                        last_seen: page.last_seen,
                    });

                page_entry.total_tracked_ms += page.total_tracked_ms;
                page_entry.sessions += page.sessions;
                if page.last_seen >= page_entry.last_seen {
                    page_entry.last_seen = page.last_seen;
                    page_entry.label = page.label.clone();
                    page_entry.url = page.url.clone();
                }
            }
        }
    }
}

fn merge_usage_entry(target: &mut HashMap<String, UsageBucket>, incoming: UsageBucket) {
    match target.get_mut(&incoming.key) {
        Some(existing) => {
            existing.total_tracked_ms += incoming.total_tracked_ms;
            existing.sessions += incoming.sessions;
            if incoming.last_seen > existing.last_seen {
                existing.last_seen = incoming.last_seen;
                existing.sublabel = incoming.sublabel;
            }
        }
        None => {
            target.insert(incoming.key.clone(), incoming);
        }
    }
}

fn finalize_app_usage_map(map: HashMap<String, AppUsageAccumulator>) -> Vec<AppUsageBucket> {
    let mut items = map
        .into_values()
        .map(|app| {
            let mut windows = app.windows.into_values().collect::<Vec<_>>();
            windows.sort_by(|a, b| {
                b.total_tracked_ms
                    .cmp(&a.total_tracked_ms)
                    .then_with(|| b.last_seen.cmp(&a.last_seen))
            });
            AppUsageBucket {
                key: app.key,
                label: app.label,
                sublabel: app.sublabel,
                total_tracked_ms: app.total_tracked_ms,
                sessions: app.sessions,
                last_seen: app.last_seen,
                windows,
            }
        })
        .collect::<Vec<_>>();
    items.sort_by(|a, b| {
        b.total_tracked_ms
            .cmp(&a.total_tracked_ms)
            .then_with(|| b.last_seen.cmp(&a.last_seen))
    });
    items.truncate(ANALYSIS_TOP_LIMIT);
    items
}

fn finalize_category_usage_map(
    map: HashMap<String, CategoryUsageBucket>,
) -> Vec<CategoryUsageBucket> {
    let mut items = map.into_values().collect::<Vec<_>>();
    items.sort_by(|a, b| {
        b.total_tracked_ms
            .cmp(&a.total_tracked_ms)
            .then_with(|| a.label.cmp(&b.label))
    });
    items
}

fn finalize_hourly_usage(map: Vec<HourlyUsageAccumulator>) -> Vec<HourlyUsageBucket> {
    map.into_iter()
        .enumerate()
        .map(|(hour, bucket)| HourlyUsageBucket {
            hour: hour as u8,
            total_tracked_ms: bucket.total_tracked_ms,
            apps: finalize_usage_map_with_limit(bucket.apps, 5),
        })
        .collect()
}

fn finalize_daily_usage_map(map: HashMap<String, DailyUsageAccumulator>) -> Vec<DailyUsageBucket> {
    let mut items = map
        .into_iter()
        .map(|(date, bucket)| DailyUsageBucket {
            date,
            total_tracked_ms: bucket.total_tracked_ms,
            work_tracked_ms: bucket.work_tracked_ms,
            browser_tracked_ms: bucket.browser_tracked_ms,
        })
        .collect::<Vec<_>>();
    items.sort_by(|a, b| a.date.cmp(&b.date));
    items
}

fn finalize_usage_map(map: HashMap<String, UsageBucket>) -> Vec<UsageBucket> {
    finalize_usage_map_with_limit(map, ANALYSIS_TOP_LIMIT)
}

fn finalize_usage_map_with_limit(
    map: HashMap<String, UsageBucket>,
    limit: usize,
) -> Vec<UsageBucket> {
    let mut items = map.into_values().collect::<Vec<_>>();
    items.sort_by(|a, b| {
        b.total_tracked_ms
            .cmp(&a.total_tracked_ms)
            .then_with(|| b.last_seen.cmp(&a.last_seen))
    });
    items.truncate(limit);
    items
}

fn finalize_browser_usage_map(
    map: HashMap<String, BrowserUsageAccumulator>,
) -> Vec<BrowserUsageBucket> {
    let mut items = map
        .into_values()
        .map(|browser| BrowserUsageBucket {
            key: browser.key,
            label: browser.label,
            family: browser.family,
            total_tracked_ms: browser.total_tracked_ms,
            sessions: browser.sessions,
            last_seen: browser.last_seen,
            domains: finalize_domain_usage_map(browser.domains),
        })
        .collect::<Vec<_>>();

    items.sort_by(|a, b| {
        b.total_tracked_ms
            .cmp(&a.total_tracked_ms)
            .then_with(|| b.last_seen.cmp(&a.last_seen))
    });
    items.truncate(ANALYSIS_TOP_LIMIT);
    items
}

fn finalize_domain_usage_map(
    map: HashMap<String, DomainUsageAccumulator>,
) -> Vec<DomainUsageBucket> {
    let mut items = map
        .into_values()
        .map(|domain| DomainUsageBucket {
            key: domain.key,
            label: domain.label,
            total_tracked_ms: domain.total_tracked_ms,
            sessions: domain.sessions,
            last_seen: domain.last_seen,
            pages: finalize_page_usage_map(domain.pages),
        })
        .collect::<Vec<_>>();

    items.sort_by(|a, b| {
        b.total_tracked_ms
            .cmp(&a.total_tracked_ms)
            .then_with(|| b.last_seen.cmp(&a.last_seen))
    });
    items.truncate(ANALYSIS_TOP_LIMIT);
    items
}

fn finalize_page_usage_map(map: HashMap<String, PageUsageAccumulator>) -> Vec<PageUsageBucket> {
    let mut items = map
        .into_values()
        .map(|page| PageUsageBucket {
            key: page.key,
            label: page.label,
            url: page.url,
            total_tracked_ms: page.total_tracked_ms,
            sessions: page.sessions,
            last_seen: page.last_seen,
        })
        .collect::<Vec<_>>();

    items.sort_by(|a, b| {
        b.total_tracked_ms
            .cmp(&a.total_tracked_ms)
            .then_with(|| b.last_seen.cmp(&a.last_seen))
    });
    items.truncate(ANALYSIS_TOP_LIMIT);
    items
}

fn current_activity_label(activity: &ActivityEvent) -> String {
    match activity.presence {
        PresenceState::Locked => "屏幕已锁定".to_string(),
        PresenceState::Idle => "当前空闲中".to_string(),
        PresenceState::Active => activity
            .browser
            .as_ref()
            .and_then(|browser| browser.page_title.clone())
            .or_else(|| activity.window_title.clone())
            .or_else(|| activity.app.title.clone())
            .unwrap_or_else(|| activity.app.name.clone()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use eyes_on_me_shared::{
        ActivityApp, ActivityEvent, ActivityKind, BrowserContext, Platform, PresenceState,
    };

    use super::{
        AnalysisRange, MAX_ACTIVITY_CONTINUITY, accumulate_app_usage, accumulate_browser_usage,
        accumulate_time_usage, build_device_analysis_payload, duration_within_window,
        finalize_app_usage_map, finalize_browser_usage_map, hourly_accumulators,
    };
    use time::{Date, Duration as TimeDuration, Month, OffsetDateTime, UtcOffset};

    #[test]
    fn parses_expected_analysis_ranges() {
        assert_eq!(AnalysisRange::from_query(None), Some(AnalysisRange::Today));
        assert_eq!(
            AnalysisRange::from_query(Some("3h")),
            Some(AnalysisRange::Last3Hours)
        );
        assert_eq!(
            AnalysisRange::from_query(Some("6h")),
            Some(AnalysisRange::Last6Hours)
        );
        assert_eq!(
            AnalysisRange::from_query(Some("today")),
            Some(AnalysisRange::Today)
        );
        assert_eq!(
            AnalysisRange::from_query(Some("1d")),
            Some(AnalysisRange::LastDay)
        );
        assert_eq!(
            AnalysisRange::from_query(Some("1w")),
            Some(AnalysisRange::LastWeek)
        );
        assert_eq!(
            AnalysisRange::from_query(Some("1m")),
            Some(AnalysisRange::LastMonth)
        );
        assert_eq!(
            AnalysisRange::from_query(Some("all")),
            Some(AnalysisRange::All)
        );
        assert_eq!(AnalysisRange::from_query(Some("bogus")), None);
    }

    #[test]
    fn today_range_uses_local_day_boundary() {
        let offset = UtcOffset::from_hms(8, 0, 0).expect("valid offset");
        let local_date = Date::from_calendar_date(2026, Month::March, 27).expect("valid date");
        let local_now =
            (local_date.midnight() + TimeDuration::hours(11) + TimeDuration::minutes(7))
                .assume_offset(offset);
        let now = local_now.to_offset(UtcOffset::UTC);

        let window_start = AnalysisRange::Today
            .window_start_with_offset(now, Some(offset))
            .expect("today should have a window start");

        assert_eq!(window_start, local_date.midnight().assume_offset(offset));
    }

    #[test]
    fn truncates_activity_duration_to_window_start() {
        let now = OffsetDateTime::now_utc();
        let start = now - TimeDuration::hours(4);
        let end = now - TimeDuration::hours(1);
        let cutoff = Some(now - TimeDuration::hours(3));

        let tracked_ms = duration_within_window(start, end, cutoff, now);

        assert_eq!(tracked_ms, 0);
    }

    #[test]
    fn ignores_segments_outside_window() {
        let now = OffsetDateTime::now_utc();
        let start = now - TimeDuration::hours(6);
        let end = now - TimeDuration::hours(5);
        let cutoff = Some(now - TimeDuration::hours(3));

        assert_eq!(duration_within_window(start, end, cutoff, now), 0);
    }

    #[test]
    fn groups_browser_activity_under_browser_app_in_app_usage() {
        let now = OffsetDateTime::now_utc();
        let activity = ActivityEvent {
            event_id: "evt-1".to_string(),
            ts: now,
            device_id: "windows-agent".to_string(),
            agent_name: "client-desktop".to_string(),
            platform: Platform::Windows,
            kind: ActivityKind::ForegroundChanged,
            app: ActivityApp {
                id: "msedge.exe".to_string(),
                name: "Microsoft Edge".to_string(),
                title: None,
                pid: Some(1234),
            },
            window_title: Some("Chaoleme/Eyes-on-me and 45 more pages - Personal".to_string()),
            browser: Some(BrowserContext {
                family: "chromium".to_string(),
                name: "Microsoft Edge".to_string(),
                page_title: Some("Chaoleme/Eyes-on-me and 45 more pages - Personal".to_string()),
                url: Some("https://github.com/Chaoleme/Eyes-on-me".to_string()),
                domain: Some("github.com".to_string()),
                source: "window-title".to_string(),
                confidence: 0.9,
            }),
            presence: PresenceState::Active,
            source: "desktop".to_string(),
        };

        let mut usage = HashMap::new();
        accumulate_app_usage(&mut usage, &activity, 60_000, true, true);

        let bucket = usage.get("app:msedge.exe").expect("browser app bucket");
        assert_eq!(bucket.label, "Microsoft Edge");
        assert_eq!(bucket.sublabel.as_deref(), Some("msedge.exe"));
        assert_eq!(bucket.windows.len(), 1);
    }

    #[test]
    fn groups_terminal_tabs_under_one_app_with_window_breakdown() {
        let now = OffsetDateTime::now_utc();
        let base = ActivityEvent {
            event_id: "terminal-1".to_string(),
            ts: now - TimeDuration::minutes(2),
            device_id: "mac-agent".to_string(),
            agent_name: "client-desktop".to_string(),
            platform: Platform::Macos,
            kind: ActivityKind::ForegroundChanged,
            app: ActivityApp {
                id: "com.apple.Terminal".to_string(),
                name: "Terminal".to_string(),
                title: None,
                pid: Some(42),
            },
            window_title: Some("Eyes_on_me - zsh".to_string()),
            browser: None,
            presence: PresenceState::Active,
            source: "desktop".to_string(),
        };
        let mut second = base.clone();
        second.event_id = "terminal-2".to_string();
        second.ts = now - TimeDuration::minutes(1);
        second.window_title = Some("Work_Review - git".to_string());

        let mut usage = HashMap::new();
        accumulate_app_usage(&mut usage, &base, 60_000, true, true);
        accumulate_app_usage(&mut usage, &second, 60_000, false, true);
        let apps = finalize_app_usage_map(usage);

        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].label, "Terminal");
        assert_eq!(apps[0].total_tracked_ms, 120_000);
        assert_eq!(apps[0].sessions, 1);
        assert_eq!(apps[0].windows.len(), 2);
        assert!(
            apps[0]
                .windows
                .iter()
                .any(|window| window.label == "Eyes_on_me - zsh")
        );
        assert!(
            apps[0]
                .windows
                .iter()
                .any(|window| window.label == "Work_Review - git")
        );
    }

    #[test]
    fn splits_hourly_usage_on_fractional_timezone_local_hour() {
        let offset = UtcOffset::from_hms(5, 30, 0).expect("valid offset");
        let date = Date::from_calendar_date(2026, Month::July, 26).expect("valid date");
        let start = (date.with_hms(10, 59, 30).expect("valid time"))
            .assume_offset(offset)
            .to_offset(UtcOffset::UTC);
        let end = start + TimeDuration::minutes(1);
        let activity = ActivityEvent {
            event_id: "fractional-zone".to_string(),
            ts: start,
            device_id: "device".to_string(),
            agent_name: "client-desktop".to_string(),
            platform: Platform::Linux,
            kind: ActivityKind::ForegroundChanged,
            app: ActivityApp {
                id: "terminal".to_string(),
                name: "Terminal".to_string(),
                title: None,
                pid: None,
            },
            window_title: Some("work".to_string()),
            browser: None,
            presence: PresenceState::Active,
            source: "test".to_string(),
        };
        let mut hourly = hourly_accumulators();
        let mut daily = HashMap::new();

        accumulate_time_usage(&mut hourly, &mut daily, &activity, start, end, offset, true);

        assert_eq!(hourly[10].total_tracked_ms, 30_000);
        assert_eq!(hourly[11].total_tracked_ms, 30_000);
    }

    #[test]
    fn caps_large_gaps_between_events() {
        let now = OffsetDateTime::now_utc();
        let start = now - TimeDuration::hours(8);
        let end = start + TimeDuration::hours(2);

        let tracked_ms = duration_within_window(start, end, None, now);

        assert_eq!(
            tracked_ms,
            MAX_ACTIVITY_CONTINUITY.whole_milliseconds() as u64
        );
    }

    #[test]
    fn builds_nested_browser_usage() {
        let now = OffsetDateTime::now_utc();
        let activity = ActivityEvent {
            event_id: "evt-2".to_string(),
            ts: now,
            device_id: "mac-agent".to_string(),
            agent_name: "client-desktop".to_string(),
            platform: Platform::Macos,
            kind: ActivityKind::ForegroundChanged,
            app: ActivityApp {
                id: "com.google.Chrome".to_string(),
                name: "Google Chrome".to_string(),
                title: None,
                pid: Some(777),
            },
            window_title: Some("Docs".to_string()),
            browser: Some(BrowserContext {
                family: "chromium".to_string(),
                name: "Google Chrome".to_string(),
                page_title: Some("Engineering Spec".to_string()),
                url: Some("https://docs.example.com/spec".to_string()),
                domain: Some("docs.example.com".to_string()),
                source: "test".to_string(),
                confidence: 0.95,
            }),
            presence: PresenceState::Active,
            source: "desktop".to_string(),
        };

        let mut usage = HashMap::new();
        accumulate_browser_usage(&mut usage, &activity, 120_000, true, true, true);
        let browsers = finalize_browser_usage_map(usage);

        assert_eq!(browsers.len(), 1);
        assert_eq!(browsers[0].label, "Google Chrome");
        assert_eq!(browsers[0].domains.len(), 1);
        assert_eq!(browsers[0].domains[0].label, "docs.example.com");
        assert_eq!(browsers[0].domains[0].pages[0].label, "Engineering Spec");
        assert_eq!(
            browsers[0].domains[0].pages[0].url.as_deref(),
            Some("https://docs.example.com/spec")
        );
    }

    #[test]
    fn keeps_latest_active_activity_for_current_label() {
        let now = OffsetDateTime::now_utc();
        let activities = vec![
            ActivityEvent {
                event_id: "evt-older".to_string(),
                ts: now - TimeDuration::minutes(5),
                device_id: "mac-agent".to_string(),
                agent_name: "client-desktop".to_string(),
                platform: Platform::Macos,
                kind: ActivityKind::ForegroundChanged,
                app: ActivityApp {
                    id: "com.google.Chrome".to_string(),
                    name: "Google Chrome".to_string(),
                    title: None,
                    pid: Some(777),
                },
                window_title: Some("Older page".to_string()),
                browser: None,
                presence: PresenceState::Active,
                source: "desktop".to_string(),
            },
            ActivityEvent {
                event_id: "evt-newer".to_string(),
                ts: now - TimeDuration::seconds(10),
                device_id: "mac-agent".to_string(),
                agent_name: "client-desktop".to_string(),
                platform: Platform::Macos,
                kind: ActivityKind::ForegroundChanged,
                app: ActivityApp {
                    id: "com.google.Chrome".to_string(),
                    name: "Google Chrome".to_string(),
                    title: None,
                    pid: Some(777),
                },
                window_title: Some("Latest page".to_string()),
                browser: None,
                presence: PresenceState::Active,
                source: "desktop".to_string(),
            },
            ActivityEvent {
                event_id: "evt-idle".to_string(),
                ts: now - TimeDuration::seconds(5),
                device_id: "mac-agent".to_string(),
                agent_name: "client-desktop".to_string(),
                platform: Platform::Macos,
                kind: ActivityKind::PresenceChanged,
                app: ActivityApp {
                    id: "com.google.Chrome".to_string(),
                    name: "Google Chrome".to_string(),
                    title: None,
                    pid: Some(777),
                },
                window_title: Some("Idle page".to_string()),
                browser: None,
                presence: PresenceState::Idle,
                source: "desktop".to_string(),
            },
        ];

        let analysis = build_device_analysis_payload(
            "mac-agent".to_string(),
            None,
            activities,
            now,
            AnalysisRange::All,
            &[],
            &crate::governance::default_work_schedule(),
        );

        assert_eq!(analysis.current_label.as_deref(), Some("Latest page"));
    }
}
