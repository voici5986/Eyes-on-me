use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::{Context, bail};
use eyes_on_me_shared::{
    ActivityApp, ActivityEvent, ActivityKind, ActivitySearchHit, AgentDiagnostics, CategoryRule,
    CategoryTarget, DailyReport, DashboardSnapshot, DeviceRecordingState, DeviceStatus,
    MemoryEntry, Platform, PresenceState, RemoteMirrorStatus, ReportPreferences, ReviewSettings,
    ScreenshotRecord, WorkScheduleSegment,
};
use sqlx::{
    ConnectOptions, Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use time::{OffsetDateTime, UtcOffset};

pub async fn connect(database_url: &str) -> anyhow::Result<SqlitePool> {
    let sqlite_path = database_url.trim_start_matches("sqlite://");
    if !sqlite_path.is_empty() && sqlite_path != ":memory:" {
        ensure_parent_dir(sqlite_path).await?;
        migrate_legacy_database_file(sqlite_path)?;
    }

    let options = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .disable_statement_logging();

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .with_context(|| format!("failed to connect to database at {database_url}"))?;

    migrate(&pool).await?;
    Ok(pool)
}

fn migrate_legacy_database_file(target_path: &str) -> anyhow::Result<()> {
    let target = PathBuf::from(target_path);
    if target.exists() {
        return Ok(());
    }

    let parent = match target.parent() {
        Some(parent) => parent,
        None => return Ok(()),
    };

    let legacy_candidates = [
        parent.join("amiokay.db"),
        parent.join("../data/amiokay.db"),
        parent.join("../DB/amiokay.db"),
    ];

    if let Some(source) = legacy_candidates.into_iter().find(|path| path.exists()) {
        fs::copy(&source, &target).with_context(|| {
            format!(
                "failed to migrate legacy sqlite database from {} to {}",
                source.display(),
                target.display()
            )
        })?;
    }

    Ok(())
}

pub async fn load_snapshot(pool: &SqlitePool) -> anyhow::Result<DashboardSnapshot> {
    let recent_rows = sqlx::query(
        r#"
        SELECT event_id, ts, device_id, agent_name, platform, kind, app_json, window_title, browser_json, presence, source
        FROM activity_log
        WHERE kind != 'activity_sample'
        ORDER BY ts DESC
        LIMIT 20
        "#,
    )
    .fetch_all(pool)
    .await?;

    let recent_activities = recent_rows
        .iter()
        .map(activity_from_row)
        .collect::<anyhow::Result<Vec<_>>>()?;

    let device_rows = sqlx::query(
        r#"
        SELECT event_id, ts, device_id, agent_name, platform, kind, app_json, window_title, browser_json, presence, source
        FROM (
            SELECT *,
                   ROW_NUMBER() OVER (PARTITION BY device_id ORDER BY ts DESC) AS row_num
            FROM activity_log
        )
        WHERE row_num = 1
        ORDER BY ts DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    let devices = device_rows
        .iter()
        .map(activity_from_row)
        .collect::<anyhow::Result<Vec<_>>>()?;

    let latest_status = sqlx::query(
        r#"
        SELECT ts, device_id, agent_name, platform, status_text, source
        FROM device_status
        ORDER BY ts DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(pool)
    .await?
    .map(|row| status_from_row(&row))
    .transpose()?;

    Ok(DashboardSnapshot {
        devices,
        latest_status,
        recent_activities,
    })
}

pub async fn persist_activity(pool: &SqlitePool, event: &ActivityEvent) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO activity_log (
            event_id, ts, device_id, agent_name, platform, kind, app_json, window_title, browser_json, presence, source
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        ON CONFLICT(event_id) DO UPDATE SET
            ts = excluded.ts,
            device_id = excluded.device_id,
            agent_name = excluded.agent_name,
            platform = excluded.platform,
            kind = excluded.kind,
            app_json = excluded.app_json,
            window_title = excluded.window_title,
            browser_json = excluded.browser_json,
            presence = excluded.presence,
            source = excluded.source
        "#,
    )
    .bind(&event.event_id)
    .bind(format_timestamp(event.ts)?)
    .bind(&event.device_id)
    .bind(&event.agent_name)
    .bind(platform_to_str(&event.platform))
    .bind(kind_to_str(&event.kind))
    .bind(serde_json::to_string(&event.app)?)
    .bind(&event.window_title)
    .bind(event.browser.as_ref().map(serde_json::to_string).transpose()?)
    .bind(presence_to_str(event.presence))
    .bind(&event.source)
    .execute(pool)
    .await?;

    upsert_activity_search(pool, event).await?;

    Ok(())
}

pub async fn persist_status(pool: &SqlitePool, status: &DeviceStatus) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO device_status (
            device_id, ts, agent_name, platform, status_text, source
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT(device_id) DO UPDATE SET
            ts = excluded.ts,
            agent_name = excluded.agent_name,
            platform = excluded.platform,
            status_text = excluded.status_text,
            source = excluded.source
        "#,
    )
    .bind(&status.device_id)
    .bind(format_timestamp(status.ts)?)
    .bind(&status.agent_name)
    .bind(platform_to_str(&status.platform))
    .bind(&status.status_text)
    .bind(&status.source)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn load_device_statuses(pool: &SqlitePool) -> anyhow::Result<Vec<DeviceStatus>> {
    let rows = sqlx::query(
        r#"
        SELECT ts, device_id, agent_name, platform, status_text, source
        FROM device_status
        ORDER BY ts DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    rows.iter().map(status_from_row).collect()
}

pub async fn load_device_status(
    pool: &SqlitePool,
    device_id: &str,
) -> anyhow::Result<Option<DeviceStatus>> {
    sqlx::query(
        r#"
        SELECT ts, device_id, agent_name, platform, status_text, source
        FROM device_status
        WHERE device_id = ?1
        LIMIT 1
        "#,
    )
    .bind(device_id)
    .fetch_optional(pool)
    .await?
    .map(|row| status_from_row(&row))
    .transpose()
}

pub async fn load_latest_activity_for_device(
    pool: &SqlitePool,
    device_id: &str,
) -> anyhow::Result<Option<ActivityEvent>> {
    sqlx::query(
        r#"
        SELECT event_id, ts, device_id, agent_name, platform, kind, app_json, window_title, browser_json, presence, source
        FROM activity_log
        WHERE device_id = ?1
        ORDER BY ts DESC
        LIMIT 1
        "#,
    )
    .bind(device_id)
    .fetch_optional(pool)
    .await?
    .map(|row| activity_from_row(&row))
    .transpose()
}

pub async fn load_activity(
    pool: &SqlitePool,
    event_id: &str,
) -> anyhow::Result<Option<ActivityEvent>> {
    sqlx::query(
        r#"
        SELECT event_id, ts, device_id, agent_name, platform, kind, app_json,
               window_title, browser_json, presence, source
        FROM activity_log WHERE event_id = ?1
        "#,
    )
    .bind(event_id)
    .fetch_optional(pool)
    .await?
    .map(|row| activity_from_row(&row))
    .transpose()
}

pub async fn load_recent_activities_for_device(
    pool: &SqlitePool,
    device_id: &str,
    limit: i64,
) -> anyhow::Result<Vec<ActivityEvent>> {
    let rows = sqlx::query(
        r#"
        SELECT event_id, ts, device_id, agent_name, platform, kind, app_json, window_title, browser_json, presence, source
        FROM activity_log
        WHERE device_id = ?1
          AND kind != 'activity_sample'
        ORDER BY ts DESC
        LIMIT ?2
        "#,
    )
    .bind(device_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.iter().map(activity_from_row).collect()
}

pub async fn load_all_activities_for_device(
    pool: &SqlitePool,
    device_id: &str,
) -> anyhow::Result<Vec<ActivityEvent>> {
    let rows = sqlx::query(
        r#"
        SELECT event_id, ts, device_id, agent_name, platform, kind, app_json, window_title, browser_json, presence, source
        FROM activity_log
        WHERE device_id = ?1
        ORDER BY ts ASC
        "#,
    )
    .bind(device_id)
    .fetch_all(pool)
    .await?;

    rows.iter().map(activity_from_row).collect()
}

pub async fn load_analysis_activities_for_device(
    pool: &SqlitePool,
    device_id: &str,
    window_start: Option<OffsetDateTime>,
) -> anyhow::Result<Vec<ActivityEvent>> {
    let Some(window_start) = window_start else {
        return load_all_activities_for_device(pool, device_id).await;
    };
    let cutoff = format_timestamp(window_start)?;
    let rows = sqlx::query(
        r#"
        SELECT event_id, ts, device_id, agent_name, platform, kind, app_json, window_title, browser_json, presence, source
        FROM activity_log
        WHERE device_id = ?1 AND ts >= ?2
        UNION ALL
        SELECT event_id, ts, device_id, agent_name, platform, kind, app_json, window_title, browser_json, presence, source
        FROM activity_log
        WHERE event_id = (
            SELECT event_id
            FROM activity_log
            WHERE device_id = ?1 AND ts < ?2
            ORDER BY ts DESC
            LIMIT 1
        )
        ORDER BY ts ASC
        "#,
    )
    .bind(device_id)
    .bind(cutoff)
    .fetch_all(pool)
    .await?;

    rows.iter().map(activity_from_row).collect()
}

pub async fn search_activities(
    pool: &SqlitePool,
    query: &str,
    device_id: Option<&str>,
    limit: i64,
) -> anyhow::Result<Vec<ActivitySearchHit>> {
    let fts_query = build_fts_query(query)?;
    let rows = sqlx::query(
        r#"
        WITH search_hits AS (
            SELECT
                activity_search.event_id AS event_id,
                snippet(activity_search_fts, -1, '', '', ' … ', 12) AS snippet,
                bm25(activity_search_fts) AS score
            FROM activity_search_fts
            JOIN activity_search ON activity_search.rowid = activity_search_fts.rowid
            WHERE activity_search_fts MATCH ?1
              AND (?2 IS NULL OR activity_search.device_id = ?2)
            UNION ALL
            SELECT
                screenshot_search_fts.event_id AS event_id,
                snippet(screenshot_search_fts, -1, '', '', ' … ', 20) AS snippet,
                bm25(screenshot_search_fts) AS score
            FROM screenshot_search_fts
            WHERE screenshot_search_fts MATCH ?1
              AND (?2 IS NULL OR screenshot_search_fts.device_id = ?2)
        ), ranked AS (
            SELECT event_id, MIN(score) AS score, MAX(snippet) AS snippet
            FROM search_hits
            GROUP BY event_id
        )
        SELECT
            l.event_id,
            l.ts,
            l.device_id,
            l.agent_name,
            l.platform,
            l.kind,
            l.app_json,
            l.window_title,
            l.browser_json,
            l.presence,
            l.source,
            ranked.snippet,
            ranked.score
        FROM ranked
        JOIN activity_log l ON l.event_id = ranked.event_id
        ORDER BY ranked.score, l.ts DESC
        LIMIT ?3
        "#,
    )
    .bind(&fts_query)
    .bind(device_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            Ok(ActivitySearchHit {
                activity: activity_from_row(&row)?,
                snippet: row.try_get("snippet")?,
                score: row.try_get::<f64, _>("score").unwrap_or_default() as f32,
            })
        })
        .collect()
}

pub async fn count_activity_search_results(
    pool: &SqlitePool,
    query: &str,
    device_id: Option<&str>,
) -> anyhow::Result<i64> {
    let fts_query = build_fts_query(query)?;
    sqlx::query_scalar(
        r#"
        SELECT COUNT(DISTINCT event_id) FROM (
            SELECT activity_search.event_id
            FROM activity_search_fts
            JOIN activity_search ON activity_search.rowid = activity_search_fts.rowid
            WHERE activity_search_fts MATCH ?1
              AND (?2 IS NULL OR activity_search.device_id = ?2)
            UNION ALL
            SELECT screenshot_search_fts.event_id
            FROM screenshot_search_fts
            WHERE screenshot_search_fts MATCH ?1
              AND (?2 IS NULL OR screenshot_search_fts.device_id = ?2)
        )
        "#,
    )
    .bind(&fts_query)
    .bind(device_id)
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

#[derive(Debug, Clone)]
pub struct ActivityIdentity {
    pub device_id: String,
    pub ts: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct StoredScreenshot {
    pub record: ScreenshotRecord,
    pub storage_key: String,
    pub thumbnail_storage_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RemoteMirrorRecord {
    pub remote_key: String,
}

pub async fn load_activity_identity(
    pool: &SqlitePool,
    event_id: &str,
) -> anyhow::Result<Option<ActivityIdentity>> {
    let row = sqlx::query("SELECT device_id, ts FROM activity_log WHERE event_id = ?1")
        .bind(event_id)
        .fetch_optional(pool)
        .await?;

    row.map(|row| {
        Ok(ActivityIdentity {
            device_id: row.try_get("device_id")?,
            ts: parse_timestamp(row.try_get("ts")?)?,
        })
    })
    .transpose()
}

pub async fn insert_screenshot(
    pool: &SqlitePool,
    record: &ScreenshotRecord,
    storage_key: &str,
    thumbnail_storage_key: Option<&str>,
    visual_hash: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO screenshots (
            id, event_id, device_id, captured_at, mime_type, byte_size,
            width, height, sha256, storage_key, thumbnail_storage_key, visual_hash,
            ocr_status, ocr_text, ocr_error, ocr_attempts, duplicate_of, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
        "#,
    )
    .bind(&record.id)
    .bind(&record.event_id)
    .bind(&record.device_id)
    .bind(format_timestamp(record.captured_at)?)
    .bind(&record.mime_type)
    .bind(i64::try_from(record.byte_size).unwrap_or(i64::MAX))
    .bind(record.width.map(i64::from))
    .bind(record.height.map(i64::from))
    .bind(&record.sha256)
    .bind(storage_key)
    .bind(thumbnail_storage_key)
    .bind(visual_hash)
    .bind(&record.ocr_status)
    .bind(&record.ocr_text)
    .bind(&record.ocr_error)
    .bind(i64::from(record.ocr_attempts))
    .bind(&record.duplicate_of)
    .bind(format_timestamp(record.created_at)?)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_screenshot_ocr(
    pool: &SqlitePool,
    screenshot_id: &str,
    status: &str,
    text: Option<&str>,
    error: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE screenshots SET ocr_status = ?1, ocr_text = ?2, ocr_error = ?3, ocr_attempted_at = ?4 WHERE id = ?5",
    )
    .bind(status)
    .bind(text)
    .bind(error)
    .bind(format_timestamp(OffsetDateTime::now_utc())?)
    .bind(screenshot_id)
    .execute(pool)
    .await?;

    sqlx::query("DELETE FROM screenshot_search_fts WHERE screenshot_id = ?1")
        .bind(screenshot_id)
        .execute(pool)
        .await?;

    if let Some(text) = text.filter(|value| !value.trim().is_empty()) {
        sqlx::query(
            r#"
            INSERT INTO screenshot_search_fts (screenshot_id, event_id, device_id, ocr_text)
            SELECT id, event_id, device_id, ?1 FROM screenshots WHERE id = ?2
            "#,
        )
        .bind(text)
        .bind(screenshot_id)
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub async fn load_screenshot(
    pool: &SqlitePool,
    screenshot_id: &str,
) -> anyhow::Result<Option<StoredScreenshot>> {
    sqlx::query(
        r#"
        SELECT id, event_id, device_id, captured_at, mime_type, byte_size,
               width, height, sha256, storage_key, thumbnail_storage_key, visual_hash,
               ocr_status, ocr_text, ocr_error, ocr_attempts, duplicate_of, created_at
        FROM screenshots WHERE id = ?1
        "#,
    )
    .bind(screenshot_id)
    .fetch_optional(pool)
    .await?
    .map(|row| stored_screenshot_from_row(&row))
    .transpose()
}

pub async fn load_screenshot_for_event(
    pool: &SqlitePool,
    event_id: &str,
) -> anyhow::Result<Option<StoredScreenshot>> {
    sqlx::query(
        r#"
        SELECT id, event_id, device_id, captured_at, mime_type, byte_size,
               width, height, sha256, storage_key, thumbnail_storage_key, visual_hash,
               ocr_status, ocr_text, ocr_error, ocr_attempts, duplicate_of, created_at
        FROM screenshots WHERE event_id = ?1
        "#,
    )
    .bind(event_id)
    .fetch_optional(pool)
    .await?
    .map(|row| stored_screenshot_from_row(&row))
    .transpose()
}

pub async fn load_screenshots_for_device(
    pool: &SqlitePool,
    device_id: &str,
    limit: i64,
) -> anyhow::Result<Vec<ScreenshotRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT id, event_id, device_id, captured_at, mime_type, byte_size,
               width, height, sha256, storage_key, thumbnail_storage_key, visual_hash,
               ocr_status, ocr_text, ocr_error, ocr_attempts, duplicate_of, created_at
        FROM screenshots
        WHERE device_id = ?1
        ORDER BY captured_at DESC
        LIMIT ?2
        "#,
    )
    .bind(device_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.iter()
        .map(stored_screenshot_from_row)
        .map(|result| result.map(|stored| stored.record))
        .collect()
}

pub async fn load_screenshots_between(
    pool: &SqlitePool,
    start: OffsetDateTime,
    end: OffsetDateTime,
) -> anyhow::Result<Vec<ScreenshotRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT id, event_id, device_id, captured_at, mime_type, byte_size,
               width, height, sha256, storage_key, thumbnail_storage_key, visual_hash,
               ocr_status, ocr_text, ocr_error, ocr_attempts, duplicate_of, created_at
        FROM screenshots
        WHERE captured_at >= ?1 AND captured_at < ?2
        ORDER BY captured_at ASC
        "#,
    )
    .bind(format_timestamp(start)?)
    .bind(format_timestamp(end)?)
    .fetch_all(pool)
    .await?;

    rows.iter()
        .map(stored_screenshot_from_row)
        .map(|result| result.map(|stored| stored.record))
        .collect()
}

pub async fn load_activities_between(
    pool: &SqlitePool,
    start: OffsetDateTime,
    end: OffsetDateTime,
) -> anyhow::Result<Vec<ActivityEvent>> {
    let rows = sqlx::query(
        r#"
        SELECT event_id, ts, device_id, agent_name, platform, kind, app_json,
               window_title, browser_json, presence, source
        FROM activity_log
        WHERE ts >= ?1 AND ts < ?2
        ORDER BY ts ASC
        "#,
    )
    .bind(format_timestamp(start)?)
    .bind(format_timestamp(end)?)
    .fetch_all(pool)
    .await?;
    rows.iter().map(activity_from_row).collect()
}

pub async fn load_all_activities(pool: &SqlitePool) -> anyhow::Result<Vec<ActivityEvent>> {
    let rows = sqlx::query(
        r#"
        SELECT event_id, ts, device_id, agent_name, platform, kind, app_json,
               window_title, browser_json, presence, source
        FROM activity_log ORDER BY ts ASC
        "#,
    )
    .fetch_all(pool)
    .await?;
    rows.iter().map(activity_from_row).collect()
}

pub async fn load_timeline_activities(
    pool: &SqlitePool,
    start: OffsetDateTime,
    end: OffsetDateTime,
    device_id: Option<&str>,
) -> anyhow::Result<Vec<ActivityEvent>> {
    let rows = sqlx::query(
        r#"
        SELECT event_id, ts, device_id, agent_name, platform, kind, app_json,
               window_title, browser_json, presence, source
        FROM activity_log
        WHERE ts >= ?1 AND ts < ?2
          AND (?3 IS NULL OR device_id = ?3)
        ORDER BY ts ASC
        "#,
    )
    .bind(format_timestamp(start)?)
    .bind(format_timestamp(end)?)
    .bind(device_id)
    .fetch_all(pool)
    .await?;
    rows.iter().map(activity_from_row).collect()
}

pub async fn load_screenshots_for_event_ids(
    pool: &SqlitePool,
    event_ids: &[String],
) -> anyhow::Result<Vec<ScreenshotRecord>> {
    let mut screenshots = Vec::new();
    for event_id in event_ids {
        if let Some(stored) = load_screenshot_for_event(pool, event_id).await? {
            screenshots.push(stored.record);
        }
    }
    Ok(screenshots)
}

pub async fn load_all_screenshots(pool: &SqlitePool) -> anyhow::Result<Vec<ScreenshotRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT id, event_id, device_id, captured_at, mime_type, byte_size,
               width, height, sha256, storage_key, thumbnail_storage_key, visual_hash,
               ocr_status, ocr_text, ocr_error, ocr_attempts, duplicate_of, created_at
        FROM screenshots ORDER BY captured_at ASC
        "#,
    )
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(stored_screenshot_from_row)
        .map(|result| result.map(|stored| stored.record))
        .collect()
}

pub async fn load_recent_screenshots(
    pool: &SqlitePool,
    limit: i64,
) -> anyhow::Result<Vec<ScreenshotRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT id, event_id, device_id, captured_at, mime_type, byte_size,
               width, height, sha256, storage_key, thumbnail_storage_key, visual_hash,
               ocr_status, ocr_text, ocr_error, ocr_attempts, duplicate_of, created_at
        FROM screenshots ORDER BY captured_at DESC LIMIT ?1
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(stored_screenshot_from_row)
        .map(|result| result.map(|stored| stored.record))
        .collect()
}

pub async fn load_all_stored_screenshots(
    pool: &SqlitePool,
) -> anyhow::Result<Vec<StoredScreenshot>> {
    let rows = sqlx::query(
        r#"
        SELECT id, event_id, device_id, captured_at, mime_type, byte_size,
               width, height, sha256, storage_key, thumbnail_storage_key, visual_hash,
               ocr_status, ocr_text, ocr_error, ocr_attempts, duplicate_of, created_at
        FROM screenshots ORDER BY captured_at ASC
        "#,
    )
    .fetch_all(pool)
    .await?;
    rows.iter().map(stored_screenshot_from_row).collect()
}

pub async fn set_remote_mirror_status(
    pool: &SqlitePool,
    screenshot_id: &str,
    provider: &str,
    remote_key: &str,
    status: &str,
    error: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO screenshot_remote_mirror (screenshot_id, provider, remote_key, status, error, updated_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT(screenshot_id) DO UPDATE SET
            provider = excluded.provider,
            remote_key = excluded.remote_key,
            status = excluded.status,
            error = excluded.error,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(screenshot_id)
    .bind(provider)
    .bind(remote_key)
    .bind(status)
    .bind(error)
    .bind(format_timestamp(OffsetDateTime::now_utc())?)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn load_remote_mirror_status(
    pool: &SqlitePool,
    configured: bool,
    provider: Option<String>,
) -> anyhow::Result<RemoteMirrorStatus> {
    let rows = sqlx::query(
        "SELECT status, COUNT(*) AS count FROM screenshot_remote_mirror GROUP BY status",
    )
    .fetch_all(pool)
    .await?;
    let mut pending = 0_u64;
    let mut complete = 0_u64;
    let mut failed = 0_u64;
    for row in rows {
        let count = u64::try_from(row.try_get::<i64, _>("count")?).unwrap_or_default();
        match row.try_get::<String, _>("status")?.as_str() {
            "complete" => complete = count,
            "failed" => failed = count,
            _ => pending = pending.saturating_add(count),
        }
    }
    let last_error = sqlx::query_scalar::<_, String>(
        "SELECT error FROM screenshot_remote_mirror WHERE error IS NOT NULL ORDER BY updated_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(RemoteMirrorStatus {
        configured,
        provider,
        pending,
        complete,
        failed,
        last_error,
    })
}

pub async fn load_remote_mirror_record(
    pool: &SqlitePool,
    screenshot_id: &str,
) -> anyhow::Result<Option<RemoteMirrorRecord>> {
    sqlx::query("SELECT remote_key FROM screenshot_remote_mirror WHERE screenshot_id = ?1")
        .bind(screenshot_id)
        .fetch_optional(pool)
        .await?
        .map(|row| {
            Ok(RemoteMirrorRecord {
                remote_key: row.try_get("remote_key")?,
            })
        })
        .transpose()
}

pub async fn load_retryable_remote_mirrors(
    pool: &SqlitePool,
    limit: i64,
) -> anyhow::Result<Vec<(String, String, String)>> {
    let rows = sqlx::query(
        r#"
        SELECT screenshots.id, screenshots.storage_key, screenshots.mime_type
        FROM screenshot_remote_mirror
        JOIN screenshots ON screenshots.id = screenshot_remote_mirror.screenshot_id
        WHERE screenshot_remote_mirror.status IN ('pending', 'failed')
        ORDER BY screenshot_remote_mirror.updated_at ASC LIMIT ?1
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok((
                row.try_get("id")?,
                row.try_get("storage_key")?,
                row.try_get("mime_type")?,
            ))
        })
        .collect()
}

pub async fn load_recent_ocr_candidates(
    pool: &SqlitePool,
    device_id: &str,
    limit: i64,
) -> anyhow::Result<Vec<(String, String, String)>> {
    let rows = sqlx::query(
        r#"
        SELECT id, visual_hash, ocr_text FROM screenshots
        WHERE device_id = ?1 AND visual_hash IS NOT NULL
          AND ocr_status = 'complete' AND ocr_text IS NOT NULL
        ORDER BY captured_at DESC LIMIT ?2
        "#,
    )
    .bind(device_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok((
                row.try_get("id")?,
                row.try_get("visual_hash")?,
                row.try_get("ocr_text")?,
            ))
        })
        .collect()
}

pub async fn mark_screenshot_ocr_pending(
    pool: &SqlitePool,
    screenshot_id: &str,
) -> anyhow::Result<bool> {
    let result = sqlx::query(
        "UPDATE screenshots SET ocr_status = 'pending', ocr_text = NULL, ocr_error = NULL, duplicate_of = NULL, ocr_attempts = ocr_attempts + 1 WHERE id = ?1",
    )
    .bind(screenshot_id)
    .execute(pool)
    .await?;
    if result.rows_affected() > 0 {
        sqlx::query("DELETE FROM screenshot_search_fts WHERE screenshot_id = ?1")
            .bind(screenshot_id)
            .execute(pool)
            .await?;
    }
    Ok(result.rows_affected() > 0)
}

pub async fn set_screenshot_thumbnail_key(
    pool: &SqlitePool,
    screenshot_id: &str,
    storage_key: &str,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE screenshots SET thumbnail_storage_key = ?1 WHERE id = ?2")
        .bind(storage_key)
        .bind(screenshot_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_screenshot_row(
    pool: &SqlitePool,
    screenshot_id: &str,
) -> anyhow::Result<Option<StoredScreenshot>> {
    let stored = load_screenshot(pool, screenshot_id).await?;
    let Some(stored) = stored else {
        return Ok(None);
    };
    let mut transaction = pool.begin().await?;
    sqlx::query("DELETE FROM screenshot_search_fts WHERE screenshot_id = ?1")
        .bind(screenshot_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM screenshot_remote_mirror WHERE screenshot_id = ?1")
        .bind(screenshot_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM screenshots WHERE id = ?1")
        .bind(screenshot_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(Some(stored))
}

pub async fn persist_agent_diagnostics(
    pool: &SqlitePool,
    diagnostics: &AgentDiagnostics,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO agent_diagnostics (device_id, payload_json, updated_at)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(device_id) DO UPDATE SET payload_json = excluded.payload_json, updated_at = excluded.updated_at
        "#,
    )
    .bind(&diagnostics.device_id)
    .bind(serde_json::to_string(diagnostics)?)
    .bind(&diagnostics.updated_at)
    .execute(pool).await?;
    Ok(())
}

pub async fn load_agent_diagnostics(pool: &SqlitePool) -> anyhow::Result<Vec<AgentDiagnostics>> {
    let rows = sqlx::query("SELECT payload_json FROM agent_diagnostics ORDER BY updated_at DESC")
        .fetch_all(pool)
        .await?;
    rows.into_iter()
        .map(|row| {
            serde_json::from_str(&row.try_get::<String, _>("payload_json")?).map_err(Into::into)
        })
        .collect()
}

pub async fn load_recording_state(
    pool: &SqlitePool,
    device_id: &str,
) -> anyhow::Result<DeviceRecordingState> {
    let row = sqlx::query(
        r#"
        SELECT device_id, desired_enabled, applied_enabled, revision, updated_at, acknowledged_at
        FROM device_recording_control WHERE device_id = ?1
        "#,
    )
    .bind(device_id)
    .fetch_optional(pool)
    .await?;
    match row {
        Some(row) => recording_state_from_row(&row),
        None => Ok(DeviceRecordingState {
            device_id: device_id.to_string(),
            desired_enabled: true,
            applied_enabled: None,
            revision: 0,
            updated_at: OffsetDateTime::now_utc(),
            acknowledged_at: None,
        }),
    }
}

pub async fn set_recording_desired(
    pool: &SqlitePool,
    device_id: &str,
    enabled: bool,
) -> anyhow::Result<DeviceRecordingState> {
    let now = format_timestamp(OffsetDateTime::now_utc())?;
    sqlx::query(
        r#"
        INSERT INTO device_recording_control (
            device_id, desired_enabled, applied_enabled, revision, updated_at, acknowledged_at
        ) VALUES (?1, ?2, NULL, 1, ?3, NULL)
        ON CONFLICT(device_id) DO UPDATE SET
            desired_enabled = excluded.desired_enabled,
            revision = device_recording_control.revision + 1,
            updated_at = excluded.updated_at,
            acknowledged_at = CASE
                WHEN device_recording_control.applied_enabled = excluded.desired_enabled
                THEN device_recording_control.acknowledged_at ELSE NULL END
        "#,
    )
    .bind(device_id)
    .bind(enabled)
    .bind(now)
    .execute(pool)
    .await?;
    load_recording_state(pool, device_id).await
}

pub async fn acknowledge_recording_state(
    pool: &SqlitePool,
    device_id: &str,
    enabled: bool,
    revision: u64,
) -> anyhow::Result<DeviceRecordingState> {
    let now = format_timestamp(OffsetDateTime::now_utc())?;
    sqlx::query(
        r#"
        INSERT INTO device_recording_control (
            device_id, desired_enabled, applied_enabled, revision, updated_at, acknowledged_at
        ) VALUES (?1, ?2, ?2, ?3, ?4, ?4)
        ON CONFLICT(device_id) DO UPDATE SET
            applied_enabled = excluded.applied_enabled,
            acknowledged_at = excluded.acknowledged_at
        WHERE device_recording_control.revision = excluded.revision
        "#,
    )
    .bind(device_id)
    .bind(enabled)
    .bind(i64::try_from(revision).unwrap_or(i64::MAX))
    .bind(now)
    .execute(pool)
    .await?;
    load_recording_state(pool, device_id).await
}

pub async fn load_review_settings(pool: &SqlitePool) -> anyhow::Result<ReviewSettings> {
    let category_rows = sqlx::query(
        "SELECT id, name, color, target, pattern, priority FROM category_rules ORDER BY priority DESC, name ASC",
    )
    .fetch_all(pool)
    .await?;
    let category_rules = category_rows
        .into_iter()
        .map(|row| {
            Ok(CategoryRule {
                id: row.try_get("id")?,
                name: row.try_get("name")?,
                color: row.try_get("color")?,
                target: match row.try_get::<String, _>("target")?.as_str() {
                    "domain" => CategoryTarget::Domain,
                    _ => CategoryTarget::App,
                },
                pattern: row.try_get("pattern")?,
                priority: row.try_get("priority")?,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    let schedule_rows = sqlx::query(
        "SELECT id, weekday, start_minute, end_minute FROM work_schedule_segments ORDER BY weekday, start_minute",
    )
    .fetch_all(pool)
    .await?;
    let work_schedule = schedule_rows
        .into_iter()
        .map(|row| {
            Ok(WorkScheduleSegment {
                id: row.try_get("id")?,
                weekday: u8::try_from(row.try_get::<i64, _>("weekday")?).unwrap_or_default(),
                start_minute: u16::try_from(row.try_get::<i64, _>("start_minute")?)
                    .unwrap_or_default(),
                end_minute: u16::try_from(row.try_get::<i64, _>("end_minute")?).unwrap_or_default(),
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    let report_preferences = sqlx::query_scalar::<_, String>(
        "SELECT value_json FROM review_settings WHERE key = 'report_preferences'",
    )
    .fetch_optional(pool)
    .await?
    .and_then(|value| serde_json::from_str::<ReportPreferences>(&value).ok())
    .unwrap_or_default();

    Ok(ReviewSettings {
        category_rules,
        work_schedule,
        report_preferences,
    })
}

pub async fn save_review_settings(
    pool: &SqlitePool,
    settings: &ReviewSettings,
) -> anyhow::Result<()> {
    let mut transaction = pool.begin().await?;
    sqlx::query("DELETE FROM category_rules")
        .execute(&mut *transaction)
        .await?;
    for rule in &settings.category_rules {
        sqlx::query(
            "INSERT INTO category_rules (id, name, color, target, pattern, priority) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(&rule.id)
        .bind(&rule.name)
        .bind(&rule.color)
        .bind(match rule.target {
            CategoryTarget::App => "app",
            CategoryTarget::Domain => "domain",
        })
        .bind(&rule.pattern)
        .bind(rule.priority)
        .execute(&mut *transaction)
        .await?;
    }
    sqlx::query("DELETE FROM work_schedule_segments")
        .execute(&mut *transaction)
        .await?;
    for segment in &settings.work_schedule {
        sqlx::query(
            "INSERT INTO work_schedule_segments (id, weekday, start_minute, end_minute) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(&segment.id)
        .bind(i64::from(segment.weekday))
        .bind(i64::from(segment.start_minute))
        .bind(i64::from(segment.end_minute))
        .execute(&mut *transaction)
        .await?;
    }
    sqlx::query(
        r#"
        INSERT INTO review_settings (key, value_json, updated_at)
        VALUES ('report_preferences', ?1, ?2)
        ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at
        "#,
    )
    .bind(serde_json::to_string(&settings.report_preferences)?)
    .bind(format_timestamp(OffsetDateTime::now_utc())?)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

#[derive(Debug)]
pub struct BulkDeletedActivities {
    pub screenshots: Vec<StoredScreenshot>,
    pub deleted_activities: u64,
    pub affected_dates: Vec<String>,
}

pub async fn delete_activity_rows(
    pool: &SqlitePool,
    activities: &[ActivityEvent],
) -> anyhow::Result<BulkDeletedActivities> {
    if activities.is_empty() {
        return Ok(BulkDeletedActivities {
            screenshots: Vec::new(),
            deleted_activities: 0,
            affected_dates: Vec::new(),
        });
    }
    let mut screenshots = Vec::new();
    for activity in activities {
        if let Some(item) = load_screenshot_for_event(pool, &activity.event_id).await? {
            screenshots.push(item);
        }
    }
    let local_offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    let affected_dates = activities
        .iter()
        .map(|activity| activity.ts.to_offset(local_offset).date().to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut transaction = pool.begin().await?;
    let mut deleted_activities = 0_u64;
    for activity in activities {
        sqlx::query(
            "DELETE FROM screenshot_remote_mirror WHERE screenshot_id IN (SELECT id FROM screenshots WHERE event_id = ?1)",
        )
        .bind(&activity.event_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("DELETE FROM screenshot_search_fts WHERE event_id = ?1")
            .bind(&activity.event_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM screenshots WHERE event_id = ?1")
            .bind(&activity.event_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM activity_search WHERE event_id = ?1")
            .bind(&activity.event_id)
            .execute(&mut *transaction)
            .await?;
        deleted_activities += sqlx::query("DELETE FROM activity_log WHERE event_id = ?1")
            .bind(&activity.event_id)
            .execute(&mut *transaction)
            .await?
            .rows_affected();
    }
    for date in &affected_dates {
        sqlx::query("DELETE FROM daily_reports WHERE date = ?1")
            .bind(date)
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "DELETE FROM memory_entries_fts WHERE memory_id IN (SELECT id FROM memory_entries WHERE date = ?1)",
        )
        .bind(date)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("DELETE FROM memory_entries WHERE date = ?1")
            .bind(date)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    Ok(BulkDeletedActivities {
        screenshots,
        deleted_activities,
        affected_dates,
    })
}

pub async fn save_daily_report(pool: &SqlitePool, report: &DailyReport) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO daily_reports (
            date, content, generation_mode, model_name, fallback_reason, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT(date) DO UPDATE SET
            content = excluded.content,
            generation_mode = excluded.generation_mode,
            model_name = excluded.model_name,
            fallback_reason = excluded.fallback_reason,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(&report.date)
    .bind(&report.content)
    .bind(&report.generation_mode)
    .bind(&report.model_name)
    .bind(&report.fallback_reason)
    .bind(format_timestamp(report.created_at)?)
    .bind(format_timestamp(report.updated_at)?)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn load_daily_report(
    pool: &SqlitePool,
    date: &str,
) -> anyhow::Result<Option<DailyReport>> {
    sqlx::query(
        r#"
        SELECT date, content, generation_mode, model_name, fallback_reason, created_at, updated_at
        FROM daily_reports WHERE date = ?1
        "#,
    )
    .bind(date)
    .fetch_optional(pool)
    .await?
    .map(|row| daily_report_from_row(&row))
    .transpose()
}

pub async fn load_all_daily_reports(pool: &SqlitePool) -> anyhow::Result<Vec<DailyReport>> {
    let rows = sqlx::query(
        r#"
        SELECT date, content, generation_mode, model_name, fallback_reason, created_at, updated_at
        FROM daily_reports ORDER BY date ASC
        "#,
    )
    .fetch_all(pool)
    .await?;
    rows.iter().map(daily_report_from_row).collect()
}

pub async fn load_daily_reports_between(
    pool: &SqlitePool,
    start: &str,
    end: &str,
) -> anyhow::Result<Vec<DailyReport>> {
    let rows = sqlx::query(
        r#"
        SELECT date, content, generation_mode, model_name, fallback_reason, created_at, updated_at
        FROM daily_reports WHERE date >= ?1 AND date <= ?2 ORDER BY date ASC
        "#,
    )
    .bind(start)
    .bind(end)
    .fetch_all(pool)
    .await?;
    rows.iter().map(daily_report_from_row).collect()
}

pub async fn upsert_memory_entry(
    pool: &SqlitePool,
    memory_key: &str,
    entry: &MemoryEntry,
) -> anyhow::Result<MemoryEntry> {
    let tags = serde_json::to_string(&entry.tags)?;
    sqlx::query(
        r#"
        INSERT INTO memory_entries (
            id, memory_key, date, source_type, title, content, tags, embedding,
            created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?9)
        ON CONFLICT(memory_key) DO UPDATE SET
            date = excluded.date,
            source_type = excluded.source_type,
            title = excluded.title,
            content = excluded.content,
            tags = excluded.tags,
            embedding = CASE
                WHEN memory_entries.content = excluded.content
                 AND memory_entries.title = excluded.title
                 AND memory_entries.tags = excluded.tags
                THEN memory_entries.embedding ELSE NULL END,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(&entry.id)
    .bind(memory_key)
    .bind(&entry.date)
    .bind(&entry.source_type)
    .bind(&entry.title)
    .bind(&entry.content)
    .bind(tags)
    .bind(format_timestamp(entry.created_at)?)
    .bind(format_timestamp(entry.updated_at)?)
    .execute(pool)
    .await?;

    let row = sqlx::query(
        r#"
        SELECT id, date, source_type, title, content, tags, created_at, updated_at
        FROM memory_entries WHERE memory_key = ?1
        "#,
    )
    .bind(memory_key)
    .fetch_one(pool)
    .await?;
    let stored = memory_entry_from_row(&row)?;

    sqlx::query("DELETE FROM memory_entries_fts WHERE memory_id = ?1")
        .bind(&stored.id)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO memory_entries_fts (memory_id, title, content, tags) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(&stored.id)
    .bind(&stored.title)
    .bind(&stored.content)
    .bind(serde_json::to_string(&stored.tags)?)
    .execute(pool)
    .await?;
    Ok(stored)
}

pub async fn load_recent_memory_entries(
    pool: &SqlitePool,
    limit: i64,
) -> anyhow::Result<Vec<MemoryEntry>> {
    let rows = sqlx::query(
        r#"
        SELECT id, date, source_type, title, content, tags, created_at, updated_at
        FROM memory_entries ORDER BY date DESC, updated_at DESC LIMIT ?1
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(memory_entry_from_row).collect()
}

pub async fn search_memory_entries_fts(
    pool: &SqlitePool,
    query: &str,
    limit: i64,
) -> anyhow::Result<Vec<MemoryEntry>> {
    let fts_query = build_fts_query(query)?;
    let rows = sqlx::query(
        r#"
        SELECT m.id, m.date, m.source_type, m.title, m.content, m.tags,
               m.created_at, m.updated_at, bm25(memory_entries_fts) AS score
        FROM memory_entries_fts
        JOIN memory_entries m ON m.id = memory_entries_fts.memory_id
        WHERE memory_entries_fts MATCH ?1
        ORDER BY score, m.date DESC
        LIMIT ?2
        "#,
    )
    .bind(fts_query)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(memory_entry_from_row).collect()
}

pub async fn load_memory_embeddings(
    pool: &SqlitePool,
) -> anyhow::Result<Vec<(MemoryEntry, Vec<f32>)>> {
    let rows = sqlx::query(
        r#"
        SELECT id, date, source_type, title, content, tags, created_at, updated_at, embedding
        FROM memory_entries WHERE embedding IS NOT NULL
        "#,
    )
    .fetch_all(pool)
    .await?;

    rows.iter()
        .map(|row| {
            let entry = memory_entry_from_row(row)?;
            let raw: String = row.try_get("embedding")?;
            let embedding = serde_json::from_str(&raw)?;
            Ok((entry, embedding))
        })
        .collect()
}

pub async fn load_unembedded_memory_entries(
    pool: &SqlitePool,
    limit: i64,
) -> anyhow::Result<Vec<MemoryEntry>> {
    let rows = sqlx::query(
        r#"
        SELECT id, date, source_type, title, content, tags, created_at, updated_at
        FROM memory_entries WHERE embedding IS NULL ORDER BY updated_at ASC LIMIT ?1
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(memory_entry_from_row).collect()
}

pub async fn set_memory_embedding(
    pool: &SqlitePool,
    memory_id: &str,
    embedding: &[f32],
) -> anyhow::Result<()> {
    sqlx::query("UPDATE memory_entries SET embedding = ?1 WHERE id = ?2")
        .bind(serde_json::to_string(embedding)?)
        .bind(memory_id)
        .execute(pool)
        .await?;
    Ok(())
}

async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS activity_log (
            event_id TEXT PRIMARY KEY,
            ts TEXT NOT NULL,
            device_id TEXT NOT NULL,
            agent_name TEXT NOT NULL,
            platform TEXT NOT NULL,
            kind TEXT NOT NULL,
            app_json TEXT NOT NULL,
            window_title TEXT,
            browser_json TEXT,
            presence TEXT NOT NULL DEFAULT 'active',
            source TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        ALTER TABLE activity_log ADD COLUMN presence TEXT NOT NULL DEFAULT 'active'
        "#,
    )
    .execute(pool)
    .await
    .ok();

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_activity_log_device_ts
        ON activity_log(device_id, ts DESC)
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS device_status (
            device_id TEXT PRIMARY KEY,
            ts TEXT NOT NULL,
            agent_name TEXT NOT NULL,
            platform TEXT NOT NULL,
            status_text TEXT NOT NULL,
            source TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS activity_search (
            event_id TEXT PRIMARY KEY,
            device_id TEXT NOT NULL,
            app_name TEXT NOT NULL,
            window_title TEXT,
            page_title TEXT,
            browser_url TEXT,
            browser_domain TEXT
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_activity_search_device_id
        ON activity_search(device_id)
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE VIRTUAL TABLE IF NOT EXISTS activity_search_fts USING fts5(
            app_name,
            window_title,
            page_title,
            browser_url,
            browser_domain,
            content='activity_search',
            content_rowid='rowid',
            tokenize='unicode61 remove_diacritics 2'
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TRIGGER IF NOT EXISTS activity_search_ai
        AFTER INSERT ON activity_search
        BEGIN
            INSERT INTO activity_search_fts(
                rowid,
                app_name,
                window_title,
                page_title,
                browser_url,
                browser_domain
            )
            VALUES (
                new.rowid,
                new.app_name,
                new.window_title,
                new.page_title,
                new.browser_url,
                new.browser_domain
            );
        END
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TRIGGER IF NOT EXISTS activity_search_ad
        AFTER DELETE ON activity_search
        BEGIN
            INSERT INTO activity_search_fts(
                activity_search_fts,
                rowid,
                app_name,
                window_title,
                page_title,
                browser_url,
                browser_domain
            )
            VALUES (
                'delete',
                old.rowid,
                old.app_name,
                old.window_title,
                old.page_title,
                old.browser_url,
                old.browser_domain
            );
        END
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TRIGGER IF NOT EXISTS activity_search_au
        AFTER UPDATE ON activity_search
        BEGIN
            INSERT INTO activity_search_fts(
                activity_search_fts,
                rowid,
                app_name,
                window_title,
                page_title,
                browser_url,
                browser_domain
            )
            VALUES (
                'delete',
                old.rowid,
                old.app_name,
                old.window_title,
                old.page_title,
                old.browser_url,
                old.browser_domain
            );
            INSERT INTO activity_search_fts(
                rowid,
                app_name,
                window_title,
                page_title,
                browser_url,
                browser_domain
            )
            VALUES (
                new.rowid,
                new.app_name,
                new.window_title,
                new.page_title,
                new.browser_url,
                new.browser_domain
            );
        END
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS screenshots (
            id TEXT PRIMARY KEY,
            event_id TEXT NOT NULL UNIQUE,
            device_id TEXT NOT NULL,
            captured_at TEXT NOT NULL,
            mime_type TEXT NOT NULL,
            byte_size INTEGER NOT NULL,
            width INTEGER,
            height INTEGER,
            sha256 TEXT NOT NULL,
            storage_key TEXT NOT NULL UNIQUE,
            thumbnail_storage_key TEXT,
            visual_hash TEXT,
            ocr_status TEXT NOT NULL,
            ocr_text TEXT,
            ocr_error TEXT,
            ocr_attempts INTEGER NOT NULL DEFAULT 0,
            ocr_attempted_at TEXT,
            duplicate_of TEXT,
            created_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    for statement in [
        "ALTER TABLE screenshots ADD COLUMN thumbnail_storage_key TEXT",
        "ALTER TABLE screenshots ADD COLUMN visual_hash TEXT",
        "ALTER TABLE screenshots ADD COLUMN ocr_attempts INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE screenshots ADD COLUMN ocr_attempted_at TEXT",
        "ALTER TABLE screenshots ADD COLUMN duplicate_of TEXT",
    ] {
        sqlx::query(statement).execute(pool).await.ok();
    }

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_screenshots_device_captured
        ON screenshots(device_id, captured_at DESC)
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE VIRTUAL TABLE IF NOT EXISTS screenshot_search_fts USING fts5(
            screenshot_id UNINDEXED,
            event_id UNINDEXED,
            device_id UNINDEXED,
            ocr_text,
            tokenize='unicode61 remove_diacritics 2'
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS agent_diagnostics (
            device_id TEXT PRIMARY KEY,
            payload_json TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS device_recording_control (
            device_id TEXT PRIMARY KEY,
            desired_enabled INTEGER NOT NULL DEFAULT 1,
            applied_enabled INTEGER,
            revision INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL,
            acknowledged_at TEXT
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS category_rules (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            color TEXT NOT NULL,
            target TEXT NOT NULL CHECK(target IN ('app', 'domain')),
            pattern TEXT NOT NULL,
            priority INTEGER NOT NULL DEFAULT 0
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS work_schedule_segments (
            id TEXT PRIMARY KEY,
            weekday INTEGER NOT NULL CHECK(weekday BETWEEN 1 AND 7),
            start_minute INTEGER NOT NULL CHECK(start_minute BETWEEN 0 AND 1439),
            end_minute INTEGER NOT NULL CHECK(end_minute BETWEEN 1 AND 1440),
            CHECK(start_minute < end_minute)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS review_settings (
            key TEXT PRIMARY KEY,
            value_json TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS assistant_conversations (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS assistant_messages (
            id TEXT PRIMARY KEY,
            conversation_id TEXT NOT NULL,
            role TEXT NOT NULL CHECK(role IN ('user', 'assistant')),
            content TEXT NOT NULL,
            mode TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY(conversation_id) REFERENCES assistant_conversations(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS screenshot_remote_mirror (
            screenshot_id TEXT PRIMARY KEY,
            provider TEXT NOT NULL,
            remote_key TEXT NOT NULL,
            status TEXT NOT NULL,
            error TEXT,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_assistant_messages_conversation ON assistant_messages(conversation_id, created_at)",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS daily_reports (
            date TEXT PRIMARY KEY,
            content TEXT NOT NULL,
            generation_mode TEXT NOT NULL,
            model_name TEXT,
            fallback_reason TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS memory_entries (
            id TEXT PRIMARY KEY,
            memory_key TEXT NOT NULL UNIQUE,
            date TEXT NOT NULL,
            source_type TEXT NOT NULL,
            title TEXT NOT NULL,
            content TEXT NOT NULL,
            tags TEXT NOT NULL,
            embedding TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_memory_entries_date
        ON memory_entries(date DESC, updated_at DESC)
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE VIRTUAL TABLE IF NOT EXISTS memory_entries_fts USING fts5(
            memory_id UNINDEXED,
            title,
            content,
            tags,
            tokenize='unicode61 remove_diacritics 2'
        )
        "#,
    )
    .execute(pool)
    .await?;

    run_data_migrations(pool).await?;

    rebuild_activity_search_index_if_needed(pool).await?;

    Ok(())
}

async fn run_data_migrations(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS eyes_on_me_schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    let timestamp_migration_applied: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM eyes_on_me_schema_migrations WHERE version = 1)",
    )
    .fetch_one(pool)
    .await?;
    if timestamp_migration_applied {
        return Ok(());
    }

    let mut transaction = pool.begin().await?;
    for statement in [
        "UPDATE activity_log SET ts = strftime('%Y-%m-%dT%H:%M:%fZ', ts) WHERE julianday(ts) IS NOT NULL",
        "UPDATE device_status SET ts = strftime('%Y-%m-%dT%H:%M:%fZ', ts) WHERE julianday(ts) IS NOT NULL",
        "UPDATE screenshots SET captured_at = CASE WHEN julianday(captured_at) IS NULL THEN captured_at ELSE strftime('%Y-%m-%dT%H:%M:%fZ', captured_at) END, created_at = CASE WHEN julianday(created_at) IS NULL THEN created_at ELSE strftime('%Y-%m-%dT%H:%M:%fZ', created_at) END, ocr_attempted_at = CASE WHEN julianday(ocr_attempted_at) IS NULL THEN ocr_attempted_at ELSE strftime('%Y-%m-%dT%H:%M:%fZ', ocr_attempted_at) END",
        "UPDATE agent_diagnostics SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', updated_at) WHERE julianday(updated_at) IS NOT NULL",
        "UPDATE device_recording_control SET updated_at = CASE WHEN julianday(updated_at) IS NULL THEN updated_at ELSE strftime('%Y-%m-%dT%H:%M:%fZ', updated_at) END, acknowledged_at = CASE WHEN julianday(acknowledged_at) IS NULL THEN acknowledged_at ELSE strftime('%Y-%m-%dT%H:%M:%fZ', acknowledged_at) END",
        "UPDATE review_settings SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', updated_at) WHERE julianday(updated_at) IS NOT NULL",
        "UPDATE assistant_conversations SET created_at = CASE WHEN julianday(created_at) IS NULL THEN created_at ELSE strftime('%Y-%m-%dT%H:%M:%fZ', created_at) END, updated_at = CASE WHEN julianday(updated_at) IS NULL THEN updated_at ELSE strftime('%Y-%m-%dT%H:%M:%fZ', updated_at) END",
        "UPDATE assistant_messages SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', created_at) WHERE julianday(created_at) IS NOT NULL",
        "UPDATE screenshot_remote_mirror SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', updated_at) WHERE julianday(updated_at) IS NOT NULL",
        "UPDATE daily_reports SET created_at = CASE WHEN julianday(created_at) IS NULL THEN created_at ELSE strftime('%Y-%m-%dT%H:%M:%fZ', created_at) END, updated_at = CASE WHEN julianday(updated_at) IS NULL THEN updated_at ELSE strftime('%Y-%m-%dT%H:%M:%fZ', updated_at) END",
        "UPDATE memory_entries SET created_at = CASE WHEN julianday(created_at) IS NULL THEN created_at ELSE strftime('%Y-%m-%dT%H:%M:%fZ', created_at) END, updated_at = CASE WHEN julianday(updated_at) IS NULL THEN updated_at ELSE strftime('%Y-%m-%dT%H:%M:%fZ', updated_at) END",
    ] {
        sqlx::query(statement).execute(&mut *transaction).await?;
    }
    sqlx::query("INSERT INTO eyes_on_me_schema_migrations (version, applied_at) VALUES (1, ?1)")
        .bind(format_timestamp(OffsetDateTime::now_utc())?)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

async fn upsert_activity_search(pool: &SqlitePool, event: &ActivityEvent) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO activity_search (
            event_id,
            device_id,
            app_name,
            window_title,
            page_title,
            browser_url,
            browser_domain
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT(event_id) DO UPDATE SET
            device_id = excluded.device_id,
            app_name = excluded.app_name,
            window_title = excluded.window_title,
            page_title = excluded.page_title,
            browser_url = excluded.browser_url,
            browser_domain = excluded.browser_domain
        "#,
    )
    .bind(&event.event_id)
    .bind(&event.device_id)
    .bind(&event.app.name)
    .bind(&event.window_title)
    .bind(
        event
            .browser
            .as_ref()
            .and_then(|browser| browser.page_title.clone()),
    )
    .bind(
        event
            .browser
            .as_ref()
            .and_then(|browser| browser.url.clone()),
    )
    .bind(
        event
            .browser
            .as_ref()
            .and_then(|browser| browser.domain.clone()),
    )
    .execute(pool)
    .await?;

    Ok(())
}

async fn rebuild_activity_search_index_if_needed(pool: &SqlitePool) -> anyhow::Result<()> {
    let activity_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM activity_log")
        .fetch_one(pool)
        .await?;
    let search_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM activity_search")
        .fetch_one(pool)
        .await?;

    if activity_count == search_count {
        return Ok(());
    }

    sqlx::query("DELETE FROM activity_search")
        .execute(pool)
        .await?;

    let rows = sqlx::query(
        r#"
        SELECT event_id, ts, device_id, agent_name, platform, kind, app_json, window_title, browser_json, presence, source
        FROM activity_log
        ORDER BY ts ASC
        "#,
    )
    .fetch_all(pool)
    .await?;

    for row in rows {
        let event = activity_from_row(&row)?;
        upsert_activity_search(pool, &event).await?;
    }

    sqlx::query("INSERT INTO activity_search_fts(activity_search_fts) VALUES('rebuild')")
        .execute(pool)
        .await?;

    Ok(())
}

fn build_fts_query(query: &str) -> anyhow::Result<String> {
    let tokens = query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| format!("\"{token}\"*"))
        .collect::<Vec<_>>();

    if tokens.is_empty() {
        bail!("search query is required");
    }

    Ok(tokens.join(" AND "))
}

#[cfg(test)]
mod search_query_tests {
    use super::build_fts_query;

    #[test]
    fn treats_window_title_punctuation_as_separators() {
        assert_eq!(
            build_fts_query("Work_Review - git log").unwrap(),
            "\"Work\"* AND \"Review\"* AND \"git\"* AND \"log\"*"
        );
    }

    #[test]
    fn converts_urls_to_safe_prefix_terms() {
        assert_eq!(
            build_fts_query("https://github.com/wm94i/Work_Review").unwrap(),
            "\"https\"* AND \"github\"* AND \"com\"* AND \"wm94i\"* AND \"Work\"* AND \"Review\"*"
        );
    }

    #[test]
    fn rejects_queries_without_searchable_terms() {
        assert!(build_fts_query("--- / ").is_err());
    }
}

pub(crate) fn format_timestamp(value: OffsetDateTime) -> anyhow::Result<String> {
    let value = value.to_offset(UtcOffset::UTC);
    Ok(format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        value.year(),
        value.month() as u8,
        value.day(),
        value.hour(),
        value.minute(),
        value.second(),
        value.millisecond(),
    ))
}

fn parse_timestamp(value: String) -> anyhow::Result<OffsetDateTime> {
    OffsetDateTime::parse(&value, &time::format_description::well_known::Rfc3339)
        .map_err(Into::into)
}

fn stored_screenshot_from_row(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<StoredScreenshot> {
    let id: String = row.try_get("id")?;
    Ok(StoredScreenshot {
        record: ScreenshotRecord {
            content_url: format!("/api/screenshots/{id}/content"),
            thumbnail_url: format!("/api/screenshots/{id}/thumbnail"),
            id,
            event_id: row.try_get("event_id")?,
            device_id: row.try_get("device_id")?,
            captured_at: parse_timestamp(row.try_get("captured_at")?)?,
            mime_type: row.try_get("mime_type")?,
            byte_size: u64::try_from(row.try_get::<i64, _>("byte_size")?).unwrap_or_default(),
            width: row
                .try_get::<Option<i64>, _>("width")?
                .and_then(|value| u32::try_from(value).ok()),
            height: row
                .try_get::<Option<i64>, _>("height")?
                .and_then(|value| u32::try_from(value).ok()),
            sha256: row.try_get("sha256")?,
            ocr_status: row.try_get("ocr_status")?,
            ocr_text: row.try_get("ocr_text")?,
            ocr_error: row.try_get("ocr_error")?,
            ocr_attempts: u32::try_from(row.try_get::<i64, _>("ocr_attempts")?).unwrap_or_default(),
            duplicate_of: row.try_get("duplicate_of")?,
            created_at: parse_timestamp(row.try_get("created_at")?)?,
        },
        storage_key: row.try_get("storage_key")?,
        thumbnail_storage_key: row.try_get("thumbnail_storage_key")?,
    })
}

fn daily_report_from_row(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<DailyReport> {
    Ok(DailyReport {
        date: row.try_get("date")?,
        content: row.try_get("content")?,
        generation_mode: row.try_get("generation_mode")?,
        model_name: row.try_get("model_name")?,
        fallback_reason: row.try_get("fallback_reason")?,
        created_at: parse_timestamp(row.try_get("created_at")?)?,
        updated_at: parse_timestamp(row.try_get("updated_at")?)?,
    })
}

fn recording_state_from_row(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<DeviceRecordingState> {
    Ok(DeviceRecordingState {
        device_id: row.try_get("device_id")?,
        desired_enabled: row.try_get::<i64, _>("desired_enabled")? != 0,
        applied_enabled: row
            .try_get::<Option<i64>, _>("applied_enabled")?
            .map(|value| value != 0),
        revision: u64::try_from(row.try_get::<i64, _>("revision")?).unwrap_or_default(),
        updated_at: parse_timestamp(row.try_get("updated_at")?)?,
        acknowledged_at: row.try_get("acknowledged_at")?,
    })
}

fn memory_entry_from_row(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<MemoryEntry> {
    let tags: String = row.try_get("tags")?;
    Ok(MemoryEntry {
        id: row.try_get("id")?,
        date: row.try_get("date")?,
        source_type: row.try_get("source_type")?,
        title: row.try_get("title")?,
        content: row.try_get("content")?,
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        score: row
            .try_get::<f64, _>("score")
            .ok()
            .map(|score| score as f32),
        created_at: parse_timestamp(row.try_get("created_at")?)?,
        updated_at: parse_timestamp(row.try_get("updated_at")?)?,
    })
}

fn activity_from_row(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<ActivityEvent> {
    Ok(ActivityEvent {
        event_id: row.try_get("event_id")?,
        ts: OffsetDateTime::parse(
            &row.try_get::<String, _>("ts")?,
            &time::format_description::well_known::Rfc3339,
        )?,
        device_id: row.try_get("device_id")?,
        agent_name: row.try_get("agent_name")?,
        platform: platform_from_str(&row.try_get::<String, _>("platform")?),
        kind: kind_from_str(&row.try_get::<String, _>("kind")?),
        app: serde_json::from_str::<ActivityApp>(&row.try_get::<String, _>("app_json")?)?,
        window_title: row.try_get("window_title")?,
        browser: parse_optional_json(row.try_get::<Option<String>, _>("browser_json")?)?,
        presence: presence_from_str(&row.try_get::<String, _>("presence")?),
        source: row.try_get("source")?,
    })
}

fn status_from_row(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<DeviceStatus> {
    Ok(DeviceStatus {
        ts: OffsetDateTime::parse(
            &row.try_get::<String, _>("ts")?,
            &time::format_description::well_known::Rfc3339,
        )?,
        device_id: row.try_get("device_id")?,
        agent_name: row.try_get("agent_name")?,
        platform: platform_from_str(&row.try_get::<String, _>("platform")?),
        status_text: row.try_get("status_text")?,
        source: row.try_get("source")?,
    })
}

fn platform_to_str(platform: &Platform) -> &'static str {
    match platform {
        Platform::Macos => "macos",
        Platform::Windows => "windows",
        Platform::Linux => "linux",
        Platform::Android => "android",
        Platform::Unknown => "unknown",
    }
}

fn platform_from_str(value: &str) -> Platform {
    match value {
        "macos" => Platform::Macos,
        "windows" => Platform::Windows,
        "linux" => Platform::Linux,
        "android" => Platform::Android,
        _ => Platform::Unknown,
    }
}

fn kind_to_str(kind: &ActivityKind) -> &'static str {
    match kind {
        ActivityKind::ForegroundChanged => "foreground_changed",
        ActivityKind::ActivitySample => "activity_sample",
        ActivityKind::PresenceChanged => "presence_changed",
    }
}

fn kind_from_str(value: &str) -> ActivityKind {
    match value {
        "foreground_changed" => ActivityKind::ForegroundChanged,
        "activity_sample" => ActivityKind::ActivitySample,
        "presence_changed" => ActivityKind::PresenceChanged,
        _ => ActivityKind::ForegroundChanged,
    }
}

fn presence_to_str(value: PresenceState) -> &'static str {
    match value {
        PresenceState::Active => "active",
        PresenceState::Idle => "idle",
        PresenceState::Locked => "locked",
    }
}

fn presence_from_str(value: &str) -> PresenceState {
    match value {
        "idle" => PresenceState::Idle,
        "locked" => PresenceState::Locked,
        _ => PresenceState::Active,
    }
}

fn parse_optional_json<T>(value: Option<String>) -> anyhow::Result<Option<T>>
where
    T: serde::de::DeserializeOwned,
{
    value
        .map(|raw| serde_json::from_str::<T>(&raw).map_err(anyhow::Error::from))
        .transpose()
}

async fn ensure_parent_dir(path: &str) -> anyhow::Result<()> {
    let file_path = Path::new(path);
    if let Some(parent) = file_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use eyes_on_me_shared::{ActivityKind, BrowserContext, Platform};
    use time::{OffsetDateTime, format_description::well_known::Rfc3339};

    use super::{
        connect, count_activity_search_results, load_analysis_activities_for_device,
        persist_activity, run_data_migrations, search_activities,
    };

    fn sample_event(
        event_id: &str,
        device_id: &str,
        title: &str,
        url: Option<&str>,
    ) -> eyes_on_me_shared::ActivityEvent {
        eyes_on_me_shared::ActivityEvent {
            event_id: event_id.to_string(),
            ts: OffsetDateTime::now_utc(),
            device_id: device_id.to_string(),
            agent_name: "client-desktop".to_string(),
            platform: Platform::Macos,
            kind: ActivityKind::ForegroundChanged,
            app: eyes_on_me_shared::ActivityApp {
                id: "com.google.Chrome".to_string(),
                name: "Google Chrome".to_string(),
                title: Some(title.to_string()),
                pid: Some(42),
            },
            window_title: Some(title.to_string()),
            browser: url.map(|url| BrowserContext {
                family: "chromium".to_string(),
                name: "Google Chrome".to_string(),
                page_title: Some(title.to_string()),
                url: Some(url.to_string()),
                domain: Some("github.com".to_string()),
                source: "test".to_string(),
                confidence: 0.9,
            }),
            presence: eyes_on_me_shared::PresenceState::Active,
            source: "desktop".to_string(),
        }
    }

    #[tokio::test]
    async fn searches_window_titles_and_urls() {
        let pool = connect("sqlite::memory:")
            .await
            .expect("connect in-memory db");
        persist_activity(
            &pool,
            &sample_event(
                "evt-1",
                "mac-1",
                "GitHub Pull Request Review",
                Some("https://github.com/wm94i/Work_Review/pull/10"),
            ),
        )
        .await
        .expect("persist search event");

        let results = search_activities(&pool, "github review", None, 10)
            .await
            .expect("search activities");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].activity.event_id, "evt-1");
    }

    #[tokio::test]
    async fn filters_search_results_by_device() {
        let pool = connect("sqlite::memory:")
            .await
            .expect("connect in-memory db");
        persist_activity(
            &pool,
            &sample_event(
                "evt-1",
                "mac-1",
                "Rust RFC",
                Some("https://github.com/rust-lang/rfcs"),
            ),
        )
        .await
        .expect("persist first event");
        persist_activity(
            &pool,
            &sample_event(
                "evt-2",
                "mac-2",
                "Rust RFC",
                Some("https://github.com/rust-lang/rfcs"),
            ),
        )
        .await
        .expect("persist second event");

        let total = count_activity_search_results(&pool, "rust", Some("mac-2"))
            .await
            .expect("count filtered search");
        let results = search_activities(&pool, "rust", Some("mac-2"), 10)
            .await
            .expect("search filtered activities");

        assert_eq!(total, 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].activity.device_id, "mac-2");
    }

    #[tokio::test]
    async fn normalizes_offsets_for_indexed_time_range_queries() {
        let pool = connect("sqlite::memory:")
            .await
            .expect("connect in-memory db");
        let mut first = sample_event("offset-1", "mac-offset", "First tab", None);
        first.ts = OffsetDateTime::parse("2026-07-29T00:00:00.125+08:00", &Rfc3339)
            .expect("parse first timestamp");
        let mut second = sample_event("offset-2", "mac-offset", "Second tab", None);
        second.ts = OffsetDateTime::parse("2026-07-29T00:00:00.875+08:00", &Rfc3339)
            .expect("parse second timestamp");
        persist_activity(&pool, &first)
            .await
            .expect("persist first offset event");
        persist_activity(&pool, &second)
            .await
            .expect("persist second offset event");

        let cutoff = OffsetDateTime::parse("2026-07-29T00:00:00+08:00", &Rfc3339)
            .expect("parse local-day cutoff");
        let activities = load_analysis_activities_for_device(&pool, "mac-offset", Some(cutoff))
            .await
            .expect("load analysis range");
        let stored = sqlx::query_scalar::<_, String>(
            "SELECT ts FROM activity_log WHERE device_id = ?1 ORDER BY ts ASC",
        )
        .bind("mac-offset")
        .fetch_all(&pool)
        .await
        .expect("load normalized timestamps");

        assert_eq!(
            stored,
            ["2026-07-28T16:00:00.125Z", "2026-07-28T16:00:00.875Z"]
        );
        assert_eq!(activities.len(), 2);
        assert_eq!(activities[0].event_id, "offset-1");
        assert_eq!(activities[1].event_id, "offset-2");

        sqlx::query("UPDATE activity_log SET ts = '2026-07-29T00:00:00.125+08:00' WHERE event_id = 'offset-1'")
            .execute(&pool)
            .await
            .expect("restore a legacy offset timestamp");
        sqlx::query("DELETE FROM eyes_on_me_schema_migrations WHERE version = 1")
            .execute(&pool)
            .await
            .expect("reset timestamp migration marker");
        run_data_migrations(&pool)
            .await
            .expect("migrate legacy timestamps");
        let migrated = sqlx::query_scalar::<_, String>(
            "SELECT ts FROM activity_log WHERE event_id = 'offset-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("load migrated timestamp");
        assert_eq!(migrated, "2026-07-28T16:00:00.125Z");
    }
}
