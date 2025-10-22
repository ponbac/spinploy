use std::{net::SocketAddr, sync::Arc};

use axum::http::request::Parts;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{delete, get, post},
};
use spinploy::dokploy_client::{DeployPreviewRequest, DeployPreviewResponse, DokployClient};
use std::future::ready;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
struct AppState {
    client: Arc<DokployClient>,
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

    let client = DokployClient::new(std::env::var("DOKPLOY_URL").unwrap());
    let state = AppState {
        client: Arc::new(client),
    };

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/previews", post(create_or_update_preview))
        .route("/previews", delete(delete_preview))
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

// Simple extractor to pull x-api-key from headers
struct ApiKey(String);

impl<S> axum::extract::FromRequestParts<S> for ApiKey
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);

    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let res = parts
            .headers
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
            .map(|s| ApiKey(s.to_string()))
            .ok_or((StatusCode::BAD_REQUEST, "missing x-api-key".to_string()));
        ready(res)
    }
}

#[derive(serde::Deserialize)]
struct DeletePreviewBody {
    project_id: String,
    environment_id: String,
    // Either direct compose name or identifying params to derive it
    compose_name: Option<String>,
    pr_number: Option<u64>,
    branch_name: Option<String>,
}

async fn create_or_update_preview(
    State(state): State<AppState>,
    ApiKey(api_key): ApiKey,
    Json(body): Json<DeployPreviewRequest>,
) -> Result<Json<DeployPreviewResponse>, (StatusCode, String)> {
    state
        .client
        .deploy_preview(&api_key, body)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("deploy failed: {e}")))
}

async fn delete_preview(
    State(state): State<AppState>,
    ApiKey(api_key): ApiKey,
    Json(body): Json<DeletePreviewBody>,
) -> Result<StatusCode, (StatusCode, String)> {
    let compose_name = if let Some(name) = body.compose_name {
        name
    } else {
        // Derive identifier -> default compose name
        spinploy::compute_identifier(body.pr_number.map(|n| n.to_string()), body.branch_name)
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
    };

    state
        .client
        .delete_preview(
            &api_key,
            &body.project_id,
            &body.environment_id,
            &compose_name,
        )
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("delete failed: {e}")))
}
