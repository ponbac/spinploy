use std::{net::SocketAddr, sync::Arc};

use axum::http::request::Parts;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use spinploy::{Config, DokployClient, DomainCreateRequest, UpdateComposeRequest};
use std::future::ready;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
struct AppState {
    dokploy_client: Arc<DokployClient>,
    config: Config,
}

async fn healthz(State(_state): State<AppState>) -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing with env filter, defaulting to debug levels if RUST_LOG is unset.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("debug,axum=info,reqwest=info,hyper_util=info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .compact()
        .init();

    let config = Config::load()?;
    let client = DokployClient::new(&config.dokploy_url);

    let state = AppState {
        dokploy_client: Arc::new(client),
        config,
    };

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/previews", post(create_or_update_preview))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr = std::env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
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
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let res = parts
            .headers
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
            .map(|s| ApiKey(s.to_string()))
            .ok_or((StatusCode::BAD_REQUEST, "missing x-api-key".to_string()));
        ready(res)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeCreateUpdateRequest {
    pub git_branch: String,
    pub pr_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeCreateUpdateResponse {
    pub compose_id: String,
    pub domains: Vec<String>,
}

async fn create_or_update_preview(
    State(AppState {
        dokploy_client,
        config,
    }): State<AppState>,
    ApiKey(api_key): ApiKey,
    Json(body): Json<ComposeCreateUpdateRequest>,
) -> Result<Json<ComposeCreateUpdateResponse>, (StatusCode, String)> {
    let identifier = spinploy::compute_identifier(&body.pr_id, &body.git_branch);
    let app_name = format!("preview-{}", &identifier);

    if let Some(compose) = dokploy_client
        .find_compose_by_name(&api_key, &identifier)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?
    {
        dokploy_client
            .deploy_compose(&api_key, &compose.compose_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let domains = dokploy_client
            .list_domains_by_compose_id(&api_key, &compose.compose_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        Ok(Json(ComposeCreateUpdateResponse {
            compose_id: compose.compose_id,
            domains: domains.into_iter().map(|d| d.host).collect(),
        }))
    } else {
        let compose = dokploy_client
            .create_compose(&api_key, &config.environment_id, &identifier, &app_name)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let frontend_domain = format!("{}.{}", &identifier, &config.base_domain);
        let backend_domain = format!("api-{}.{}", &identifier, &config.base_domain);
        let env_vars = format!(
            "APP_URL=https://{}\nBACKEND_API_URL=https://{}\nCOOKIE_DOMAIN=.{}",
            frontend_domain, backend_domain, &config.base_domain
        );

        dokploy_client
            .update_compose(
                &api_key,
                UpdateComposeRequest {
                    compose_id: compose.compose_id.clone(),
                    name: identifier.clone(),
                    app_name: app_name.clone(),
                    env: env_vars,
                    environment_id: config.environment_id.clone(),
                    auto_deploy: true,
                    isolated_deployment: true,
                    compose_path: config.compose_path.clone(),
                    source_type: "git".to_string(),
                    compose_type: "docker-compose".to_string(),
                    custom_git_url: config.custom_git_url.clone(),
                    custom_git_branch: body.git_branch.clone(),
                    custom_git_ssh_key_id: config.custom_git_ssh_key_id.clone(),
                },
            )
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        // Frontend domain
        dokploy_client
            .create_domain(
                &api_key,
                DomainCreateRequest {
                    compose_id: compose.compose_id.clone(),
                    service_name: config.frontend_service_name.clone(),
                    domain_type: "compose".to_string(),
                    host: frontend_domain,
                    path: "/".to_string(),
                    port: config.frontend_port,
                    https: true,
                    certificate_type: "none".to_string(),
                },
            )
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        // Backend domain
        dokploy_client
            .create_domain(
                &api_key,
                DomainCreateRequest {
                    compose_id: compose.compose_id.clone(),
                    service_name: config.backend_service_name.clone(),
                    domain_type: "compose".to_string(),
                    host: backend_domain,
                    path: "/".to_string(),
                    port: config.backend_port,
                    https: true,
                    certificate_type: "none".to_string(),
                },
            )
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        // Deploy
        dokploy_client
            .deploy_compose(&api_key, &compose.compose_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let domains = dokploy_client
            .list_domains_by_compose_id(&api_key, &compose.compose_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        Ok(Json(ComposeCreateUpdateResponse {
            compose_id: compose.compose_id,
            domains: domains.into_iter().map(|d| d.host).collect(),
        }))
    }
}
