use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::request::Parts;
use axum::http::{HeaderName, HeaderValue, Request};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    middleware::{self, Next},
    routing::{delete, get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use futures_util::stream::Stream;
use serde::{Deserialize, Serialize};
use spinploy::azure_client::AzureDevOpsClient;
use spinploy::docker_client::DockerClient;
use spinploy::models::azure::*;
use spinploy::slack_client::SlackWebhookClient;
use spinploy::{
    Config, DokployClient, DomainCreateRequest, SlashCommand, UpdateComposeRequest, parse_ts,
};
use tokio::sync::RwLock;
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::ReceiverStream;
use tower::ServiceBuilder;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

mod api;

const PREVIEW_LIMIT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthDecision {
    Valid,
    Invalid,
}

struct CacheEntry {
    decision: AuthDecision,
    expires_at: Instant,
}

struct AuthCache {
    entries: RwLock<HashMap<String, CacheEntry>>,
    ttl: Duration,
    negative_ttl: Duration,
    max_keys: usize,
}

impl AuthCache {
    fn new(ttl_secs: u64, negative_ttl_secs: u64, max_keys: usize) -> Self {
        Self {
            entries: RwLock::new(HashMap::with_capacity(max_keys)),
            ttl: Duration::from_secs(ttl_secs),
            negative_ttl: Duration::from_secs(negative_ttl_secs),
            max_keys,
        }
    }

    async fn get(&self, key: &str) -> Option<AuthDecision> {
        let entries = self.entries.read().await;
        entries
            .get(key)
            .filter(|entry| entry.expires_at > Instant::now())
            .map(|entry| entry.decision)
    }

    async fn insert(&self, key: String, decision: AuthDecision) {
        let mut entries = self.entries.write().await;

        // Simple eviction: if we're at capacity, clear everything to keep it simple
        // as we don't have a dedicated LRU here and max_keys is usually large.
        if entries.len() >= self.max_keys {
            entries.clear();
        }

        let ttl = match decision {
            AuthDecision::Valid => self.ttl,
            AuthDecision::Invalid => self.negative_ttl,
        };

        entries.insert(
            key,
            CacheEntry {
                decision,
                expires_at: Instant::now() + ttl,
            },
        );
    }
}

#[derive(Clone)]
pub struct AppState {
    pub dokploy_client: Arc<DokployClient>,
    pub config: Config,
    pub azure_client: Arc<AzureDevOpsClient>,
    pub docker_client: Option<Arc<DockerClient>>,
    pub slack_client: Arc<SlackWebhookClient>,
    pub(crate) auth_cache: Arc<AuthCache>,
}

async fn healthz(State(_state): State<AppState>) -> &'static str {
    "ok"
}

// Middleware to protect static storage with a simple header token check
async fn storage_auth(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Result<axum::response::Response, StatusCode> {
    let Some(expected) = state.config.storage.map(|config| config.token) else {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };

    let header_name = HeaderName::from_static("x-storage-token");
    let provided = req
        .headers()
        .get(&header_name)
        .and_then(|v| v.to_str().ok());

    if Some(expected).as_deref() == provided {
        Ok(next.run(req).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
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

    // Try to connect to Docker socket; if unavailable, log a warning and proceed without it
    let docker_client = match DockerClient::new() {
        Ok(dc) => {
            tracing::info!("Docker client initialized successfully");
            Some(Arc::new(dc))
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "Docker client unavailable. Container log streaming will be disabled. \
                Ensure /var/run/docker.sock is mounted."
            );
            None
        }
    };

    let state = AppState {
        dokploy_client: Arc::new(client),
        azure_client: Arc::new(AzureDevOpsClient::new(
            &config.azdo_org,
            &config.azdo_project,
            &config.azdo_pat,
        )),
        docker_client,
        slack_client: Arc::new(SlackWebhookClient::new(&config.slack_webhook_url)?),
        auth_cache: Arc::new(AuthCache::new(
            config.auth_cache_ttl_secs,
            config.auth_cache_negative_ttl_secs,
            1024, // At the moment there will only be one valid key, but could be useful in the future
        )),
        config,
    };

    // Frontend serving: index.html with no-cache headers
    let serve_index = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("cache-control"),
            HeaderValue::from_static("no-store, no-cache, must-revalidate, max-age=0"),
        ))
        .service(ServeFile::new("./app/dist/index.html"));

    // Serve static assets from app/dist, fallback to index.html for SPA routing
    let serve_frontend = ServeDir::new("./app/dist").not_found_service(serve_index);

    let mut app = Router::new()
        .route("/healthz", get(healthz))
        .route("/previews", post(create_or_update_preview))
        .route("/previews", delete(delete_preview))
        .route("/webhooks/azure/pr-comment", post(azure_pr_comment_webhook))
        .route("/webhooks/azure/pr-updated", post(azure_pr_updated_webhook))
        .route(
            "/webhooks/azure/build-completed",
            post(azure_build_completed_webhook),
        )
        .route("/containers", get(list_containers))
        .route("/containers/{name}/logs", get(stream_container_logs))
        .nest("/api", api::preview_routes())
        .fallback_service(serve_frontend)
        .with_state(state.clone())
        .layer(TraceLayer::new_for_http());

    if let Some(storage_config) = state.config.storage.clone() {
        let storage_router = Router::new()
            .route_service("/{*path}", ServeDir::new(storage_config.dir))
            .route_layer(middleware::from_fn_with_state(state.clone(), storage_auth))
            .with_state(state.clone());

        app = app.nest("/storage", storage_router);
    } else {
        tracing::info!(
            storage_config = state.config.storage.is_some(),
            "Storage serving disabled: missing STORAGE_BASE_URL, STORAGE_DIR or STORAGE_TOKEN"
        );
    }

    let addr: SocketAddr = std::env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
        .parse()?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("listening on {}", addr);
    axum::serve(listener, app).await?;

    Ok(())
}

// Extractor to pull API key from `x-api-key` or fallback Basic auth password
pub struct ApiKey(pub String);

impl axum::extract::FromRequestParts<AppState> for ApiKey {
    type Rejection = (StatusCode, String);

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
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

        let state = state.clone();

        async move {
            let Some(api_key) = api_key else {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "missing x-api-key or Basic auth password".to_string(),
                ));
            };

            // Check cache first
            if let Some(decision) = state.auth_cache.get(&api_key).await {
                return match decision {
                    AuthDecision::Valid => Ok(ApiKey(api_key)),
                    AuthDecision::Invalid => {
                        Err((StatusCode::UNAUTHORIZED, "Invalid API key".to_string()))
                    }
                };
            }

            // Validate against Dokploy
            match state.dokploy_client.fetch_projects(&api_key).await {
                Ok(_) => {
                    state
                        .auth_cache
                        .insert(api_key.clone(), AuthDecision::Valid)
                        .await;
                    Ok(ApiKey(api_key))
                }
                Err(e) => {
                    // Check if it's an auth error (401/403)
                    let is_auth_error = if let Some(reqwest_err) =
                        e.downcast_ref::<reqwest::Error>()
                    {
                        reqwest_err
                            .status()
                            .map(|s| s == StatusCode::UNAUTHORIZED || s == StatusCode::FORBIDDEN)
                            .unwrap_or(false)
                    } else {
                        false
                    };

                    if is_auth_error {
                        state
                            .auth_cache
                            .insert(api_key, AuthDecision::Invalid)
                            .await;
                        Err((StatusCode::UNAUTHORIZED, "Invalid API key".to_string()))
                    } else {
                        // Connectivity or other errors - fail closed but don't cache negative decision
                        tracing::error!(error = %e, "Failed to validate API key against Dokploy");
                        Err((
                            StatusCode::SERVICE_UNAVAILABLE,
                            "Unable to validate API key with Dokploy at this time".to_string(),
                        ))
                    }
                }
            }
        }
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

        let dynamic_env_vars = format!(
            "APP_URL=https://{}\nBACKEND_API_URL=https://{}\nEMAIL_ENVIRONMENT_PREFIX=\"[{}] \"\n",
            frontend_domain, backend_domain, identifier
        );
        let project_env_vars = r#"
COOKIE_DOMAIN=${{project.COOKIE_DOMAIN}}
STORAGE_URL=${{project.STORAGE_URL}}
STORAGE_TOKEN=${{project.STORAGE_TOKEN}}

EMAIL_INVOICE_CREDENTIALS_PASSWORD=${{project.EMAIL_INVOICE_CREDENTIALS_PASSWORD}}
EMAIL_DIRECT_REGULATION_CREDENTIALS_PASSWORD=${{project.EMAIL_DIRECT_REGULATION_CREDENTIALS_PASSWORD}}
EMAIL_TEST_ANSWER_CREDENTIALS_PASSWORD=${{project.EMAIL_TEST_ANSWER_CREDENTIALS_PASSWORD}}
EMAIL_REFERRAL_CREDENTIALS_PASSWORD=${{project.EMAIL_REFERRAL_CREDENTIALS_PASSWORD}}

FEATURE_MANAGEMENT_FREJA_POLLING_JOB=${{project.FEATURE_MANAGEMENT_FREJA_POLLING_JOB}}
FEATURE_MANAGEMENT_VARA_IMPORT_JOB=${{project.FEATURE_MANAGEMENT_VARA_IMPORT_JOB}}
FEATURE_MANAGEMENT_SMS_JOBS=${{project.FEATURE_MANAGEMENT_SMS_JOBS}}

SMS_PASSWORD_BASIC_AUTH=${{project.SMS_PASSWORD_BASIC_AUTH}}
SMS_PASSWORD_XML=${{project.SMS_PASSWORD_XML}}

VARA_PASSWORD=${{project.VARA_PASSWORD}}
        "#;

        dokploy_client
            .update_compose(
                api_key,
                UpdateComposeRequest {
                    compose_id: compose.compose_id.clone(),
                    name: identifier.clone(),
                    app_name: app_name.clone(),
                    env: dynamic_env_vars + project_env_vars,
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
        ..
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
        azure_client,
        ..
    }): State<AppState>,
    ApiKey(api_key): ApiKey,
    Json(payload): Json<AzurePrCommentEvent>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    if payload.event_type != "ms.vss-code.git-pullrequest-comment-event" {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }

    // No-op on deleted comments or missing/empty content
    if payload.resource.comment.is_deleted
        || payload
            .resource
            .comment
            .content
            .as_deref()
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
    {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }

    let Some(cmd) = &payload
        .resource
        .comment
        .content
        .as_deref()
        .unwrap_or("")
        .parse::<SlashCommand>()
        .ok()
    else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };

    let branch = spinploy::strip_refs_heads(&payload.resource.pull_request.source_ref_name);
    let pr_id = Some(payload.resource.pull_request.pull_request_id.to_string());

    tracing::info!(
        pr = pr_id.as_deref().unwrap_or("?"),
        branch,
        ?cmd,
        "Received Azure PR comment webhook"
    );

    // Extract thread id from the threads link ending with /threads/{id}
    let thread_href = &payload.resource.comment.links.threads.href;
    let thread_id = thread_href
        .rsplit('/')
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or((
            StatusCode::BAD_REQUEST,
            "invalid threads href in payload".to_string(),
        ))?;
    let repo_id = &config.azdo_repository_id;

    match cmd {
        SlashCommand::Preview => {
            let resp = upsert_preview_internal(&dokploy_client, &config, &api_key, &branch, &pr_id)
                .await?;

            let identifier = spinploy::compute_identifier(&pr_id, &branch);
            let frontend = format!("https://{}.{}", identifier, &config.base_domain);
            if let Err(e) = azure_client
                .reply_in_thread(
                    repo_id,
                    payload.resource.pull_request.pull_request_id,
                    thread_id,
                    &format!("👷 Preview building, should be available soon: {} \n\n💻 View the status of all previews here: {}", frontend, config.deployed_preview_api_path),
                )
                .await
            {
                tracing::warn!(error = %e, "Failed to post ADO reply for /preview");
            }

            Ok(Json(resp).into_response())
        }
        SlashCommand::Delete => {
            delete_preview_internal(&dokploy_client, &api_key, &pr_id, &branch).await?;

            if let Err(e) = azure_client
                .reply_in_thread(
                    repo_id,
                    payload.resource.pull_request.pull_request_id,
                    thread_id,
                    "Preview deleted",
                )
                .await
            {
                tracing::warn!(error = %e, "Failed to post ADO reply for /delete");
            }

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

    let branch = spinploy::strip_refs_heads(&payload.resource.source_ref_name);
    let pr_id = Some(payload.resource.pull_request_id.to_string());

    // If this is a status update and PR is completed, delete preview (if target is main)
    if payload
        .resource
        .status
        .as_deref()
        .map(|s| s.eq_ignore_ascii_case("completed"))
        .unwrap_or(false)
    {
        let target_branch =
            spinploy::strip_refs_heads(payload.resource.target_ref_name.as_deref().unwrap_or(""));

        tracing::info!(
            pr = pr_id.as_deref().unwrap_or("?"),
            source_branch = branch,
            target_branch,
            "Received Azure PR updated webhook (status=completed)"
        );

        if target_branch == "main" {
            delete_preview_internal(&dokploy_client, &api_key, &pr_id, &branch).await?;
        }
        return Ok(StatusCode::NO_CONTENT.into_response());
    }

    tracing::info!(
        pr = pr_id.as_deref().unwrap_or("?"),
        branch,
        "Received Azure PR updated webhook (push). Attempting redeploy if exists"
    );

    redeploy_preview_if_exists(&dokploy_client, &api_key, &pr_id, &branch).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn azure_build_completed_webhook(
    State(AppState {
        azure_client,
        slack_client,
        ..
    }): State<AppState>,
    ApiKey(_api_key): ApiKey,
    Json(payload): Json<AzureBuildCompletedEvent>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let event_ok = payload.event_type.eq_ignore_ascii_case("build.complete")
        || payload.event_type.eq_ignore_ascii_case("build.completed");
    if !event_ok {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }

    let build_id = payload.resource.id;

    let build = azure_client.get_build(build_id).await.map_err(|e| {
        tracing::error!(error = %e, build_id, "Failed to fetch build details");
        (
            StatusCode::BAD_GATEWAY,
            "failed to fetch build details".to_string(),
        )
    })?;

    let build_failed = payload
        .resource
        .result
        .as_deref()
        .map(|r| r.eq_ignore_ascii_case("failed"))
        .unwrap_or(false)
        || build
            .result
            .as_deref()
            .map(|r| r.eq_ignore_ascii_case("failed"))
            .unwrap_or(false);

    if !build_failed {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }

    let timeline = azure_client
        .get_build_timeline(build_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, build_id, "Failed to fetch build timeline");
            (
                StatusCode::BAD_GATEWAY,
                "failed to fetch build timeline".to_string(),
            )
        })?;

    let e2e_failed = timeline.records.iter().any(|r| {
        r.name == "Run E2E tests"
            && r.result
                .as_deref()
                .map(|res| res.eq_ignore_ascii_case("failed"))
                .unwrap_or(false)
    });

    if !e2e_failed {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }

    tracing::info!(
        build_id,
        build_number = build.build_number.as_deref().unwrap_or(""),
        "E2E stage failed; checking prior builds for regression"
    );

    // Helper closures reused below
    let e2e_failed_in = |tl: &AzureBuildTimeline| {
        tl.records.iter().any(|r| {
            r.name == "Run E2E tests"
                && r.result
                    .as_deref()
                    .map(|res| res.eq_ignore_ascii_case("failed"))
                    .unwrap_or(false)
        })
    };
    let e2e_stage_present =
        |tl: &AzureBuildTimeline| tl.records.iter().any(|r| r.name == "Run E2E tests");

    // If we cannot check history, proceed to send (per user request).
    if let (Some(definition_id), Some(branch_name)) = (
        build.definition.as_ref().map(|d| d.id),
        build.source_branch.as_deref(),
    ) {
        match azure_client
            .list_builds(definition_id, branch_name, 10)
            .await
        {
            Ok(recent) => {
                tracing::debug!(
                    build_id,
                    definition_id,
                    branch_name,
                    recent_count = recent.len(),
                    "Fetched recent builds for regression check"
                );
                for b in recent {
                    if b.id == build_id {
                        continue;
                    }
                    match azure_client.get_build_timeline(b.id).await {
                        Ok(prev_tl) => {
                            if !e2e_stage_present(&prev_tl) {
                                tracing::debug!(
                                    build_id,
                                    prev_build_id = b.id,
                                    "Previous build missing E2E stage; continuing search"
                                );
                                continue;
                            }
                            if e2e_failed_in(&prev_tl) {
                                tracing::info!(
                                    build_id,
                                    prev_build_id = b.id,
                                    "E2E already failing in previous build; suppressing Slack"
                                );
                                return Ok(StatusCode::NO_CONTENT.into_response());
                            }
                            tracing::info!(
                                build_id,
                                prev_build_id = b.id,
                                "Previous build had E2E stage without failure; treating as new regression"
                            );
                            break;
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                build_id,
                                prev_build_id = b.id,
                                "Failed to fetch previous build timeline; continuing search"
                            );
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    build_id,
                    definition_id,
                    branch_name,
                    "Failed to list builds; proceeding to send Slack"
                );
            }
        }
    } else {
        tracing::warn!(
            build_id,
            has_definition = build.definition.is_some(),
            has_branch = build.source_branch.is_some(),
            "Missing definition or branch; proceeding to send Slack without regression check"
        );
    }

    let repo_id = build.repository.as_ref().map(|r| r.id.as_str()).ok_or((
        StatusCode::BAD_REQUEST,
        "build missing repository id".to_string(),
    ))?;

    let commit = azure_client
        .get_commit(repo_id, &build.source_version)
        .await
        .map_err(|e| {
            tracing::error!(
                error = %e,
                build_id,
                repo = repo_id,
                commit = build.source_version,
                "Failed to fetch commit details"
            );
            (
                StatusCode::BAD_GATEWAY,
                "failed to fetch commit details".to_string(),
            )
        })?;

    let build_number = build
        .build_number
        .clone()
        .unwrap_or_else(|| build_id.to_string());
    let build_link = build
        .links
        .as_ref()
        .and_then(|l| l.web.as_ref())
        .map(|h| h.href.as_str())
        .unwrap_or("");

    let mut message = format!(
        "*:warning: Run E2E tests failed*\n\n• 🏗️ Build: *{}* (ID `{}`)\n• 🧪 Stage: `Run E2E tests`\n• 👤 Commit author: *{}*",
        build_number, build_id, commit.author.name
    );

    if !build_link.is_empty() {
        message.push('\n');
        message.push_str(&format!("• 🔗 Link: {}", build_link));
    }

    slack_client.send_text(message).await.map_err(|e| {
        tracing::error!(error = %e, build_id, "Failed to send Slack webhook");
        (
            StatusCode::BAD_GATEWAY,
            "failed to send Slack notification".to_string(),
        )
    })?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

// =====================
// Container Log Endpoints
// =====================

#[derive(Debug, Deserialize)]
struct LogsQuery {
    /// Number of lines to return from the end of the logs (default: 100, 0 = all)
    #[serde(default = "default_tail")]
    tail: u64,
    /// Whether to follow the log stream in real-time (default: true)
    #[serde(default = "default_follow")]
    follow: bool,
}

fn default_tail() -> u64 {
    100
}

fn default_follow() -> bool {
    true
}

/// GET /containers
/// Lists all containers, optionally filtered by name.
async fn list_containers(
    State(state): State<AppState>,
    ApiKey(_api_key): ApiKey,
    Query(params): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let docker = state.docker_client.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Docker client not available. Ensure /var/run/docker.sock is mounted.".to_string(),
    ))?;

    let name_filter = params.get("name").map(|s| s.as_str());
    let containers = docker
        .list_containers(name_filter)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(containers))
}

/// GET /containers/{name}/logs
/// Streams container logs as Server-Sent Events (SSE).
///
/// Query parameters:
/// - `tail`: Number of lines to return from the end (default: 100, 0 = all)
/// - `follow`: Whether to follow logs in real-time (default: true)
///
/// Example: GET /containers/my-app/logs?tail=50&follow=true
async fn stream_container_logs(
    State(state): State<AppState>,
    ApiKey(_api_key): ApiKey,
    Path(container_name): Path<String>,
    Query(query): Query<LogsQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, (StatusCode, String)>
{
    let docker = state.docker_client.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Docker client not available. Ensure /var/run/docker.sock is mounted.".to_string(),
    ))?;

    tracing::info!(
        container = %container_name,
        tail = query.tail,
        follow = query.follow,
        "Starting log stream"
    );

    let rx = docker
        .stream_logs(&container_name, query.tail, query.follow)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;

    let stream = ReceiverStream::new(rx).map(|result| {
        let event = match result {
            Ok(line) => Event::default().data(line),
            Err(e) => Event::default().event("error").data(e),
        };
        Ok::<_, std::convert::Infallible>(event)
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
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
            let mut detailed = futures::future::join_all(comps.iter().map(|c| async move {
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
