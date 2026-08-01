use std::collections::{BTreeMap, HashSet};

use anyhow::Context;
use eyes_on_me_shared::{MemoryEntry, MemoryReindexResponse, MemorySearchResponse};
use serde::Deserialize;
use time::{OffsetDateTime, UtcOffset};
use uuid::Uuid;

use crate::app_state::AppState;

pub async fn reindex(state: &AppState) -> anyhow::Result<MemoryReindexResponse> {
    let activities = crate::db::load_all_activities(&state.pool()).await?;
    let screenshots = crate::db::load_all_screenshots(&state.pool()).await?;
    let reports = crate::db::load_all_daily_reports(&state.pool()).await?;
    let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    let now = OffsetDateTime::now_utc();

    #[derive(Default)]
    struct ActivityGroup {
        titles: HashSet<String>,
        domains: HashSet<String>,
        events: usize,
    }
    let mut groups: BTreeMap<(String, String, String), ActivityGroup> = BTreeMap::new();
    for event in activities {
        let date = event.ts.to_offset(offset).date().to_string();
        let group = groups
            .entry((date, event.device_id.clone(), event.app.name.clone()))
            .or_default();
        group.events += 1;
        if let Some(title) = event.window_title.filter(|value| !value.trim().is_empty()) {
            group.titles.insert(title);
        }
        if let Some(domain) = event.browser.and_then(|browser| browser.domain) {
            group.domains.insert(domain);
        }
    }

    let mut indexed_entries = 0;
    for ((date, device, app), group) in groups {
        let title = format!("{date} · {app}");
        let content = format!(
            "设备：{device}\n应用：{app}\n事件数：{}\n窗口：{}\n域名：{}",
            group.events,
            join_limited(group.titles, 30),
            join_limited(group.domains, 30),
        );
        let entry = MemoryEntry {
            id: Uuid::new_v4().to_string(),
            date: date.clone(),
            source_type: "activity".to_string(),
            title,
            content,
            tags: vec![device.clone(), app.clone()],
            score: None,
            created_at: now,
            updated_at: now,
        };
        crate::db::upsert_memory_entry(
            &state.pool(),
            &format!("activity:{date}:{device}:{app}"),
            &entry,
        )
        .await?;
        indexed_entries += 1;
    }

    let mut ocr_by_date: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for screenshot in screenshots {
        let Some(text) = screenshot.ocr_text.filter(|value| !value.trim().is_empty()) else {
            continue;
        };
        let date = screenshot.captured_at.to_offset(offset).date().to_string();
        ocr_by_date
            .entry((date, screenshot.device_id))
            .or_default()
            .push(text);
    }
    for ((date, device), texts) in ocr_by_date {
        let entry = MemoryEntry {
            id: Uuid::new_v4().to_string(),
            date: date.clone(),
            source_type: "screenshot_ocr".to_string(),
            title: format!("{date} · 截图文字"),
            content: texts.into_iter().take(40).collect::<Vec<_>>().join("\n\n"),
            tags: vec![device.clone(), "OCR".to_string()],
            score: None,
            created_at: now,
            updated_at: now,
        };
        crate::db::upsert_memory_entry(&state.pool(), &format!("ocr:{date}:{device}"), &entry)
            .await?;
        indexed_entries += 1;
    }

    for report in reports {
        let entry = MemoryEntry {
            id: Uuid::new_v4().to_string(),
            date: report.date.clone(),
            source_type: "daily_report".to_string(),
            title: format!("{} · 活动日报", report.date),
            content: report.content,
            tags: vec!["日报".to_string()],
            score: None,
            created_at: report.created_at,
            updated_at: report.updated_at,
        };
        crate::db::upsert_memory_entry(&state.pool(), &format!("report:{}", report.date), &entry)
            .await?;
        indexed_entries += 1;
    }

    let mut embedded_entries = 0;
    let mut embedding_failures = 0;
    if embedding_config(state).is_some() {
        for entry in crate::db::load_unembedded_memory_entries(&state.pool(), 500).await? {
            let input = format!(
                "{}\n{}\n{}",
                entry.title,
                entry.content,
                entry.tags.join(" ")
            );
            match embed(state, &input).await {
                Ok(vector) => {
                    crate::db::set_memory_embedding(&state.pool(), &entry.id, &vector).await?;
                    embedded_entries += 1;
                }
                Err(_) => embedding_failures += 1,
            }
        }
    }

    Ok(MemoryReindexResponse {
        indexed_entries,
        embedded_entries,
        embedding_failures,
    })
}

pub async fn search(
    state: &AppState,
    query: &str,
    limit: i64,
) -> anyhow::Result<MemorySearchResponse> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(MemorySearchResponse {
            query: String::new(),
            mode: "recent".to_string(),
            entries: crate::db::load_recent_memory_entries(&state.pool(), limit).await?,
        });
    }

    if embedding_config(state).is_some()
        && let Ok(vector) = embed(state, query).await
    {
        let mut entries = crate::db::load_memory_embeddings(&state.pool())
            .await?
            .into_iter()
            .filter_map(|(mut entry, candidate)| {
                cosine_similarity(&vector, &candidate).map(|score| {
                    entry.score = Some(score);
                    entry
                })
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            right
                .score
                .unwrap_or_default()
                .total_cmp(&left.score.unwrap_or_default())
        });
        entries.truncate(limit as usize);
        if !entries.is_empty() {
            return Ok(MemorySearchResponse {
                query: query.to_string(),
                mode: "semantic".to_string(),
                entries,
            });
        }
    }

    Ok(MemorySearchResponse {
        query: query.to_string(),
        mode: "full_text".to_string(),
        entries: crate::db::search_memory_entries_fts(&state.pool(), query, limit).await?,
    })
}

fn embedding_config(state: &AppState) -> Option<(&crate::config::AiConfig, &str)> {
    let config = state.ai_config()?;
    Some((config, config.embedding_model.as_deref()?))
}

async fn embed(state: &AppState, input: &str) -> anyhow::Result<Vec<f32>> {
    let (config, model) = embedding_config(state).context("embedding model is not configured")?;
    #[derive(Deserialize)]
    struct EmbeddingData {
        embedding: Vec<f32>,
    }
    #[derive(Deserialize)]
    struct EmbeddingResponse {
        data: Vec<EmbeddingData>,
    }
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(45))
        .build()?
        .post(format!("{}/embeddings", config.base_url))
        .bearer_auth(&config.api_key)
        .json(&serde_json::json!({"model": model, "input": input}))
        .send()
        .await?
        .error_for_status()?
        .json::<EmbeddingResponse>()
        .await?;
    response
        .data
        .into_iter()
        .next()
        .map(|item| item.embedding)
        .filter(|value| !value.is_empty())
        .context("embedding API returned no vector")
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> Option<f32> {
    if left.is_empty() || left.len() != right.len() {
        return None;
    }
    let (dot, left_norm, right_norm) =
        left.iter().zip(right).fold((0.0, 0.0, 0.0), |acc, (a, b)| {
            (acc.0 + a * b, acc.1 + a * a, acc.2 + b * b)
        });
    if left_norm == 0.0 || right_norm == 0.0 {
        None
    } else {
        Some(dot / (left_norm.sqrt() * right_norm.sqrt()))
    }
}

fn join_limited(values: HashSet<String>, limit: usize) -> String {
    let mut values = values.into_iter().collect::<Vec<_>>();
    values.sort();
    values.truncate(limit);
    if values.is_empty() {
        "无".to_string()
    } else {
        values.join("；")
    }
}

#[cfg(test)]
mod tests {
    use super::cosine_similarity;

    #[test]
    fn computes_cosine_similarity() {
        assert!((cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]).unwrap() - 1.0).abs() < 0.001);
        assert!(cosine_similarity(&[1.0], &[1.0, 2.0]).is_none());
    }
}
