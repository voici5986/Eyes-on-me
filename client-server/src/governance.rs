use std::collections::HashMap;

use anyhow::{bail, ensure};
use eyes_on_me_shared::{
    ActivityEvent, CategoryRule, CategoryTarget, DeletionSummary, PotentialTodo, ReviewSettings,
    TimelineItem, TimelineResponse, WorkScheduleSegment, WorkSession, WorkSessionResponse,
};
use time::{Duration, OffsetDateTime, UtcOffset};
use uuid::Uuid;

use crate::app_state::AppState;

const MAX_ACTIVITY_CONTINUITY: Duration = Duration::seconds(120);
const MAX_DELETE_BATCH: usize = 100_000;

#[derive(Debug, Clone)]
pub struct TimelineFilter {
    pub start: OffsetDateTime,
    pub end: OffsetDateTime,
    pub device_id: Option<String>,
    pub event_id: Option<String>,
    pub app: Option<String>,
    pub domain: Option<String>,
    pub category: Option<String>,
    pub query: Option<String>,
    pub limit: usize,
    pub offset: usize,
}

impl TimelineFilter {
    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(self.end > self.start, "end must be after start");
        ensure!(
            self.end - self.start <= Duration::days(3660),
            "timeline range is too large"
        );
        ensure!(
            self.limit > 0 && self.limit <= 500,
            "invalid timeline limit"
        );
        Ok(())
    }
}

pub async fn load_timeline(
    state: &AppState,
    filter: TimelineFilter,
) -> anyhow::Result<TimelineResponse> {
    filter.validate()?;
    let settings = effective_settings(&state.pool()).await?;
    let mut activities = matching_activities(state, &filter, &settings.category_rules).await?;
    let durations = activity_durations(&activities, filter.end);
    activities.reverse();
    let total = activities.len();
    let page = activities
        .into_iter()
        .skip(filter.offset)
        .take(filter.limit)
        .collect::<Vec<_>>();
    let event_ids = page
        .iter()
        .map(|activity| activity.event_id.clone())
        .collect::<Vec<_>>();
    let screenshots = crate::db::load_screenshots_for_event_ids(&state.pool(), &event_ids)
        .await?
        .into_iter()
        .map(|item| (item.event_id.clone(), item))
        .collect::<HashMap<_, _>>();
    let items = page
        .into_iter()
        .map(|activity| {
            let (category_key, category_label) =
                classify_activity(&activity, &settings.category_rules);
            TimelineItem {
                duration_ms: durations
                    .get(&activity.event_id)
                    .copied()
                    .unwrap_or_default(),
                screenshot: screenshots.get(&activity.event_id).cloned(),
                category_key,
                category_label,
                activity,
            }
        })
        .collect();
    Ok(TimelineResponse {
        items,
        total,
        limit: filter.limit,
        offset: filter.offset,
        start: filter.start.to_string(),
        end: filter.end.to_string(),
    })
}

pub async fn delete_matching_activities(
    state: &AppState,
    filter: TimelineFilter,
) -> anyhow::Result<DeletionSummary> {
    filter.validate()?;
    let settings = effective_settings(&state.pool()).await?;
    let activities = matching_activities(state, &filter, &settings.category_rules).await?;
    ensure!(
        activities.len() <= MAX_DELETE_BATCH,
        "delete selection exceeds the safety limit"
    );
    for activity in &activities {
        if let Some(stored) =
            crate::db::load_screenshot_for_event(&state.pool(), &activity.event_id).await?
            && let Some(remote) =
                crate::db::load_remote_mirror_record(&state.pool(), &stored.record.id).await?
        {
            crate::remote::delete_remote_key(state, &remote.remote_key).await?;
        }
    }
    let deleted = crate::db::delete_activity_rows(&state.pool(), &activities).await?;
    let deleted_screenshots = deleted.screenshots.len() as u64;
    for screenshot in deleted.screenshots {
        crate::media::delete_stored_files(state.media_dir().to_path_buf(), screenshot).await;
    }
    state.reload_snapshot().await?;
    Ok(DeletionSummary {
        deleted_activities: deleted.deleted_activities,
        deleted_screenshots,
        affected_dates: deleted.affected_dates,
    })
}

async fn matching_activities(
    state: &AppState,
    filter: &TimelineFilter,
    rules: &[CategoryRule],
) -> anyhow::Result<Vec<ActivityEvent>> {
    let app_filter = normalized_filter(filter.app.as_deref());
    let domain_filter = normalized_filter(filter.domain.as_deref());
    let category_filter = normalized_filter(filter.category.as_deref());
    let query_filter = normalized_filter(filter.query.as_deref());
    let activities = crate::db::load_timeline_activities(
        &state.pool(),
        filter.start,
        filter.end,
        filter.device_id.as_deref(),
    )
    .await?;
    Ok(activities
        .into_iter()
        .filter(|activity| {
            filter
                .event_id
                .as_ref()
                .is_none_or(|event_id| activity.event_id == *event_id)
        })
        .filter(|activity| {
            app_filter.as_ref().is_none_or(|needle| {
                activity.app.id.to_ascii_lowercase().contains(needle)
                    || activity.app.name.to_ascii_lowercase().contains(needle)
            })
        })
        .filter(|activity| {
            domain_filter.as_ref().is_none_or(|needle| {
                activity
                    .browser
                    .as_ref()
                    .and_then(|browser| browser.domain.as_deref())
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .contains(needle)
            })
        })
        .filter(|activity| {
            category_filter.as_ref().is_none_or(|needle| {
                let (key, label) = classify_activity(activity, rules);
                key.to_ascii_lowercase().contains(needle)
                    || label.to_ascii_lowercase().contains(needle)
            })
        })
        .filter(|activity| {
            query_filter
                .as_ref()
                .is_none_or(|needle| searchable_activity_text(activity).contains(needle))
        })
        .collect())
}

fn normalized_filter(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
}

fn searchable_activity_text(activity: &ActivityEvent) -> String {
    let browser = activity.browser.as_ref();
    [
        Some(activity.app.id.as_str()),
        Some(activity.app.name.as_str()),
        activity.window_title.as_deref(),
        activity.app.title.as_deref(),
        browser.and_then(|item| item.page_title.as_deref()),
        browser.and_then(|item| item.url.as_deref()),
        browser.and_then(|item| item.domain.as_deref()),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ")
    .to_ascii_lowercase()
}

fn activity_durations(
    activities: &[ActivityEvent],
    range_end: OffsetDateTime,
) -> HashMap<String, u64> {
    let mut next_by_device = HashMap::<&str, OffsetDateTime>::new();
    let mut durations = HashMap::new();
    for activity in activities.iter().rev() {
        let next = next_by_device
            .get(activity.device_id.as_str())
            .copied()
            .unwrap_or(range_end)
            .min(activity.ts + MAX_ACTIVITY_CONTINUITY);
        let duration_ms = (next - activity.ts)
            .whole_milliseconds()
            .max(0)
            .try_into()
            .unwrap_or_default();
        durations.insert(activity.event_id.clone(), duration_ms);
        next_by_device.insert(activity.device_id.as_str(), activity.ts);
    }
    durations
}

pub async fn effective_settings(pool: &sqlx::SqlitePool) -> anyhow::Result<ReviewSettings> {
    let mut settings = crate::db::load_review_settings(pool).await?;
    if settings.work_schedule.is_empty() {
        settings.work_schedule = default_work_schedule();
    }
    Ok(settings)
}

pub async fn save_settings(
    pool: &sqlx::SqlitePool,
    mut settings: ReviewSettings,
) -> anyhow::Result<ReviewSettings> {
    normalize_and_validate_settings(&mut settings)?;
    crate::db::save_review_settings(pool, &settings).await?;
    Ok(settings)
}

fn normalize_and_validate_settings(settings: &mut ReviewSettings) -> anyhow::Result<()> {
    let mut ids = std::collections::HashSet::new();
    for rule in &mut settings.category_rules {
        rule.id = rule.id.trim().to_string();
        if rule.id.is_empty() {
            rule.id = Uuid::new_v4().to_string();
        }
        rule.name = rule.name.trim().to_string();
        rule.pattern = rule.pattern.trim().to_ascii_lowercase();
        rule.color = rule.color.trim().to_string();
        ensure!(!rule.name.is_empty(), "category name is required");
        ensure!(!rule.pattern.is_empty(), "category pattern is required");
        ensure!(ids.insert(rule.id.clone()), "duplicate category id");
        ensure!(
            valid_color(&rule.color),
            "category color must be a hex color"
        );
    }
    settings
        .category_rules
        .sort_by(|left, right| right.priority.cmp(&left.priority));
    ids.clear();
    for segment in &mut settings.work_schedule {
        segment.id = segment.id.trim().to_string();
        if segment.id.is_empty() {
            segment.id = Uuid::new_v4().to_string();
        }
        ensure!(ids.insert(segment.id.clone()), "duplicate schedule id");
        ensure!(
            (1..=7).contains(&segment.weekday),
            "weekday must be 1 through 7"
        );
        ensure!(
            segment.start_minute < segment.end_minute && segment.end_minute <= 1440,
            "invalid working-hours segment"
        );
    }
    settings
        .work_schedule
        .sort_by_key(|item| (item.weekday, item.start_minute));
    for pair in settings.work_schedule.windows(2) {
        if pair[0].weekday == pair[1].weekday && pair[0].end_minute > pair[1].start_minute {
            bail!("working-hours segments cannot overlap");
        }
    }
    let valid_blocks = ["overview", "applications", "websites", "screenshots"];
    settings
        .report_preferences
        .pinned_blocks
        .retain(|value| valid_blocks.contains(&value.as_str()));
    settings
        .report_preferences
        .hidden_blocks
        .retain(|value| valid_blocks.contains(&value.as_str()));
    settings
        .report_preferences
        .block_order
        .retain(|value| valid_blocks.contains(&value.as_str()));
    for block in valid_blocks {
        if !settings
            .report_preferences
            .block_order
            .iter()
            .any(|value| value == block)
        {
            settings
                .report_preferences
                .block_order
                .push(block.to_string());
        }
    }
    if !settings.report_preferences.auto_export_enabled {
        settings.report_preferences.auto_export_directory = None;
    }
    Ok(())
}

fn valid_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

pub fn default_work_schedule() -> Vec<WorkScheduleSegment> {
    (1..=5)
        .map(|weekday| WorkScheduleSegment {
            id: format!("weekday-{weekday}"),
            weekday,
            start_minute: 9 * 60,
            end_minute: 18 * 60,
        })
        .collect()
}

pub fn classify_activity(activity: &ActivityEvent, rules: &[CategoryRule]) -> (String, String) {
    for rule in rules {
        let candidate = match rule.target {
            CategoryTarget::App => format!("{} {}", activity.app.name, activity.app.id),
            CategoryTarget::Domain => activity
                .browser
                .as_ref()
                .and_then(|browser| browser.domain.clone())
                .unwrap_or_default(),
        };
        if pattern_matches(&candidate, &rule.pattern) {
            return (rule.id.clone(), rule.name.clone());
        }
    }
    let (key, label) = built_in_category(activity);
    (key.to_string(), label.to_string())
}

fn pattern_matches(candidate: &str, pattern: &str) -> bool {
    let candidate = candidate.to_ascii_lowercase();
    let pattern = pattern.trim().to_ascii_lowercase();
    if pattern.is_empty() {
        return false;
    }
    if !pattern.contains('*') {
        return candidate.contains(&pattern);
    }
    let mut remaining = candidate.as_str();
    for token in pattern.split('*').filter(|token| !token.is_empty()) {
        let Some(index) = remaining.find(token) else {
            return false;
        };
        remaining = &remaining[index + token.len()..];
    }
    true
}

fn built_in_category(activity: &ActivityEvent) -> (&'static str, &'static str) {
    if activity.browser.is_some() {
        return ("browser", "网页浏览");
    }
    let identity = format!("{} {}", activity.app.name, activity.app.id).to_ascii_lowercase();
    let contains_any = |candidates: &[&str]| candidates.iter().any(|item| identity.contains(item));
    if contains_any(&[
        "terminal",
        "iterm",
        "warp",
        "wezterm",
        "alacritty",
        "kitty",
        "powershell",
        "cmd.exe",
        "code",
        "cursor",
        "xcode",
        "intellij",
        "idea",
        "pycharm",
        "webstorm",
        "goland",
        "clion",
        "rider",
        "zed",
        "vim",
        "nvim",
        "emacs",
        "docker",
        "postman",
        "tableplus",
        "navicat",
    ]) {
        return ("development", "开发工具");
    }
    if contains_any(&[
        "slack", "teams", "discord", "zoom", "wechat", "wecom", "lark", "feishu", "ding", "mail",
        "outlook", "telegram",
    ]) {
        return ("communication", "沟通协作");
    }
    if contains_any(&[
        "figma",
        "sketch",
        "photoshop",
        "illustrator",
        "affinity",
        "blender",
        "cinema 4d",
        "premiere",
        "davinci",
        "final cut",
    ]) {
        return ("creative", "设计创作");
    }
    if contains_any(&[
        "notion",
        "obsidian",
        "word",
        "excel",
        "powerpoint",
        "pages",
        "numbers",
        "keynote",
        "preview",
        "acrobat",
    ]) {
        return ("productivity", "文档效率");
    }
    ("other", "其他")
}

pub fn duration_within_schedule(
    start: OffsetDateTime,
    end: OffsetDateTime,
    local_offset: UtcOffset,
    schedule: &[WorkScheduleSegment],
) -> u64 {
    let start_local = start.to_offset(local_offset);
    let end_local = end.to_offset(local_offset);
    let mut date = start_local.date();
    let mut total = 0_u64;
    while date <= end_local.date() {
        let weekday = date.weekday().number_from_monday();
        for segment in schedule.iter().filter(|item| item.weekday == weekday) {
            let midnight = date.midnight().assume_offset(local_offset);
            let segment_start = midnight + Duration::minutes(i64::from(segment.start_minute));
            let segment_end = midnight + Duration::minutes(i64::from(segment.end_minute));
            let overlap_start = start_local.max(segment_start);
            let overlap_end = end_local.min(segment_end);
            if overlap_end > overlap_start {
                total = total.saturating_add(
                    (overlap_end - overlap_start)
                        .whole_milliseconds()
                        .max(0)
                        .try_into()
                        .unwrap_or_default(),
                );
            }
        }
        let Some(next) = date.next_day() else {
            break;
        };
        date = next;
    }
    total
}

pub async fn load_work_sessions(
    state: &AppState,
    start: OffsetDateTime,
    end: OffsetDateTime,
    device_id: Option<&str>,
) -> anyhow::Result<WorkSessionResponse> {
    ensure!(end > start, "end must be after start");
    let activities = crate::db::load_timeline_activities(&state.pool(), start, end, device_id)
        .await?
        .into_iter()
        .filter(|item| item.presence == eyes_on_me_shared::PresenceState::Active)
        .collect::<Vec<_>>();
    let screenshots = crate::db::load_screenshots_between(&state.pool(), start, end).await?;
    let ocr_by_event = screenshots
        .into_iter()
        .filter_map(|item| item.ocr_text.map(|text| (item.event_id, text)))
        .collect::<HashMap<_, _>>();
    let mut by_device = HashMap::<String, Vec<ActivityEvent>>::new();
    for activity in activities {
        by_device
            .entry(activity.device_id.clone())
            .or_default()
            .push(activity);
    }
    let mut sessions = Vec::new();
    for (device_id, mut activities) in by_device {
        activities.sort_by_key(|item| item.ts);
        let mut current = Vec::<ActivityEvent>::new();
        for activity in activities {
            let starts_new = current
                .last()
                .is_some_and(|last| activity.ts - last.ts > Duration::minutes(5));
            if starts_new {
                sessions.push(build_work_session(&device_id, &current, &ocr_by_event, end));
                current.clear();
            }
            current.push(activity);
        }
        if !current.is_empty() {
            sessions.push(build_work_session(&device_id, &current, &ocr_by_event, end));
        }
    }
    sessions.sort_by(|left, right| right.started_at.cmp(&left.started_at));
    Ok(WorkSessionResponse { sessions })
}

fn build_work_session(
    device_id: &str,
    activities: &[ActivityEvent],
    ocr_by_event: &HashMap<String, String>,
    range_end: OffsetDateTime,
) -> WorkSession {
    let started_at = activities.first().map(|item| item.ts).unwrap_or(range_end);
    let ended_at = activities
        .last()
        .map(|item| (item.ts + MAX_ACTIVITY_CONTINUITY).min(range_end))
        .unwrap_or(started_at);
    let durations = activity_durations(activities, range_end);
    let total_tracked_ms = durations.values().copied().sum();
    let mut app_counts = HashMap::<String, u32>::new();
    let mut todos = Vec::new();
    for activity in activities {
        *app_counts.entry(activity.app.name.clone()).or_default() += 1;
        for text in [
            activity.window_title.as_deref(),
            activity.app.title.as_deref(),
            activity
                .browser
                .as_ref()
                .and_then(|browser| browser.page_title.as_deref()),
            ocr_by_event.get(&activity.event_id).map(String::as_str),
        ]
        .into_iter()
        .flatten()
        {
            for line in text
                .lines()
                .map(str::trim)
                .filter(|line| potential_todo(line))
            {
                if !todos.iter().any(|item: &PotentialTodo| item.text == line) {
                    todos.push(PotentialTodo {
                        text: line.chars().take(240).collect(),
                        source_event_id: activity.event_id.clone(),
                        observed_at: activity.ts,
                    });
                }
            }
        }
    }
    let mut ranked = app_counts.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let app_names = ranked.into_iter().map(|item| item.0).collect::<Vec<_>>();
    let summary = if app_names.is_empty() {
        "无可归属活动".to_string()
    } else {
        format!(
            "主要使用 {}",
            app_names
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join("、")
        )
    };
    WorkSession {
        id: format!("{}-{}", device_id, started_at.unix_timestamp_nanos()),
        device_id: device_id.to_string(),
        started_at,
        ended_at,
        total_tracked_ms,
        app_names,
        summary,
        potential_todos: todos.into_iter().take(20).collect(),
    }
}

fn potential_todo(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    [
        "todo",
        "fixme",
        "follow up",
        "action item",
        "待办",
        "记得",
        "下一步",
    ]
    .iter()
    .any(|needle| value.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::{default_work_schedule, pattern_matches};

    #[test]
    fn wildcard_rules_match_in_order() {
        assert!(pattern_matches("com.apple.Safari", "*safari"));
        assert!(pattern_matches("github.com", "git*com"));
        assert!(!pattern_matches("example.com", "git*com"));
    }

    #[test]
    fn default_schedule_has_five_weekdays() {
        assert_eq!(default_work_schedule().len(), 5);
    }
}
