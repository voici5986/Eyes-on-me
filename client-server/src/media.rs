use std::path::{Component, Path, PathBuf};

use anyhow::{Context, bail};
use eyes_on_me_shared::{MediaCleanupResponse, MediaStatus, ScreenshotRecord};
use image::{DynamicImage, GenericImageView, ImageFormat, ImageReader, codecs::jpeg::JpegEncoder};
use sha2::{Digest, Sha256};
use time::{Duration as TimeDuration, OffsetDateTime};
use tokio::{fs, io::AsyncWriteExt, process::Command};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::{app_state::AppState, db::StoredScreenshot};

const MAX_IMAGE_PIXELS: u64 = 50_000_000;
const THUMBNAIL_WIDTH: u32 = 640;
const THUMBNAIL_HEIGHT: u32 = 360;
const SIMILARITY_DISTANCE: u32 = 8;

pub async fn store_screenshot(
    state: &AppState,
    event_id: &str,
    declared_mime: Option<&str>,
    bytes: &[u8],
) -> anyhow::Result<ScreenshotRecord> {
    if bytes.is_empty() {
        bail!("screenshot body is empty");
    }
    if bytes.len() > state.media_max_bytes() {
        bail!("screenshot exceeds configured byte limit");
    }

    let identity = crate::db::load_activity_identity(&state.pool(), event_id)
        .await?
        .context("activity event does not exist")?;
    if let Some(stored) = crate::db::load_screenshot_for_event(&state.pool(), event_id).await? {
        return Ok(stored.record);
    }
    let (format, mime_type, extension) = detect_format(bytes, declared_mime)?;
    let image = ImageReader::with_format(std::io::Cursor::new(bytes), format)
        .decode()
        .context("invalid or corrupt screenshot")?;
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        bail!("screenshot dimensions are outside the allowed range");
    }

    let digest = format!("{:x}", Sha256::digest(bytes));
    let visual_hash = visual_hash(&image);
    let duplicate = find_similar_ocr(state, &identity.device_id, &visual_hash).await?;
    let id = Uuid::new_v4().to_string();
    let day = sanitize_segment(&identity.ts.date().to_string());
    let storage_key = format!("{day}/{id}.{extension}");
    let thumbnail_storage_key = format!("thumbnails/{day}/{id}.jpg");
    let thumbnail = encode_thumbnail(&image)?;
    write_atomically(state.media_dir(), &storage_key, bytes).await?;
    if let Err(error) =
        write_atomically(state.media_dir(), &thumbnail_storage_key, &thumbnail).await
    {
        let _ = fs::remove_file(resolve_storage_path(state.media_dir(), &storage_key)?).await;
        return Err(error);
    }

    let now = OffsetDateTime::now_utc();
    let (ocr_status, ocr_text, duplicate_of, ocr_attempts) =
        if let Some((duplicate_id, text)) = duplicate {
            ("complete", Some(text), Some(duplicate_id), 0)
        } else if state.ocr_command().is_some() {
            ("pending", None, None, 1)
        } else {
            ("unavailable", None, None, 0)
        };
    let record = ScreenshotRecord {
        id: id.clone(),
        event_id: event_id.to_string(),
        device_id: identity.device_id,
        captured_at: identity.ts,
        mime_type: mime_type.to_string(),
        byte_size: bytes.len() as u64,
        width: Some(width),
        height: Some(height),
        sha256: digest,
        ocr_status: ocr_status.to_string(),
        ocr_text,
        ocr_error: None,
        ocr_attempts,
        duplicate_of,
        content_url: format!("/api/screenshots/{id}/content"),
        thumbnail_url: format!("/api/screenshots/{id}/thumbnail"),
        created_at: now,
    };

    if let Err(error) = crate::db::insert_screenshot(
        &state.pool(),
        &record,
        &storage_key,
        Some(&thumbnail_storage_key),
        &visual_hash,
    )
    .await
    {
        let _ = fs::remove_file(resolve_storage_path(state.media_dir(), &storage_key)?).await;
        let _ = fs::remove_file(resolve_storage_path(
            state.media_dir(),
            &thumbnail_storage_key,
        )?)
        .await;
        return Err(error);
    }

    if record.ocr_status == "complete" {
        crate::db::update_screenshot_ocr(
            &state.pool(),
            &id,
            "complete",
            record.ocr_text.as_deref(),
            None,
        )
        .await?;
    } else if record.ocr_status == "pending" {
        spawn_ocr(state.clone(), id, storage_key.clone());
    }
    if let Err(error) =
        crate::remote::queue_upload(state, &record.id, &storage_key, &record.mime_type).await
    {
        warn!(screenshot_id = record.id, error = %error, "failed to queue remote mirror");
    }
    Ok(record)
}

pub async fn read_screenshot(
    state: &AppState,
    screenshot_id: &str,
) -> anyhow::Result<Option<(String, Vec<u8>)>> {
    let Some(stored) = crate::db::load_screenshot(&state.pool(), screenshot_id).await? else {
        return Ok(None);
    };
    let path = resolve_storage_path(state.media_dir(), &stored.storage_key)?;
    Ok(Some((stored.record.mime_type, fs::read(path).await?)))
}

pub async fn read_thumbnail(
    state: &AppState,
    screenshot_id: &str,
) -> anyhow::Result<Option<Vec<u8>>> {
    let Some(stored) = crate::db::load_screenshot(&state.pool(), screenshot_id).await? else {
        return Ok(None);
    };
    if let Some(key) = stored.thumbnail_storage_key {
        return Ok(Some(
            fs::read(resolve_storage_path(state.media_dir(), &key)?).await?,
        ));
    }

    let original = fs::read(resolve_storage_path(
        state.media_dir(),
        &stored.storage_key,
    )?)
    .await?;
    let image = image::load_from_memory(&original).context("legacy screenshot is invalid")?;
    let thumbnail = encode_thumbnail(&image)?;
    let day = sanitize_segment(&stored.record.captured_at.date().to_string());
    let key = format!("thumbnails/{day}/{}.jpg", stored.record.id);
    write_atomically(state.media_dir(), &key, &thumbnail).await?;
    if let Err(error) =
        crate::db::set_screenshot_thumbnail_key(&state.pool(), &stored.record.id, &key).await
    {
        let _ = fs::remove_file(resolve_storage_path(state.media_dir(), &key)?).await;
        return Err(error);
    }
    Ok(Some(thumbnail))
}

pub async fn retry_ocr(state: &AppState, screenshot_id: &str) -> anyhow::Result<ScreenshotRecord> {
    if state.ocr_command().is_none() {
        bail!("OCR is disabled");
    }
    let stored = crate::db::load_screenshot(&state.pool(), screenshot_id)
        .await?
        .context("screenshot does not exist")?;
    if !crate::db::mark_screenshot_ocr_pending(&state.pool(), screenshot_id).await? {
        bail!("screenshot does not exist");
    }
    spawn_ocr(state.clone(), screenshot_id.to_string(), stored.storage_key);
    crate::db::load_screenshot(&state.pool(), screenshot_id)
        .await?
        .map(|item| item.record)
        .context("screenshot disappeared")
}

pub async fn delete_screenshot(state: &AppState, screenshot_id: &str) -> anyhow::Result<bool> {
    if let Some(remote) = crate::db::load_remote_mirror_record(&state.pool(), screenshot_id).await?
    {
        crate::remote::delete_remote_key(state, &remote.remote_key).await?;
    }
    let Some(stored) = crate::db::delete_screenshot_row(&state.pool(), screenshot_id).await? else {
        return Ok(false);
    };
    delete_stored_files(state.media_dir().to_path_buf(), stored).await;
    Ok(true)
}

pub async fn delete_activity(state: &AppState, event_id: &str) -> anyhow::Result<bool> {
    let Some(activity) = crate::db::load_activity(&state.pool(), event_id).await? else {
        return Ok(false);
    };
    if let Some(stored) = crate::db::load_screenshot_for_event(&state.pool(), event_id).await?
        && let Some(remote) =
            crate::db::load_remote_mirror_record(&state.pool(), &stored.record.id).await?
    {
        crate::remote::delete_remote_key(state, &remote.remote_key).await?;
    }
    let deleted = crate::db::delete_activity_rows(&state.pool(), &[activity]).await?;
    for stored in deleted.screenshots {
        delete_stored_files(state.media_dir().to_path_buf(), stored).await;
    }
    state.reload_snapshot().await?;
    Ok(true)
}

pub async fn status(state: &AppState) -> anyhow::Result<MediaStatus> {
    let stored = crate::db::load_all_stored_screenshots(&state.pool()).await?;
    let media_dir = state.media_dir().to_path_buf();
    let mut thumbnail_bytes = 0u64;
    for item in &stored {
        if let Some(key) = item.thumbnail_storage_key.clone()
            && let Ok(metadata) = fs::metadata(resolve_storage_path(&media_dir, &key)?).await
        {
            thumbnail_bytes = thumbnail_bytes.saturating_add(metadata.len());
        }
    }
    let total_bytes = stored.iter().map(|item| item.record.byte_size).sum();
    Ok(MediaStatus {
        screenshot_count: stored.len() as u64,
        total_bytes,
        thumbnail_bytes,
        total_limit_bytes: state.media_total_max_bytes(),
        retention_days: state.media_retention_days(),
        pending_ocr: stored
            .iter()
            .filter(|item| item.record.ocr_status == "pending")
            .count() as u64,
        failed_ocr: stored
            .iter()
            .filter(|item| item.record.ocr_status == "failed")
            .count() as u64,
        oldest_capture: stored
            .first()
            .map(|item| item.record.captured_at.to_string()),
        newest_capture: stored
            .last()
            .map(|item| item.record.captured_at.to_string()),
    })
}

pub async fn cleanup(state: &AppState) -> anyhow::Result<MediaCleanupResponse> {
    let pool = state.pool();
    let media_dir = state.media_dir().to_path_buf();
    let retention_days = state.media_retention_days();
    let total_max_bytes = state.media_total_max_bytes();
    let stored = crate::db::load_all_stored_screenshots(&pool).await?;
    let cutoff = OffsetDateTime::now_utc() - TimeDuration::days(i64::from(retention_days));
    let mut remaining_bytes: u64 = stored.iter().map(|item| item.record.byte_size).sum();
    let mut deleted = 0u64;
    let mut freed = 0u64;
    for item in stored {
        let expired = item.record.captured_at < cutoff;
        let over_capacity = remaining_bytes > total_max_bytes;
        if !(expired || over_capacity) {
            continue;
        }
        let bytes = item.record.byte_size;
        if let Some(remote) = crate::db::load_remote_mirror_record(&pool, &item.record.id).await?
            && let Err(error) = crate::remote::delete_remote_key(state, &remote.remote_key).await
        {
            warn!(screenshot_id = item.record.id, error = %error, "retention skipped remote-mirrored screenshot");
            continue;
        }
        if let Some(removed) = crate::db::delete_screenshot_row(&pool, &item.record.id).await? {
            delete_stored_files(media_dir.clone(), removed).await;
            deleted += 1;
            freed = freed.saturating_add(bytes);
            remaining_bytes = remaining_bytes.saturating_sub(bytes);
        }
    }
    Ok(MediaCleanupResponse {
        deleted_screenshots: deleted,
        freed_bytes: freed,
        status: status(state).await?,
    })
}

pub fn start_maintenance(state: std::sync::Arc<AppState>) {
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
        loop {
            timer.tick().await;
            match cleanup(&state).await {
                Ok(result) if result.deleted_screenshots > 0 => info!(
                    deleted = result.deleted_screenshots,
                    freed_bytes = result.freed_bytes,
                    "media retention cleanup complete"
                ),
                Ok(_) => {}
                Err(error) => error!(error = %error, "media retention cleanup failed"),
            }
        }
    });
}

fn spawn_ocr(state: AppState, screenshot_id: String, storage_key: String) {
    tokio::spawn(async move {
        let semaphore = state.ocr_semaphore();
        let permit = match semaphore.acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => return,
        };
        let result = run_ocr(&state, &storage_key)
            .await
            .map(|text| redact_sensitive_text(&text, state.ocr_redact_terms()));
        drop(permit);
        let update = match result {
            Ok(text) => {
                let text = text.trim().to_string();
                crate::db::update_screenshot_ocr(
                    &state.pool(),
                    &screenshot_id,
                    "complete",
                    (!text.is_empty()).then_some(text.as_str()),
                    None,
                )
                .await
            }
            Err(error) => {
                warn!(screenshot_id, error = %error, "screenshot OCR failed");
                crate::db::update_screenshot_ocr(
                    &state.pool(),
                    &screenshot_id,
                    "failed",
                    None,
                    Some(&error.to_string()),
                )
                .await
            }
        };
        if let Err(error) = update {
            error!(screenshot_id, error = %error, "failed to store screenshot OCR state");
        }
    });
}

async fn run_ocr(state: &AppState, storage_key: &str) -> anyhow::Result<String> {
    let command = state.ocr_command().context("OCR is disabled")?;
    let bytes = fs::read(resolve_storage_path(state.media_dir(), storage_key)?)
        .await
        .context("failed to read screenshot for OCR")?;
    let mut child = Command::new(command)
        .arg("stdin")
        .arg("stdout")
        .arg("-l")
        .arg(state.ocr_language())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to run OCR command {command}"))?;
    let mut stdin = child.stdin.take().context("failed to open OCR stdin")?;
    stdin.write_all(&bytes).await?;
    drop(stdin);
    let output = child.wait_with_output().await?;
    if !output.status.success() {
        bail!(
            "OCR exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout).context("OCR output was not UTF-8")
}

async fn find_similar_ocr(
    state: &AppState,
    device_id: &str,
    hash: &str,
) -> anyhow::Result<Option<(String, String)>> {
    let current = u64::from_str_radix(hash, 16)?;
    for (id, candidate_hash, text) in
        crate::db::load_recent_ocr_candidates(&state.pool(), device_id, 120).await?
    {
        if let Ok(candidate) = u64::from_str_radix(&candidate_hash, 16)
            && (current ^ candidate).count_ones() <= SIMILARITY_DISTANCE
        {
            return Ok(Some((id, text)));
        }
    }
    Ok(None)
}

fn visual_hash(image: &DynamicImage) -> String {
    let gray = image
        .resize_exact(9, 8, image::imageops::FilterType::Triangle)
        .to_luma8();
    let mut hash = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            hash <<= 1;
            if gray.get_pixel(x, y)[0] > gray.get_pixel(x + 1, y)[0] {
                hash |= 1;
            }
        }
    }
    format!("{hash:016x}")
}

fn encode_thumbnail(image: &DynamicImage) -> anyhow::Result<Vec<u8>> {
    let thumb = image.thumbnail(THUMBNAIL_WIDTH, THUMBNAIL_HEIGHT);
    let mut bytes = Vec::new();
    JpegEncoder::new_with_quality(&mut bytes, 78).encode_image(&thumb)?;
    Ok(bytes)
}

fn redact_sensitive_text(text: &str, terms: &[String]) -> String {
    if terms.is_empty() {
        return text.to_string();
    }
    text.lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if terms.iter().any(|term| lower.contains(term)) {
                "[redacted]"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) async fn delete_stored_files(media_dir: PathBuf, stored: StoredScreenshot) {
    let keys = [Some(stored.storage_key), stored.thumbnail_storage_key]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    for key in keys {
        match resolve_storage_path(&media_dir, &key) {
            Ok(path) => {
                if let Err(error) = fs::remove_file(path).await
                    && error.kind() != std::io::ErrorKind::NotFound
                {
                    warn!(error = %error, "failed to remove media file");
                }
            }
            Err(error) => warn!(error = %error, "invalid stored media path"),
        }
    }
}

fn detect_format(
    bytes: &[u8],
    declared_mime: Option<&str>,
) -> anyhow::Result<(ImageFormat, &'static str, &'static str)> {
    let format = image::guess_format(bytes).context("unsupported screenshot format")?;
    let (mime, extension) = match format {
        ImageFormat::Png => ("image/png", "png"),
        ImageFormat::Jpeg => ("image/jpeg", "jpg"),
        _ => bail!("only PNG and JPEG screenshots are accepted"),
    };
    if let Some(declared) = declared_mime {
        let declared = declared.split(';').next().unwrap_or_default().trim();
        if !declared.is_empty()
            && declared != mime
            && !(mime == "image/jpeg" && declared == "image/jpg")
        {
            bail!("declared content type does not match screenshot bytes");
        }
    }
    Ok((format, mime, extension))
}

async fn write_atomically(root: &Path, storage_key: &str, bytes: &[u8]) -> anyhow::Result<()> {
    let destination = resolve_storage_path(root, storage_key)?;
    let parent = destination.parent().context("invalid media destination")?;
    fs::create_dir_all(parent).await?;
    let temporary = destination.with_extension(format!(
        "{}.{}.tmp",
        destination
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("image"),
        Uuid::new_v4()
    ));
    fs::write(&temporary, bytes).await?;
    fs::rename(&temporary, &destination).await?;
    Ok(())
}

fn resolve_storage_path(root: &Path, storage_key: &str) -> anyhow::Result<PathBuf> {
    let relative = Path::new(storage_key);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("invalid media storage key");
    }
    Ok(root.join(relative))
}

fn sanitize_segment(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{detect_format, redact_sensitive_text, visual_hash};
    use image::{DynamicImage, ImageFormat, RgbImage};

    #[test]
    fn validates_image_magic_and_declared_type() {
        let mut bytes = Vec::new();
        DynamicImage::ImageRgb8(RgbImage::new(2, 2))
            .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        assert!(detect_format(&bytes, Some("image/png")).is_ok());
        assert!(detect_format(&bytes, Some("image/jpeg")).is_err());
    }

    #[test]
    fn hashes_and_redacts_ocr() {
        assert_eq!(
            visual_hash(&DynamicImage::ImageRgb8(RgbImage::new(20, 20))).len(),
            16
        );
        assert_eq!(
            redact_sensitive_text("safe\nSecret token", &["secret".into()]),
            "safe\n[redacted]"
        );
    }
}
