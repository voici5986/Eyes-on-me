use std::collections::HashMap;

use anyhow::{Context, bail};
use eyes_on_me_shared::{
    ActivityEvent, DailyReport, PresenceState, ReportPreferences, ScreenshotRecord,
};
use serde::Deserialize;
use time::{
    Date, Duration, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset, format_description,
};

use crate::app_state::AppState;

const MAX_INTERVAL_SECONDS: i64 = 120;

pub async fn generate(state: &AppState, date: &str, use_ai: bool) -> anyhow::Result<DailyReport> {
    let (start, end) = date_bounds(date)?;
    let activities = crate::db::load_activities_between(&state.pool(), start, end).await?;
    let screenshots = crate::db::load_screenshots_between(&state.pool(), start, end).await?;
    let preferences = crate::governance::effective_settings(&state.pool())
        .await?
        .report_preferences;
    let deterministic = deterministic_markdown(date, &activities, &screenshots, &preferences);
    let now = OffsetDateTime::now_utc();

    let mut report = DailyReport {
        date: date.to_string(),
        content: deterministic.clone(),
        generation_mode: "deterministic".to_string(),
        model_name: None,
        fallback_reason: None,
        created_at: now,
        updated_at: now,
    };

    if use_ai {
        match polish_with_ai(state, &deterministic).await {
            Ok((content, model)) => {
                report.content = apply_markdown_preferences(date, &content, &preferences);
                report.generation_mode = "ai_polished".to_string();
                report.model_name = Some(model);
            }
            Err(err) => report.fallback_reason = Some(err.to_string()),
        }
    }

    if let Some(existing) = crate::db::load_daily_report(&state.pool(), date).await? {
        report.created_at = existing.created_at;
    }
    crate::db::save_daily_report(&state.pool(), &report).await?;
    maybe_auto_export(&report, &preferences).await?;
    Ok(report)
}

pub async fn save_manual(
    state: &AppState,
    date: &str,
    content: String,
) -> anyhow::Result<DailyReport> {
    date_bounds(date)?;
    if content.trim().is_empty() {
        bail!("report content cannot be empty");
    }
    let now = OffsetDateTime::now_utc();
    let existing = crate::db::load_daily_report(&state.pool(), date).await?;
    let report = DailyReport {
        date: date.to_string(),
        content,
        generation_mode: "manual".to_string(),
        model_name: None,
        fallback_reason: None,
        created_at: existing.map(|item| item.created_at).unwrap_or(now),
        updated_at: now,
    };
    crate::db::save_daily_report(&state.pool(), &report).await?;
    let preferences = crate::governance::effective_settings(&state.pool())
        .await?
        .report_preferences;
    maybe_auto_export(&report, &preferences).await?;
    Ok(report)
}

pub fn date_bounds(date: &str) -> anyhow::Result<(OffsetDateTime, OffsetDateTime)> {
    let format = format_description::parse("[year]-[month]-[day]")?;
    let date = Date::parse(date, &format).context("date must use YYYY-MM-DD")?;
    let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    let start = PrimitiveDateTime::new(date, Time::MIDNIGHT).assume_offset(offset);
    Ok((
        start.to_offset(UtcOffset::UTC),
        (start + Duration::days(1)).to_offset(UtcOffset::UTC),
    ))
}

fn deterministic_markdown(
    date: &str,
    activities: &[ActivityEvent],
    screenshots: &[ScreenshotRecord],
    preferences: &ReportPreferences,
) -> String {
    let mut app_ms: HashMap<String, u64> = HashMap::new();
    let mut domain_ms: HashMap<String, u64> = HashMap::new();
    let mut tracked_ms = 0_u64;

    for (index, current) in activities.iter().enumerate() {
        if current.presence != PresenceState::Active {
            continue;
        }
        let segment_end = activities
            .get(index + 1)
            .map(|next| next.ts)
            .unwrap_or_else(|| OffsetDateTime::now_utc().min(end_for_date(date)));
        let seconds = (segment_end - current.ts)
            .whole_seconds()
            .clamp(0, MAX_INTERVAL_SECONDS);
        let duration = u64::try_from(seconds).unwrap_or_default() * 1_000;
        if duration == 0 {
            continue;
        }
        tracked_ms += duration;
        *app_ms.entry(current.app.name.clone()).or_default() += duration;
        if let Some(domain) = current
            .browser
            .as_ref()
            .and_then(|value| value.domain.clone())
        {
            *domain_ms.entry(domain).or_default() += duration;
        }
    }
    let active_events = activities
        .iter()
        .filter(|item| item.presence == PresenceState::Active)
        .count();

    let top_apps = ranked_lines(app_ms, tracked_ms, 8);
    let top_domains = ranked_lines(domain_ms, tracked_ms, 6);
    let ocr_notes = screenshots
        .iter()
        .filter_map(|item| item.ocr_text.as_deref())
        .map(collapse_text)
        .filter(|item| !item.is_empty())
        .take(8)
        .map(|item| format!("- {}", truncate_chars(&item, 180)))
        .collect::<Vec<_>>();

    let blocks = HashMap::from([
        (
            "overview".to_string(),
            format!(
                "## 概览\n\n- 可归属活动时长：{}\n- 活动事件：{active_events}\n- 截图：{} 张，其中 {} 张已有 OCR",
                format_duration(tracked_ms),
                screenshots.len(),
                screenshots
                    .iter()
                    .filter(|item| item.ocr_text.is_some())
                    .count(),
            ),
        ),
        (
            "applications".to_string(),
            format!("## 应用使用\n\n{}", list_or_empty(top_apps)),
        ),
        (
            "websites".to_string(),
            format!("## 浏览站点\n\n{}", list_or_empty(top_domains)),
        ),
        (
            "screenshots".to_string(),
            format!("## 截图线索\n\n{}", list_or_empty(ocr_notes)),
        ),
    ]);
    render_report_blocks(date, blocks, preferences)
}

fn render_report_blocks(
    date: &str,
    mut blocks: HashMap<String, String>,
    preferences: &ReportPreferences,
) -> String {
    let mut order = Vec::new();
    for id in preferences
        .pinned_blocks
        .iter()
        .chain(preferences.block_order.iter())
    {
        if !preferences.hidden_blocks.contains(id) && !order.contains(id) && blocks.contains_key(id)
        {
            order.push(id.clone());
        }
    }
    for id in ["overview", "applications", "websites", "screenshots"] {
        let id = id.to_string();
        if !preferences.hidden_blocks.contains(&id)
            && !order.contains(&id)
            && blocks.contains_key(&id)
        {
            order.push(id);
        }
    }
    let body = order
        .into_iter()
        .filter_map(|id| blocks.remove(&id))
        .collect::<Vec<_>>()
        .join("\n\n");
    format!("# {date} 活动日报\n\n{body}\n")
}

fn apply_markdown_preferences(
    date: &str,
    markdown: &str,
    preferences: &ReportPreferences,
) -> String {
    let mut blocks = HashMap::new();
    let mut current_id = None::<String>;
    let mut current = Vec::new();
    let flush =
        |id: &mut Option<String>, lines: &mut Vec<&str>, blocks: &mut HashMap<String, String>| {
            if let Some(id) = id.take() {
                blocks.insert(id, lines.join("\n").trim().to_string());
            }
            lines.clear();
        };
    for line in markdown.lines() {
        let next_id = match line.trim() {
            "## 概览" => Some("overview"),
            "## 应用使用" => Some("applications"),
            "## 浏览站点" => Some("websites"),
            "## 截图线索" => Some("screenshots"),
            _ => None,
        };
        if let Some(id) = next_id {
            flush(&mut current_id, &mut current, &mut blocks);
            current_id = Some(id.to_string());
            current.push(line);
        } else if current_id.is_some() {
            current.push(line);
        }
    }
    flush(&mut current_id, &mut current, &mut blocks);
    if blocks.is_empty() {
        return markdown.to_string();
    }
    render_report_blocks(date, blocks, preferences)
}

async fn maybe_auto_export(
    report: &DailyReport,
    preferences: &ReportPreferences,
) -> anyhow::Result<()> {
    if !preferences.auto_export_enabled {
        return Ok(());
    }
    let directory = preferences
        .auto_export_directory
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("auto-export directory is required when auto export is enabled")?;
    let directory = std::path::Path::new(directory);
    tokio::fs::create_dir_all(directory).await?;
    let destination = directory.join(format!("eyes-on-me-{}.md", report.date));
    let temporary = destination.with_extension("md.tmp");
    tokio::fs::write(&temporary, report.content.as_bytes()).await?;
    tokio::fs::rename(temporary, destination).await?;
    Ok(())
}

fn ranked_lines(values: HashMap<String, u64>, total: u64, limit: usize) -> Vec<String> {
    let mut values = values.into_iter().collect::<Vec<_>>();
    values.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    values
        .into_iter()
        .take(limit)
        .map(|(label, duration)| {
            let share = if total == 0 {
                0.0
            } else {
                duration as f64 / total as f64 * 100.0
            };
            format!("- {label}：{}（{share:.1}%）", format_duration(duration))
        })
        .collect()
}

async fn polish_with_ai(state: &AppState, source: &str) -> anyhow::Result<(String, String)> {
    let config = state
        .ai_config()
        .context("AI report generation is not configured")?;
    let model = config
        .chat_model
        .as_deref()
        .context("AI chat model is not configured")?;
    #[derive(Deserialize)]
    struct ChatMessage {
        content: String,
    }
    #[derive(Deserialize)]
    struct ChatChoice {
        message: ChatMessage,
    }
    #[derive(Deserialize)]
    struct ChatResponse {
        choices: Vec<ChatChoice>,
    }

    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(45))
        .build()?
        .post(format!("{}/chat/completions", config.base_url))
        .bearer_auth(&config.api_key)
        .json(&serde_json::json!({
            "model": model,
            "temperature": 0.2,
            "messages": [
                {"role": "system", "content": "你是活动日报编辑。只重写用户提供的事实，不推断、不补造事项，保留 Markdown。"},
                {"role": "user", "content": source}
            ]
        }))
        .send()
        .await?
        .error_for_status()?
        .json::<ChatResponse>()
        .await?;
    let content = response
        .choices
        .into_iter()
        .next()
        .map(|choice| choice.message.content)
        .filter(|value| !value.trim().is_empty())
        .context("AI returned an empty report")?;
    Ok((content, model.to_string()))
}

fn end_for_date(date: &str) -> OffsetDateTime {
    date_bounds(date)
        .map(|(_, end)| end)
        .unwrap_or_else(|_| OffsetDateTime::now_utc())
}

fn list_or_empty(lines: Vec<String>) -> String {
    if lines.is_empty() {
        "- 暂无可用数据".to_string()
    } else {
        lines.join("\n")
    }
}

fn collapse_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(value: &str, limit: usize) -> String {
    let mut chars = value.chars();
    let prefix = chars.by_ref().take(limit).collect::<String>();
    if chars.next().is_some() {
        format!("{prefix}...")
    } else {
        prefix
    }
}

fn format_duration(milliseconds: u64) -> String {
    let minutes = milliseconds / 60_000;
    if minutes >= 60 {
        format!("{} 小时 {} 分钟", minutes / 60, minutes % 60)
    } else {
        format!("{minutes} 分钟")
    }
}

#[cfg(test)]
mod tests {
    use super::date_bounds;

    #[test]
    fn validates_iso_dates() {
        assert!(date_bounds("2026-07-26").is_ok());
        assert!(date_bounds("2026/07/26").is_err());
        assert!(date_bounds("2026-02-30").is_err());
    }
}
