use std::collections::HashMap;

use anyhow::{Context, ensure};
use eyes_on_me_shared::{
    ActivityEvent, AssistantConversation, AssistantConversationDetail, AssistantMessage,
    AssistantReply, PresenceState,
};
use serde::Deserialize;
use sqlx::Row;
use time::{Duration, OffsetDateTime, UtcOffset};
use uuid::Uuid;

use crate::app_state::AppState;

pub async fn list(state: &AppState) -> anyhow::Result<Vec<AssistantConversation>> {
    let rows = sqlx::query(
        "SELECT id, title, created_at, updated_at FROM assistant_conversations ORDER BY updated_at DESC LIMIT 100",
    )
    .fetch_all(&state.pool())
    .await?;
    rows.iter().map(conversation_from_row).collect()
}

pub async fn detail(
    state: &AppState,
    conversation_id: &str,
) -> anyhow::Result<Option<AssistantConversationDetail>> {
    let Some(row) = sqlx::query(
        "SELECT id, title, created_at, updated_at FROM assistant_conversations WHERE id = ?1",
    )
    .bind(conversation_id)
    .fetch_optional(&state.pool())
    .await?
    else {
        return Ok(None);
    };
    let conversation = conversation_from_row(&row)?;
    let messages = load_messages(state, conversation_id, 200).await?;
    Ok(Some(AssistantConversationDetail {
        conversation,
        messages,
    }))
}

pub async fn ask(
    state: &AppState,
    conversation_id: Option<&str>,
    prompt: &str,
    use_ai: bool,
) -> anyhow::Result<AssistantReply> {
    let prompt = prompt.trim();
    ensure!(!prompt.is_empty(), "assistant prompt is required");
    ensure!(
        prompt.chars().count() <= 8_000,
        "assistant prompt is too long"
    );
    let conversation = ensure_conversation(state, conversation_id, prompt).await?;
    insert_message(state, &conversation.id, "user", prompt, "input").await?;
    let history = load_messages(state, &conversation.id, 20).await?;
    let (range_start, range_end, range_label) = natural_language_range(prompt)?;
    let context = build_local_context(state, range_start, range_end, &range_label).await?;
    let (content, mode) = if use_ai {
        match answer_with_ai(state, prompt, &context, &history).await {
            Ok(content) => (content, "ai_enhanced".to_string()),
            Err(error) => (
                format!(
                    "{}\n\n> AI 增强不可用：{}",
                    template_answer(prompt, &context),
                    error
                ),
                "template_fallback".to_string(),
            ),
        }
    } else {
        (template_answer(prompt, &context), "template".to_string())
    };
    let message = insert_message(state, &conversation.id, "assistant", &content, &mode).await?;
    let conversation = touch_conversation(state, &conversation.id).await?;
    Ok(AssistantReply {
        conversation,
        message,
        starter_prompts: starter_prompts(state).await?,
    })
}

pub async fn starter_prompts(state: &AppState) -> anyhow::Result<Vec<String>> {
    let snapshot = state.snapshot();
    let current = snapshot
        .devices
        .first()
        .map(|activity| format!("我刚才在 {} 上做了什么？", activity.app.name));
    let mut prompts = vec![
        "总结我今天的工作重点".to_string(),
        "今天有哪些可能需要跟进的事项？".to_string(),
        "最近一周时间主要花在哪些应用和网站？".to_string(),
    ];
    if let Some(current) = current {
        prompts.insert(0, current);
    }
    Ok(prompts)
}

pub async fn delete(state: &AppState, conversation_id: &str) -> anyhow::Result<bool> {
    let mut transaction = state.pool().begin().await?;
    sqlx::query("DELETE FROM assistant_messages WHERE conversation_id = ?1")
        .bind(conversation_id)
        .execute(&mut *transaction)
        .await?;
    let result = sqlx::query("DELETE FROM assistant_conversations WHERE id = ?1")
        .bind(conversation_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(result.rows_affected() > 0)
}

async fn ensure_conversation(
    state: &AppState,
    conversation_id: Option<&str>,
    prompt: &str,
) -> anyhow::Result<AssistantConversation> {
    if let Some(id) = conversation_id
        && let Some(detail) = detail(state, id).await?
    {
        return Ok(detail.conversation);
    }
    let id = Uuid::new_v4().to_string();
    let title = prompt.chars().take(36).collect::<String>();
    let now = OffsetDateTime::now_utc();
    sqlx::query(
        "INSERT INTO assistant_conversations (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
    )
    .bind(&id)
    .bind(&title)
    .bind(crate::db::format_timestamp(now)?)
    .execute(&state.pool())
    .await?;
    Ok(AssistantConversation {
        id,
        title,
        created_at: now,
        updated_at: now,
    })
}

async fn touch_conversation(state: &AppState, id: &str) -> anyhow::Result<AssistantConversation> {
    sqlx::query("UPDATE assistant_conversations SET updated_at = ?1 WHERE id = ?2")
        .bind(crate::db::format_timestamp(OffsetDateTime::now_utc())?)
        .bind(id)
        .execute(&state.pool())
        .await?;
    detail(state, id)
        .await?
        .map(|item| item.conversation)
        .context("assistant conversation disappeared")
}

async fn insert_message(
    state: &AppState,
    conversation_id: &str,
    role: &str,
    content: &str,
    mode: &str,
) -> anyhow::Result<AssistantMessage> {
    let message = AssistantMessage {
        id: Uuid::new_v4().to_string(),
        conversation_id: conversation_id.to_string(),
        role: role.to_string(),
        content: content.to_string(),
        mode: mode.to_string(),
        created_at: OffsetDateTime::now_utc(),
    };
    sqlx::query(
        "INSERT INTO assistant_messages (id, conversation_id, role, content, mode, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(&message.id)
    .bind(&message.conversation_id)
    .bind(&message.role)
    .bind(&message.content)
    .bind(&message.mode)
    .bind(crate::db::format_timestamp(message.created_at)?)
    .execute(&state.pool())
    .await?;
    Ok(message)
}

async fn load_messages(
    state: &AppState,
    conversation_id: &str,
    limit: i64,
) -> anyhow::Result<Vec<AssistantMessage>> {
    let rows = sqlx::query(
        r#"
        SELECT id, conversation_id, role, content, mode, created_at FROM (
            SELECT id, conversation_id, role, content, mode, created_at
            FROM assistant_messages WHERE conversation_id = ?1
            ORDER BY created_at DESC LIMIT ?2
        ) ORDER BY created_at ASC
        "#,
    )
    .bind(conversation_id)
    .bind(limit)
    .fetch_all(&state.pool())
    .await?;
    rows.iter().map(message_from_row).collect()
}

#[derive(Debug)]
struct LocalContext {
    range_label: String,
    tracked_ms: u64,
    event_count: usize,
    apps: Vec<(String, u64)>,
    domains: Vec<(String, u64)>,
    recent_contexts: Vec<String>,
    todos: Vec<String>,
    current: Vec<String>,
}

async fn build_local_context(
    state: &AppState,
    start: OffsetDateTime,
    end: OffsetDateTime,
    range_label: &str,
) -> anyhow::Result<LocalContext> {
    let activities = crate::db::load_activities_between(&state.pool(), start, end).await?;
    let mut app_ms = HashMap::<String, u64>::new();
    let mut domain_ms = HashMap::<String, u64>::new();
    let mut tracked_ms = 0_u64;
    for (index, activity) in activities.iter().enumerate() {
        if activity.presence != PresenceState::Active {
            continue;
        }
        let next = activities
            .get(index + 1)
            .filter(|next| next.device_id == activity.device_id)
            .map(|next| next.ts)
            .unwrap_or(end)
            .min(activity.ts + Duration::seconds(120));
        let duration: u64 = (next - activity.ts)
            .whole_milliseconds()
            .max(0)
            .try_into()
            .unwrap_or_default();
        tracked_ms += duration;
        *app_ms.entry(activity.app.name.clone()).or_default() += duration;
        if let Some(domain) = activity
            .browser
            .as_ref()
            .and_then(|item| item.domain.clone())
        {
            *domain_ms.entry(domain).or_default() += duration;
        }
    }
    let rank = |values: HashMap<String, u64>| {
        let mut values = values.into_iter().collect::<Vec<_>>();
        values.sort_by(|left, right| right.1.cmp(&left.1));
        values.truncate(10);
        values
    };
    let recent_contexts = activities
        .iter()
        .rev()
        .filter_map(activity_context)
        .take(12)
        .collect::<Vec<_>>();
    let sessions = crate::governance::load_work_sessions(state, start, end, None).await?;
    let todos = sessions
        .sessions
        .into_iter()
        .flat_map(|session| session.potential_todos)
        .map(|todo| todo.text)
        .take(20)
        .collect();
    let current = state
        .snapshot()
        .devices
        .iter()
        .map(|activity| {
            format!(
                "{}：{}",
                activity.device_id,
                activity_context(activity).unwrap_or_else(|| activity.app.name.clone())
            )
        })
        .collect();
    Ok(LocalContext {
        range_label: range_label.to_string(),
        tracked_ms,
        event_count: activities.len(),
        apps: rank(app_ms),
        domains: rank(domain_ms),
        recent_contexts,
        todos,
        current,
    })
}

fn template_answer(prompt: &str, context: &LocalContext) -> String {
    let wants_current = ["正在", "现在", "current", "doing now"]
        .iter()
        .any(|needle| prompt.to_ascii_lowercase().contains(needle));
    if wants_current {
        return format!(
            "## 当前上下文\n\n{}",
            bullet_list(&context.current, "当前没有在线设备上下文")
        );
    }
    let apps = context
        .apps
        .iter()
        .map(|(name, duration)| format!("{name}：{}", format_duration(*duration)))
        .collect::<Vec<_>>();
    let domains = context
        .domains
        .iter()
        .map(|(name, duration)| format!("{name}：{}", format_duration(*duration)))
        .collect::<Vec<_>>();
    format!(
        "## {}回顾\n\n- 可归属活动：{}\n- 记录事件：{}\n\n### 主要应用\n\n{}\n\n### 主要网站\n\n{}\n\n### 最近上下文\n\n{}\n\n### 待跟进线索\n\n{}",
        context.range_label,
        format_duration(context.tracked_ms),
        context.event_count,
        bullet_list(&apps, "没有应用记录"),
        bullet_list(&domains, "没有网站记录"),
        bullet_list(&context.recent_contexts, "没有上下文记录"),
        bullet_list(&context.todos, "没有提取到明确待办"),
    )
}

async fn answer_with_ai(
    state: &AppState,
    prompt: &str,
    context: &LocalContext,
    history: &[AssistantMessage],
) -> anyhow::Result<String> {
    let config = state.ai_config().context("AI model is not configured")?;
    let model = config
        .chat_model
        .as_deref()
        .context("AI chat model is not configured")?;
    #[derive(Deserialize)]
    struct ChatMessage {
        content: String,
    }
    #[derive(Deserialize)]
    struct Choice {
        message: ChatMessage,
    }
    #[derive(Deserialize)]
    struct Response {
        choices: Vec<Choice>,
    }
    let context_text = template_answer(prompt, context);
    let mut messages = vec![serde_json::json!({
        "role": "system",
        "content": "你是个人工作回顾助手。只能根据提供的本地活动上下文回答；不要虚构完成事项、意图或绩效判断。用简洁 Markdown 回答。"
    })];
    for message in history.iter().rev().take(10).rev() {
        messages.push(serde_json::json!({"role": message.role, "content": message.content}));
    }
    messages.push(serde_json::json!({
        "role": "user",
        "content": format!("本地事实上下文：\n{context_text}\n\n问题：{prompt}")
    }));
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(90))
        .build()?
        .post(format!("{}/chat/completions", config.base_url))
        .bearer_auth(&config.api_key)
        .json(&serde_json::json!({"model": model, "temperature": 0.2, "messages": messages}))
        .send()
        .await?
        .error_for_status()?
        .json::<Response>()
        .await?;
    response
        .choices
        .into_iter()
        .next()
        .map(|choice| choice.message.content)
        .filter(|value| !value.trim().is_empty())
        .context("AI returned an empty answer")
}

fn natural_language_range(
    prompt: &str,
) -> anyhow::Result<(OffsetDateTime, OffsetDateTime, String)> {
    let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    let now = OffsetDateTime::now_utc();
    let local_now = now.to_offset(offset);
    let today = local_now.date();
    let lower = prompt.to_ascii_lowercase();
    if let Some(date) = extract_iso_date(prompt) {
        let (start, end) = crate::reports::date_bounds(&date)?;
        return Ok((start, end, format!("{date} ")));
    }
    if lower.contains("昨天") || lower.contains("yesterday") {
        let date = (today - Duration::days(1)).to_string();
        let (start, end) = crate::reports::date_bounds(&date)?;
        return Ok((start, end, "昨天".to_string()));
    }
    if lower.contains("本周") || lower.contains("一周") || lower.contains("week") {
        return Ok((now - Duration::days(7), now, "最近一周".to_string()));
    }
    if lower.contains("最近三小时") || lower.contains("3小时") || lower.contains("3 hours") {
        return Ok((now - Duration::hours(3), now, "最近三小时".to_string()));
    }
    let date = today.to_string();
    let (start, end) = crate::reports::date_bounds(&date)?;
    Ok((start, end, "今天".to_string()))
}

fn extract_iso_date(value: &str) -> Option<String> {
    let chars = value.chars().collect::<Vec<_>>();
    chars.windows(10).find_map(|window| {
        let candidate = window.iter().collect::<String>();
        (candidate.as_bytes().get(4) == Some(&b'-')
            && candidate.as_bytes().get(7) == Some(&b'-')
            && crate::reports::date_bounds(&candidate).is_ok())
        .then_some(candidate)
    })
}

fn activity_context(activity: &ActivityEvent) -> Option<String> {
    activity
        .browser
        .as_ref()
        .and_then(|browser| browser.page_title.clone())
        .or_else(|| activity.window_title.clone())
        .or_else(|| activity.app.title.clone())
        .map(|title| format!("{} · {}", activity.app.name, title))
}

fn bullet_list(values: &[String], empty: &str) -> String {
    if values.is_empty() {
        format!("- {empty}")
    } else {
        values
            .iter()
            .map(|value| format!("- {value}"))
            .collect::<Vec<_>>()
            .join("\n")
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

fn parse_timestamp(value: String) -> anyhow::Result<OffsetDateTime> {
    OffsetDateTime::parse(&value, &time::format_description::well_known::Rfc3339)
        .map_err(Into::into)
}

fn conversation_from_row(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<AssistantConversation> {
    Ok(AssistantConversation {
        id: row.try_get("id")?,
        title: row.try_get("title")?,
        created_at: parse_timestamp(row.try_get("created_at")?)?,
        updated_at: parse_timestamp(row.try_get("updated_at")?)?,
    })
}

fn message_from_row(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<AssistantMessage> {
    Ok(AssistantMessage {
        id: row.try_get("id")?,
        conversation_id: row.try_get("conversation_id")?,
        role: row.try_get("role")?,
        content: row.try_get("content")?,
        mode: row.try_get("mode")?,
        created_at: parse_timestamp(row.try_get("created_at")?)?,
    })
}

#[cfg(test)]
mod tests {
    use super::extract_iso_date;

    #[test]
    fn extracts_date_from_natural_language_prompt() {
        assert_eq!(
            extract_iso_date("总结 2026-07-21 的工作"),
            Some("2026-07-21".to_string())
        );
    }
}
