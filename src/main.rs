use std::{net::SocketAddr, sync::Arc};

use axum::{Router, extract::State, routing::get};
use spinploy::{api, build_client_from_env};
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
struct AppState {
    client: Arc<api::Client>,
}

async fn healthz(State(_state): State<AppState>) -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing with env filter, defaulting to info levels if RUST_LOG is unset.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info,axum=info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();

    let client = build_client_from_env()?;
    let state = AppState {
        client: Arc::new(client),
    };

    let app = Router::new()
        .route("/healthz", get(healthz))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr = std::env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:3000".to_string())
        .parse()?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("listening on {}", addr);
    axum::serve(listener, app).await?;

    Ok(())
}
