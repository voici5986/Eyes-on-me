use std::{
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};

use amiokay_shared::{
    ActivityApp, ActivityEvent, ActivityKind, DashboardSnapshot, DeviceStatus, Platform,
    PresenceState,
};
use anyhow::Context;
use sqlx::{
    ConnectOptions, Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use time::OffsetDateTime;

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
    .bind(event.ts.format(&time::format_description::well_known::Rfc3339)?)
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
    .bind(
        status
            .ts
            .format(&time::format_description::well_known::Rfc3339)?,
    )
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

    Ok(())
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
