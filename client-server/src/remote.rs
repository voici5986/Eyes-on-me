use anyhow::{Context, bail, ensure};
use hmac::{Hmac, Mac};
use reqwest::{Client, Method, Url};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use tracing::{error, info};

use crate::{app_state::AppState, config::RemoteStorageConfig};

type HmacSha256 = Hmac<Sha256>;

pub async fn queue_upload(
    state: &AppState,
    screenshot_id: &str,
    storage_key: &str,
    mime_type: &str,
) -> anyhow::Result<()> {
    let Some(config) = state.remote_storage_config() else {
        return Ok(());
    };
    let provider = provider_name(config);
    let destination_key = remote_key(config, storage_key);
    crate::db::set_remote_mirror_status(
        &state.pool(),
        screenshot_id,
        provider,
        &destination_key,
        "pending",
        None,
    )
    .await?;
    let state = state.clone();
    let screenshot_id = screenshot_id.to_string();
    let storage_key = storage_key.to_string();
    let mime_type = mime_type.to_string();
    tokio::spawn(async move {
        let result = upload(&state, &storage_key, &mime_type).await;
        let (status, upload_error) = match &result {
            Ok(url) => {
                info!(screenshot_id, remote_url = %url, "remote screenshot mirror complete");
                ("complete", None)
            }
            Err(error) => {
                error!(screenshot_id, error = %error, "remote screenshot mirror failed");
                ("failed", Some(error.to_string()))
            }
        };
        if let Some(config) = state.remote_storage_config() {
            let _ = crate::db::set_remote_mirror_status(
                &state.pool(),
                &screenshot_id,
                provider_name(config),
                &remote_key(config, &storage_key),
                status,
                upload_error.as_deref(),
            )
            .await;
        }
    });
    Ok(())
}

pub async fn upload(
    state: &AppState,
    storage_key: &str,
    mime_type: &str,
) -> anyhow::Result<String> {
    let config = state
        .remote_storage_config()
        .context("remote storage is not configured")?;
    let bytes = tokio::fs::read(state.media_dir().join(storage_key)).await?;
    let key = remote_key(config, storage_key);
    match config {
        RemoteStorageConfig::WebDav {
            url,
            username,
            password,
            ..
        } => {
            webdav_request(
                Method::PUT,
                url,
                username,
                password,
                &key,
                Some((mime_type, bytes)),
            )
            .await
        }
        RemoteStorageConfig::S3 {
            endpoint,
            bucket,
            region,
            access_key,
            secret_key,
            ..
        } => {
            s3_request(
                Method::PUT,
                endpoint,
                bucket,
                region,
                access_key,
                secret_key,
                &key,
                mime_type,
                &bytes,
            )
            .await
        }
    }
}

pub async fn delete_remote_key(state: &AppState, remote_key: &str) -> anyhow::Result<()> {
    let config = state
        .remote_storage_config()
        .context("remote storage credentials are required to delete the mirrored object")?;
    match config {
        RemoteStorageConfig::WebDav {
            url,
            username,
            password,
            ..
        } => {
            webdav_request(Method::DELETE, url, username, password, remote_key, None).await?;
        }
        RemoteStorageConfig::S3 {
            endpoint,
            bucket,
            region,
            access_key,
            secret_key,
            ..
        } => {
            s3_request(
                Method::DELETE,
                endpoint,
                bucket,
                region,
                access_key,
                secret_key,
                remote_key,
                "application/octet-stream",
                &[],
            )
            .await?;
        }
    }
    Ok(())
}

pub async fn status(state: &AppState) -> anyhow::Result<eyes_on_me_shared::RemoteMirrorStatus> {
    let config = state.remote_storage_config();
    crate::db::load_remote_mirror_status(
        &state.pool(),
        config.is_some(),
        config.map(provider_name).map(str::to_string),
    )
    .await
}

pub async fn retry_pending(state: &AppState) -> anyhow::Result<u64> {
    if state.remote_storage_config().is_none() {
        return Ok(0);
    }
    let items = crate::db::load_retryable_remote_mirrors(&state.pool(), 100).await?;
    let count = items.len() as u64;
    for (screenshot_id, storage_key, mime_type) in items {
        queue_upload(state, &screenshot_id, &storage_key, &mime_type).await?;
    }
    Ok(count)
}

pub fn start_maintenance(state: std::sync::Arc<AppState>) {
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(5 * 60));
        loop {
            timer.tick().await;
            if let Err(error) = retry_pending(&state).await {
                error!(error = %error, "remote mirror retry failed");
            }
        }
    });
}

fn provider_name(config: &RemoteStorageConfig) -> &'static str {
    match config {
        RemoteStorageConfig::WebDav { .. } => "webdav",
        RemoteStorageConfig::S3 { .. } => "s3",
    }
}

fn remote_key(config: &RemoteStorageConfig, storage_key: &str) -> String {
    let prefix = match config {
        RemoteStorageConfig::WebDav { prefix, .. } | RemoteStorageConfig::S3 { prefix, .. } => {
            prefix
        }
    };
    [prefix.trim_matches('/'), storage_key.trim_matches('/')]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

async fn webdav_request(
    method: Method,
    base_url: &str,
    username: &str,
    password: &str,
    object_key: &str,
    body: Option<(&str, Vec<u8>)>,
) -> anyhow::Result<String> {
    validate_remote_endpoint(base_url)?;
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(45))
        .build()?;
    if method == Method::PUT {
        let mut current = base_url.trim_end_matches('/').to_string();
        let segments = object_key.split('/').collect::<Vec<_>>();
        for segment in segments.iter().take(segments.len().saturating_sub(1)) {
            current.push('/');
            current.push_str(&encode_segment(segment));
            let response = client
                .request(Method::from_bytes(b"MKCOL")?, &current)
                .basic_auth(username, Some(password))
                .send()
                .await?;
            if !(response.status().is_success() || response.status().as_u16() == 405) {
                bail!("WebDAV MKCOL returned HTTP {}", response.status());
            }
        }
    }
    let url = format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        object_key
            .split('/')
            .map(encode_segment)
            .collect::<Vec<_>>()
            .join("/")
    );
    let mut request = client
        .request(method.clone(), &url)
        .basic_auth(username, Some(password));
    if let Some((mime, bytes)) = body {
        request = request
            .header(reqwest::header::CONTENT_TYPE, mime)
            .body(bytes);
    }
    let response = request.send().await?;
    if method == Method::DELETE && response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(url);
    }
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        bail!(
            "WebDAV request returned HTTP {status}: {}",
            detail.chars().take(300).collect::<String>()
        );
    }
    Ok(url)
}

#[allow(clippy::too_many_arguments)]
async fn s3_request(
    method: Method,
    endpoint: &str,
    bucket: &str,
    region: &str,
    access_key: &str,
    secret_key: &str,
    object_key: &str,
    content_type: &str,
    bytes: &[u8],
) -> anyhow::Result<String> {
    validate_remote_endpoint(endpoint)?;
    let encoded_key = object_key
        .split('/')
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/");
    let url = format!(
        "{}/{}/{}",
        endpoint.trim_end_matches('/'),
        encode_segment(bucket),
        encoded_key
    );
    let parsed = Url::parse(&url)?;
    let host = parsed.host_str().context("S3 endpoint has no host")?;
    let host = parsed
        .port()
        .map(|port| format!("{host}:{port}"))
        .unwrap_or_else(|| host.to_string());
    let now = OffsetDateTime::now_utc();
    let date_stamp = now.format(&time::format_description::parse("[year][month][day]")?)?;
    let amz_date = now.format(&time::format_description::parse(
        "[year][month][day]T[hour][minute][second]Z",
    )?)?;
    let payload_hash = hex::encode(Sha256::digest(bytes));
    let canonical_uri = format!("/{}/{}", encode_segment(bucket), encoded_key);
    let canonical_headers = format!(
        "content-type:{content_type}\nhost:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amz_date}\n"
    );
    let signed_headers = "content-type;host;x-amz-content-sha256;x-amz-date";
    let canonical_request = format!(
        "{}\n{}\n\n{}\n{}\n{}",
        method.as_str(),
        canonical_uri,
        canonical_headers,
        signed_headers,
        payload_hash
    );
    let scope = format!("{date_stamp}/{region}/s3/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        hex::encode(Sha256::digest(canonical_request.as_bytes()))
    );
    let signature = hex::encode(hmac_sha256(
        &signing_key(secret_key, &date_stamp, region),
        string_to_sign.as_bytes(),
    ));
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
    );
    let response = Client::builder()
        .timeout(std::time::Duration::from_secs(45))
        .build()?
        .request(method.clone(), &url)
        .header(reqwest::header::CONTENT_TYPE, content_type)
        .header(reqwest::header::HOST, host)
        .header("x-amz-content-sha256", payload_hash)
        .header("x-amz-date", amz_date)
        .header(reqwest::header::AUTHORIZATION, authorization)
        .body(bytes.to_vec())
        .send()
        .await?;
    if method == Method::DELETE && response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(url);
    }
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        bail!(
            "S3 request returned HTTP {status}: {}",
            detail.chars().take(300).collect::<String>()
        );
    }
    Ok(url)
}

fn signing_key(secret_key: &str, date: &str, region: &str) -> Vec<u8> {
    let date_key = hmac_sha256(format!("AWS4{secret_key}").as_bytes(), date.as_bytes());
    let region_key = hmac_sha256(&date_key, region.as_bytes());
    let service_key = hmac_sha256(&region_key, b"s3");
    hmac_sha256(&service_key, b"aws4_request")
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key size");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn encode_segment(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

fn validate_remote_endpoint(value: &str) -> anyhow::Result<()> {
    let url = Url::parse(value)?;
    ensure!(
        matches!(url.scheme(), "http" | "https"),
        "remote endpoint must use HTTP(S)"
    );
    if url.scheme() == "https" {
        return Ok(());
    }
    let host = url.host_str().context("remote endpoint has no host")?;
    let local = host.eq_ignore_ascii_case("localhost")
        || host.parse::<std::net::IpAddr>().is_ok_and(|ip| match ip {
            std::net::IpAddr::V4(ip) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
            std::net::IpAddr::V6(ip) => ip.is_loopback(),
        });
    ensure!(
        local,
        "remote HTTP endpoint is allowed only on local/private networks"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{encode_segment, validate_remote_endpoint};

    #[test]
    fn encodes_s3_path_segments() {
        assert_eq!(encode_segment("a b.png"), "a%20b.png");
    }

    #[test]
    fn rejects_cleartext_public_remote_storage() {
        assert!(validate_remote_endpoint("http://example.com/storage").is_err());
        assert!(validate_remote_endpoint("http://127.0.0.1:9000").is_ok());
    }
}
