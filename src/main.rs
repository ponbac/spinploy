use std::{net::SocketAddr, sync::Arc};

use axum::http::request::Parts;
use axum::response::IntoResponse;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{delete, get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use spinploy::models::azure::*;
use spinploy::{
    Config, DokployClient, DomainCreateRequest, SlashCommand, UpdateComposeRequest, parse_ts,
};
use std::future::ready;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

const PREVIEW_LIMIT: usize = 5;

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
        .route("/previews", delete(delete_preview))
        .route("/webhooks/azure/pr-comment", post(azure_pr_comment_webhook))
        .route("/webhooks/azure/pr-updated", post(azure_pr_updated_webhook))
        .route("/webhooks/azure/pr-merged", post(azure_pr_merged_webhook))
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

// Extractor to pull API key from `x-api-key` or fallback Basic auth password
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
        let api_key = parts
            .headers
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .or_else(|| {
                parts
                    .headers
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|auth| {
                        let auth = auth.trim();
                        let b64 = auth
                            .strip_prefix("Basic ")
                            .or_else(|| auth.strip_prefix("basic "))?;
                        let decoded = BASE64.decode(b64.as_bytes()).ok()?;
                        let creds = String::from_utf8(decoded).ok()?; // username:password
                        let mut it = creds.splitn(2, ':');
                        let _username = it.next();
                        let password = it.next().unwrap_or("");
                        if password.is_empty() {
                            None
                        } else {
                            Some(password.to_string())
                        }
                    })
            });

        let res = api_key.map(ApiKey).ok_or((
            StatusCode::BAD_REQUEST,
            "missing x-api-key or Basic auth password".to_string(),
        ));
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

async fn upsert_preview_internal(
    dokploy_client: &DokployClient,
    config: &Config,
    api_key: &str,
    git_branch: &str,
    pr_id: &Option<String>,
) -> Result<ComposeCreateUpdateResponse, (StatusCode, String)> {
    let identifier = spinploy::compute_identifier(pr_id, git_branch);
    let app_name = format!("preview-{}", &identifier);

    if let Some(compose) = dokploy_client
        .find_compose_by_name(api_key, &identifier)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?
    {
        dokploy_client
            .deploy_compose(api_key, &compose.compose_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let domains = dokploy_client
            .list_domains_by_compose_id(api_key, &compose.compose_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        Ok(ComposeCreateUpdateResponse {
            compose_id: compose.compose_id,
            domains: domains.into_iter().map(|d| d.host).collect(),
        })
    } else {
        let compose = dokploy_client
            .create_compose(api_key, &config.environment_id, &identifier, &app_name)
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
                api_key,
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
                    custom_git_branch: git_branch.to_string(),
                    custom_git_ssh_key_id: config.custom_git_ssh_key_id.clone(),
                },
            )
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        dokploy_client
            .create_domain(
                api_key,
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

        dokploy_client
            .create_domain(
                api_key,
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

        dokploy_client
            .deploy_compose(api_key, &compose.compose_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let domains = dokploy_client
            .list_domains_by_compose_id(api_key, &compose.compose_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        // Prune previews in the environment after creating this one
        prune_previews_if_over_limit(
            dokploy_client,
            api_key,
            &config.environment_id,
            &compose.compose_id,
        )
        .await;

        Ok(ComposeCreateUpdateResponse {
            compose_id: compose.compose_id,
            domains: domains.into_iter().map(|d| d.host).collect(),
        })
    }
}

async fn delete_preview_internal(
    dokploy_client: &DokployClient,
    api_key: &str,
    pr_id: &Option<String>,
    git_branch: &str,
) -> Result<StatusCode, (StatusCode, String)> {
    let identifier = spinploy::compute_identifier(pr_id, git_branch);

    match dokploy_client
        .find_compose_by_name(&api_key, &identifier)
        .await
    {
        Ok(Some(compose)) => {
            dokploy_client
                .delete_compose(api_key, &compose.compose_id, true)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            Ok(StatusCode::NO_CONTENT)
        }
        Ok(None) => Ok(StatusCode::NO_CONTENT),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

async fn redeploy_preview_if_exists(
    dokploy_client: &DokployClient,
    api_key: &str,
    pr_id: &Option<String>,
    git_branch: &str,
) -> Result<(), (StatusCode, String)> {
    let identifier = spinploy::compute_identifier(pr_id, git_branch);
    match dokploy_client
        .find_compose_by_name(api_key, &identifier)
        .await
    {
        Ok(Some(compose)) => {
            tracing::info!(
                compose_id = compose.compose_id,
                identifier,
                "Redeploying existing preview"
            );
            dokploy_client
                .deploy_compose(api_key, &compose.compose_id)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            Ok(())
        }
        Ok(None) => {
            tracing::info!(identifier, "No existing preview to redeploy; skipping");
            Ok(())
        }
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

async fn create_or_update_preview(
    State(AppState {
        dokploy_client,
        config,
    }): State<AppState>,
    ApiKey(api_key): ApiKey,
    Json(body): Json<ComposeCreateUpdateRequest>,
) -> Result<Json<ComposeCreateUpdateResponse>, (StatusCode, String)> {
    let resp = upsert_preview_internal(
        &dokploy_client,
        &config,
        &api_key,
        &body.git_branch,
        &body.pr_id,
    )
    .await?;

    Ok(Json(resp))
}

async fn delete_preview(
    State(AppState { dokploy_client, .. }): State<AppState>,
    ApiKey(api_key): ApiKey,
    Json(body): Json<ComposeCreateUpdateRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    delete_preview_internal(&dokploy_client, &api_key, &body.pr_id, &body.git_branch).await?;

    Ok(StatusCode::NO_CONTENT)
}

async fn azure_pr_comment_webhook(
    State(AppState {
        dokploy_client,
        config,
    }): State<AppState>,
    ApiKey(api_key): ApiKey,
    Json(payload): Json<AzurePrCommentEvent>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    if payload.event_type != "ms.vss-code.git-pullrequest-comment-event" {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }

    let Some(cmd) = &payload
        .resource
        .comment
        .content
        .parse::<SlashCommand>()
        .ok()
    else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };

    let branch = payload
        .resource
        .pull_request
        .source_ref_name
        .strip_prefix("refs/heads/")
        .unwrap_or(&payload.resource.pull_request.source_ref_name)
        .to_string();
    let pr_id = Some(payload.resource.pull_request.pull_request_id.to_string());

    tracing::info!(
        pr = pr_id.as_deref().unwrap_or("?"),
        branch,
        ?cmd,
        "Received Azure PR comment webhook"
    );

    match cmd {
        SlashCommand::Preview => {
            let resp = upsert_preview_internal(&dokploy_client, &config, &api_key, &branch, &pr_id)
                .await?;
            Ok(Json(resp).into_response())
        }
        SlashCommand::Delete => {
            delete_preview_internal(&dokploy_client, &api_key, &pr_id, &branch).await?;
            Ok(StatusCode::NO_CONTENT.into_response())
        }
    }
}

async fn azure_pr_updated_webhook(
    State(AppState { dokploy_client, .. }): State<AppState>,
    ApiKey(api_key): ApiKey,
    Json(payload): Json<AzurePrUpdatedEvent>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    if payload.event_type != "git.pullrequest.updated" {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }

    let branch = payload
        .resource
        .source_ref_name
        .strip_prefix("refs/heads/")
        .unwrap_or(&payload.resource.source_ref_name)
        .to_string();
    let pr_id = Some(payload.resource.pull_request_id.to_string());

    tracing::info!(
        pr = pr_id.as_deref().unwrap_or("?"),
        branch,
        "Received Azure PR updated webhook (push). Attempting redeploy if exists"
    );

    redeploy_preview_if_exists(&dokploy_client, &api_key, &pr_id, &branch).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn azure_pr_merged_webhook(
    State(AppState { dokploy_client, .. }): State<AppState>,
    ApiKey(api_key): ApiKey,
    Json(payload): Json<AzurePrMergedEvent>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    if payload.event_type != "git.pullrequest.merged" {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }

    let target_branch = payload
        .resource
        .target_ref_name
        .strip_prefix("refs/heads/")
        .unwrap_or(&payload.resource.target_ref_name)
        .to_string();
    let source_branch = payload
        .resource
        .source_ref_name
        .strip_prefix("refs/heads/")
        .unwrap_or(&payload.resource.source_ref_name)
        .to_string();
    let pr_id = Some(payload.resource.pull_request_id.to_string());

    tracing::info!(
        pr = pr_id.as_deref().unwrap_or("?"),
        source_branch,
        target_branch,
        merge_status = %payload.resource.merge_status,
        "Received Azure PR merged webhook"
    );

    if payload
        .resource
        .merge_status
        .eq_ignore_ascii_case("succeeded")
        && target_branch == "main"
    {
        delete_preview_internal(&dokploy_client, &api_key, &pr_id, &source_branch).await?;
    }

    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn prune_previews_if_over_limit(
    client: &DokployClient,
    api_key: &str,
    environment_id: &str,
    exclude_compose_id: &str,
) {
    if let Ok(mut comps) = client
        .list_composes_with_prefix(api_key, environment_id, "preview-")
        .await
    {
        comps.retain(|c| c.compose_id != exclude_compose_id);
        let total_after_creation = comps.len() + 1; // include the newly created preview
        if total_after_creation > PREVIEW_LIMIT {
            let to_delete = total_after_creation - PREVIEW_LIMIT;

            // Fetch compose details concurrently
            let mut detailed =
                futures::future::join_all(comps.iter().cloned().map(|c| async move {
                    (
                        c.clone(),
                        client.get_compose_detail(api_key, &c.compose_id).await,
                    )
                }))
                .await;

            // Sort by latest deployment timestamp (finishedAt -> startedAt -> createdAt), fallback to compose createdAt
            detailed.sort_by_key(|(_c, detail)| {
                detail
                    .as_ref()
                    .ok()
                    .and_then(|dd| {
                        dd.deployments
                            .iter()
                            .filter_map(|d| d.finished_at.as_deref())
                            .filter_map(parse_ts)
                            .max()
                    })
                    .or_else(|| {
                        detail.as_ref().ok().and_then(|dd| {
                            dd.deployments
                                .iter()
                                .filter_map(|d| d.started_at.as_deref())
                                .filter_map(parse_ts)
                                .max()
                        })
                    })
                    .or_else(|| {
                        detail.as_ref().ok().and_then(|dd| {
                            dd.deployments
                                .iter()
                                .filter_map(|d| d.created_at.as_deref())
                                .filter_map(parse_ts)
                                .max()
                        })
                    })
                    .or_else(|| {
                        detail
                            .as_ref()
                            .ok()
                            .and_then(|dd| dd.created_at.as_deref().and_then(parse_ts))
                    })
            });

            for (doomed, _detail) in detailed.into_iter().take(to_delete) {
                if let Err(e) = client
                    .delete_compose(api_key, &doomed.compose_id, true)
                    .await
                {
                    tracing::warn!(
                        compose_id = doomed.compose_id,
                        error = %e,
                        "Failed to prune preview"
                    );
                }
            }
        }
    }
}
