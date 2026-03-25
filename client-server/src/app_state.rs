use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, RwLock},
};

use amiokay_shared::{
    ActivityEvent, AnalysisOverviewResponse, DashboardSnapshot, DeviceAnalysisResponse,
    DeviceAnalysisSummary, DeviceDetailResponse, DeviceOverview, DeviceStatus, DevicesResponse,
    StreamMessage, UsageBucket,
};
use sqlx::SqlitePool;
use tokio::sync::broadcast;
use time::{Duration as TimeDuration, OffsetDateTime};

const RECENT_ACTIVITY_LIMIT: usize = 20;
const DEVICE_ACTIVITY_LIMIT: i64 = 50;
const ANALYSIS_TOP_LIMIT: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisRange {
    Last3Hours,
    Last6Hours,
    LastDay,
    LastWeek,
    LastMonth,
    All,
}

impl AnalysisRange {
    pub fn from_query(value: Option<&str>) -> Option<Self> {
        match value.unwrap_or("1d") {
            "3h" => Some(Self::Last3Hours),
            "6h" => Some(Self::Last6Hours),
            "1d" => Some(Self::LastDay),
            "1w" => Some(Self::LastWeek),
            "1m" => Some(Self::LastMonth),
            "all" => Some(Self::All),
            _ => None,
        }
    }

    fn window_start(self, now: OffsetDateTime) -> Option<OffsetDateTime> {
        match self {
            Self::Last3Hours => Some(now - TimeDuration::hours(3)),
            Self::Last6Hours => Some(now - TimeDuration::hours(6)),
            Self::LastDay => Some(now - TimeDuration::days(1)),
            Self::LastWeek => Some(now - TimeDuration::days(7)),
            Self::LastMonth => Some(now - TimeDuration::days(30)),
            Self::All => None,
        }
    }
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
}

impl AppState {
    pub fn new(snapshot: DashboardSnapshot, pool: SqlitePool) -> Self {
        let (tx, _) = broadcast::channel(128);
        Self {
            inner: Arc::new(RwLock::new(SnapshotState { snapshot })),
            pool,
            tx,
        }
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

        let devices = snapshot
            .devices
            .into_iter()
            .map(|device| DeviceOverview {
                latest_status: status_by_device.get(&device.device_id).cloned(),
                device,
            })
            .collect();

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
        let recent_activities =
            crate::db::load_recent_activities_for_device(&self.pool, device_id, DEVICE_ACTIVITY_LIMIT)
                .await?;

        Ok(Some(DeviceDetailResponse {
            device,
            latest_status,
            recent_activities,
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

        let mut devices = Vec::new();
        let mut app_usage = HashMap::new();
        let mut domain_usage = HashMap::new();
        let mut total_tracked_ms = 0_u64;

        for current_device in snapshot.devices {
            let activities =
                crate::db::load_all_activities_for_device(&self.pool, &current_device.device_id).await?;
            if activities.is_empty() {
                continue;
            }

            let analysis = build_device_analysis_payload(
                current_device.device_id.clone(),
                latest_statuses.get(&current_device.device_id).cloned(),
                activities,
                now,
                range,
            );

            if analysis.total_tracked_ms == 0 {
                continue;
            }

            total_tracked_ms += analysis.total_tracked_ms;
            merge_usage_vec(&mut app_usage, &analysis.app_usage);
            merge_usage_vec(&mut domain_usage, &analysis.domain_usage);

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
            devices,
            top_app_usage: finalize_usage_map(app_usage),
            top_domain_usage: finalize_usage_map(domain_usage),
        })
    }

    pub async fn device_analysis(
        &self,
        device_id: &str,
        range: AnalysisRange,
    ) -> anyhow::Result<Option<DeviceAnalysisResponse>> {
        let activities = crate::db::load_all_activities_for_device(&self.pool, device_id).await?;
        if activities.is_empty() {
            return Ok(None);
        }

        let latest_status = crate::db::load_device_status(&self.pool, device_id).await?;
        Ok(Some(build_device_analysis_payload(
            device_id.to_string(),
            latest_status,
            activities,
            OffsetDateTime::now_utc(),
            range,
        )))
    }

    fn broadcast_snapshot(&self, snapshot: DashboardSnapshot) {
        let _ = self.tx.send(StreamMessage::Snapshot(snapshot));
    }
}

fn upsert_device(devices: &[ActivityEvent], event: &ActivityEvent) -> Vec<ActivityEvent> {
    let mut next = devices.to_vec();

    if let Some(existing) = next.iter_mut().find(|item| item.device_id == event.device_id) {
        *existing = event.clone();
    } else {
        next.push(event.clone());
    }

    next.sort_by(|a, b| b.ts.cmp(&a.ts));
    next
}

fn push_recent_activity(current: &[ActivityEvent], event: ActivityEvent) -> Vec<ActivityEvent> {
    let mut queue: VecDeque<ActivityEvent> = current.iter().cloned().collect();
    queue.push_front(event);

    while queue.len() > RECENT_ACTIVITY_LIMIT {
        queue.pop_back();
    }

    queue.into_iter().collect()
}

fn derive_status_from_activity(event: &ActivityEvent) -> DeviceStatus {
    let status_text = if let Some(browser) = &event.browser {
        if let Some(domain) = &browser.domain {
            format!("正在使用 {} 浏览 {}", event.app.name, domain)
        } else if let Some(page_title) = &browser.page_title {
            format!("正在使用 {} 查看 {}", event.app.name, page_title)
        } else {
            format!("正在使用 {}", event.app.name)
        }
    } else if let Some(window_title) = &event.window_title {
        format!("正在使用 {} · {}", event.app.name, window_title)
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
) -> DeviceAnalysisResponse {
    let mut app_usage = HashMap::new();
    let mut domain_usage = HashMap::new();
    let mut total_tracked_ms = 0_u64;
    let mut event_count = 0_usize;
    let mut current_label = None;
    let window_start = range.window_start(now);

    for (index, activity) in activities.iter().enumerate() {
        let next_ts = activities.get(index + 1).map(|next| next.ts).unwrap_or(now);
        let tracked_ms = duration_within_window(activity.ts, next_ts, window_start, now);
        if tracked_ms == 0 {
            continue;
        }

        event_count += 1;
        current_label = Some(current_activity_label(activity));
        total_tracked_ms += tracked_ms;
        accumulate_app_usage(&mut app_usage, activity, tracked_ms);
        accumulate_domain_usage(&mut domain_usage, activity, tracked_ms);
    }

    if current_label.is_none() {
        current_label = activities.last().map(current_activity_label);
    }

    DeviceAnalysisResponse {
        device_id,
        generated_at: now,
        total_tracked_ms,
        event_count,
        current_label,
        latest_status,
        app_usage: finalize_usage_map(app_usage),
        domain_usage: finalize_usage_map(domain_usage),
    }
}

fn duration_within_window(
    start: OffsetDateTime,
    end: OffsetDateTime,
    window_start: Option<OffsetDateTime>,
    now: OffsetDateTime,
) -> u64 {
    let effective_start = match window_start {
        Some(cutoff) if start < cutoff => cutoff,
        _ => start,
    };
    let effective_end = if end > now { now } else { end };

    if effective_end <= effective_start {
        return 0;
    }

    (effective_end - effective_start)
        .whole_milliseconds()
        .max(0)
        .try_into()
        .unwrap_or(0)
}

fn accumulate_app_usage(
    target: &mut HashMap<String, UsageBucket>,
    activity: &ActivityEvent,
    tracked_ms: u64,
) {
    let is_browser_activity = activity.browser.is_some();
    let label = if is_browser_activity {
        activity.app.name.clone()
    } else {
        activity
            .window_title
            .clone()
            .or_else(|| activity.app.title.clone())
            .unwrap_or_else(|| activity.app.name.clone())
    };

    let sublabel = if is_browser_activity || label == activity.app.name {
        Some(activity.app.id.clone())
    } else {
        Some(activity.app.name.clone())
    };

    let key = if is_browser_activity {
        format!("app:{}", activity.app.id)
    } else {
        format!("app:{}:{}", activity.app.id, label.to_lowercase())
    };

    merge_usage_entry(
        target,
        UsageBucket {
            key,
            label,
            sublabel,
            total_tracked_ms: tracked_ms,
            sessions: 1,
            last_seen: activity.ts,
        },
    );
}

fn accumulate_domain_usage(
    target: &mut HashMap<String, UsageBucket>,
    activity: &ActivityEvent,
    tracked_ms: u64,
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
            sessions: 1,
            last_seen: activity.ts,
        },
    );
}

fn merge_usage_vec(target: &mut HashMap<String, UsageBucket>, source: &[UsageBucket]) {
    for bucket in source {
        merge_usage_entry(target, bucket.clone());
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

fn finalize_usage_map(map: HashMap<String, UsageBucket>) -> Vec<UsageBucket> {
    let mut items = map.into_values().collect::<Vec<_>>();
    items.sort_by(|a, b| {
        b.total_tracked_ms
            .cmp(&a.total_tracked_ms)
            .then_with(|| b.last_seen.cmp(&a.last_seen))
    });
    items.truncate(ANALYSIS_TOP_LIMIT);
    items
}

fn current_activity_label(activity: &ActivityEvent) -> String {
    activity
        .browser
        .as_ref()
        .and_then(|browser| browser.page_title.clone())
        .or_else(|| activity.window_title.clone())
        .or_else(|| activity.app.title.clone())
        .unwrap_or_else(|| activity.app.name.clone())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use amiokay_shared::{ActivityApp, ActivityEvent, ActivityKind, BrowserContext, Platform};

    use super::{AnalysisRange, accumulate_app_usage, duration_within_window};
    use time::{Duration as TimeDuration, OffsetDateTime};

    #[test]
    fn parses_expected_analysis_ranges() {
        assert_eq!(AnalysisRange::from_query(None), Some(AnalysisRange::LastDay));
        assert_eq!(AnalysisRange::from_query(Some("3h")), Some(AnalysisRange::Last3Hours));
        assert_eq!(AnalysisRange::from_query(Some("6h")), Some(AnalysisRange::Last6Hours));
        assert_eq!(AnalysisRange::from_query(Some("1d")), Some(AnalysisRange::LastDay));
        assert_eq!(AnalysisRange::from_query(Some("1w")), Some(AnalysisRange::LastWeek));
        assert_eq!(AnalysisRange::from_query(Some("1m")), Some(AnalysisRange::LastMonth));
        assert_eq!(AnalysisRange::from_query(Some("all")), Some(AnalysisRange::All));
        assert_eq!(AnalysisRange::from_query(Some("bogus")), None);
    }

    #[test]
    fn truncates_activity_duration_to_window_start() {
        let now = OffsetDateTime::now_utc();
        let start = now - TimeDuration::hours(4);
        let end = now - TimeDuration::hours(1);
        let cutoff = Some(now - TimeDuration::hours(3));

        let tracked_ms = duration_within_window(start, end, cutoff, now);

        assert_eq!(tracked_ms, TimeDuration::hours(2).whole_milliseconds() as u64);
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
            source: "desktop".to_string(),
        };

        let mut usage = HashMap::new();
        accumulate_app_usage(&mut usage, &activity, 60_000);

        let bucket = usage.get("app:msedge.exe").expect("browser app bucket");
        assert_eq!(bucket.label, "Microsoft Edge");
        assert_eq!(bucket.sublabel.as_deref(), Some("msedge.exe"));
    }
}
