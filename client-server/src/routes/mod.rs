mod api;
mod static_files;

use std::{path::PathBuf, sync::Arc};

use axum::Router;

use crate::app_state::AppState;

pub fn api_router(state: Arc<AppState>) -> Router {
    api::router(state)
}

pub fn static_router(web_dist_dir: Option<PathBuf>) -> Router {
    static_files::router(web_dist_dir)
}
