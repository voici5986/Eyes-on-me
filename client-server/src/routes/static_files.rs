use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use axum::{
    Router,
    body::Body,
    extract::State,
    http::{StatusCode, Uri},
    response::IntoResponse,
    routing::get,
};
use include_dir::{Dir, include_dir};
use tokio::fs;

static EMBEDDED_WEB_DIST: Dir<'_> = include_dir!("$EYES_ON_ME_EMBED_WEB_DIST");

#[derive(Clone)]
struct StaticState {
    web_dist_dir: Option<Arc<PathBuf>>,
}

pub fn router(web_dist_dir: Option<PathBuf>) -> Router {
    let state = StaticState {
        web_dist_dir: web_dist_dir.map(Arc::new),
    };

    Router::new()
        .route("/", get(index))
        .route("/{*path}", get(asset))
        .with_state(state)
}

async fn index(State(state): State<StaticState>) -> impl IntoResponse {
    serve_request_path(&state, "index.html", true).await
}

async fn asset(State(state): State<StaticState>, uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');
    serve_request_path(&state, path, false).await
}

async fn serve_request_path(
    state: &StaticState,
    request_path: &str,
    is_index_request: bool,
) -> axum::response::Response {
    let Some(normalized_path) = normalize_request_path(request_path) else {
        return not_found_response();
    };

    if let Some(response) = serve_file_from_disk(state, &normalized_path).await {
        return response;
    }

    if let Some(response) = serve_file_from_embedded(&normalized_path) {
        return response;
    }

    if !is_index_request {
        let index_path = PathBuf::from("index.html");
        if let Some(response) = serve_file_from_disk(state, &index_path).await {
            return response;
        }

        if let Some(response) = serve_file_from_embedded(&index_path) {
            return response;
        }
    }

    not_found_response()
}

async fn file_exists(path: &PathBuf) -> bool {
    fs::metadata(path)
        .await
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
}

async fn serve_file_from_disk(
    state: &StaticState,
    request_path: &Path,
) -> Option<axum::response::Response> {
    let root = state.web_dist_dir.as_ref()?;
    let candidate = root.join(request_path);
    if !file_exists(&candidate).await {
        return None;
    }

    fs::read(&candidate).await.ok().map(|bytes| {
        let mime = mime_guess::from_path(&candidate).first_or_octet_stream();
        (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, mime.as_ref())],
            Body::from(bytes),
        )
            .into_response()
    })
}

fn serve_file_from_embedded(request_path: &Path) -> Option<axum::response::Response> {
    let file = EMBEDDED_WEB_DIST.get_file(request_path)?;
    let mime = mime_guess::from_path(request_path).first_or_octet_stream();

    Some(
        (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, mime.as_ref())],
            Body::from(file.contents()),
        )
            .into_response(),
    )
}

fn normalize_request_path(request_path: &str) -> Option<PathBuf> {
    let trimmed = request_path.trim_matches('/');
    if trimmed.is_empty() {
        return Some(PathBuf::from("index.html"));
    }

    let path = Path::new(trimmed);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return None;
    }
    Some(path.to_path_buf())
}

fn not_found_response() -> axum::response::Response {
    (
        StatusCode::NOT_FOUND,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; charset=utf-8",
        )],
        Body::from("not found"),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::normalize_request_path;

    #[test]
    fn rejects_parent_directory_static_paths() {
        assert_eq!(
            normalize_request_path("assets/app.js"),
            Some(PathBuf::from("assets/app.js"))
        );
        assert_eq!(
            normalize_request_path(""),
            Some(PathBuf::from("index.html"))
        );
        assert_eq!(normalize_request_path("../Cargo.toml"), None);
        assert_eq!(normalize_request_path("assets/../../Cargo.toml"), None);
    }
}
