mod app_state;
mod config;
mod db;
mod routes;

use std::{net::SocketAddr, sync::Arc};

use app_state::AppState;
use axum::Router;
use config::Config;
use tokio::net::TcpListener;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "client_server=info,tower_http=info".into()),
        )
        .compact()
        .init();

    let config = Config::from_env();
    let pool = db::connect(&config.database_url).await?;
    let snapshot = db::load_snapshot(&pool)
        .await
        .unwrap_or_else(|_| amiokay_shared::DashboardSnapshot::demo());
    let state = Arc::new(AppState::new(snapshot, pool));

    let app = Router::new()
        .merge(routes::api_router(state))
        .merge(routes::static_router(config.web_dist_dir.clone()))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive());

    let addr = SocketAddr::from((config.host, config.port));
    let listener = TcpListener::bind(addr).await?;

    info!(
        address = %addr,
        web_assets = %config.web_assets_mode(),
        database = %config.database_url,
        "server listening"
    );

    axum::serve(listener, app).await?;

    Ok(())
}
