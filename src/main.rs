use std::collections::{BTreeSet, HashMap};
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
const DELETE_RETRY_DELAYS: [Duration; 2] = [Duration::from_millis(250), Duration::from_secs(1)];
const LEGACY_E2E_RUN_NAME: &str = "Run E2E tests";
const MAIN_E2E_RUN_NAME: &str = "Run main E2E tests";
const JOURNAL_TEMPLATE_E2E_RUN_NAME: &str = "Run journal template E2E tests";
const TRACKED_E2E_RUN_NAMES: [&str; 3] = [
    LEGACY_E2E_RUN_NAME,
    MAIN_E2E_RUN_NAME,
    JOURNAL_TEMPLATE_E2E_RUN_NAME,
];

type TrackedE2eRuns = BTreeSet<&'static str>;

fn tracked_e2e_run_name(name: &str) -> Option<&'static str> {
    TRACKED_E2E_RUN_NAMES
        .iter()
        .copied()
        .find(|tracked_name| *tracked_name == name)
}

fn is_failed_result(result: Option<&str>) -> bool {
    result
        .map(|value| value.eq_ignore_ascii_case("failed"))
        .unwrap_or(false)
}

fn failed_e2e_run_names(timeline: &AzureBuildTimeline) -> TrackedE2eRuns {
    timeline
        .records
        .iter()
        .filter_map(|record| {
            tracked_e2e_run_name(&record.name)
                .filter(|_| is_failed_result(record.result.as_deref()))
        })
        .collect()
}

fn has_tracked_e2e_runs(timeline: &AzureBuildTimeline) -> bool {
    timeline
        .records
        .iter()
        .any(|record| tracked_e2e_run_name(&record.name).is_some())
}

fn format_tracked_e2e_runs(runs: &TrackedE2eRuns) -> String {
    TRACKED_E2E_RUN_NAMES
        .iter()
        .copied()
        .filter(|run_name| runs.contains(run_name))
        .collect::<Vec<_>>()
        .join("`, `")
}

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

pub struct PrTitleCache {
    entries: RwLock<HashMap<u64, (String, Instant)>>,
    ttl: Duration,
    max_entries: usize,
}

impl PrTitleCache {
    fn new(ttl_secs: u64, max_entries: usize) -> Self {
        Self {
            entries: RwLock::new(HashMap::with_capacity(max_entries)),
            ttl: Duration::from_secs(ttl_secs),
            max_entries,
        }
    }

    pub async fn get(&self, pr_id: u64) -> Option<String> {
        let entries = self.entries.read().await;
        entries
            .get(&pr_id)
            .filter(|(_, expires_at)| *expires_at > Instant::now())
            .map(|(title, _)| title.clone())
    }

    pub async fn insert(&self, pr_id: u64, title: String) {
        let mut entries = self.entries.write().await;
        if entries.len() >= self.max_entries {
            entries.clear();
        }
        entries.insert(pr_id, (title, Instant::now() + self.ttl));
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
    pub pr_title_cache: Arc<PrTitleCache>,
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
        pr_title_cache: Arc::new(PrTitleCache::new(600, 256)), // 10 minute TTL, max 256 entries
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

    let api_routes = api::preview_routes()
        .route("/previews", post(create_or_update_preview))
        .route("/previews", delete(delete_preview))
        .route("/containers", get(list_containers))
        .route("/containers/{name}/logs", get(stream_container_logs));

    let mut app = Router::new()
        .route("/healthz", get(healthz))
        .route("/webhooks/azure/pr-comment", post(azure_pr_comment_webhook))
        .route("/webhooks/azure/pr-updated", post(azure_pr_updated_webhook))
        .route(
            "/webhooks/azure/build-completed",
            post(azure_build_completed_webhook),
        )
        .nest("/api", api_routes)
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
EMAIL_NO_REPLY_CREDENTIALS_PASSWORD=${{project.EMAIL_NO_REPLY_CREDENTIALS_PASSWORD}}

FEATURE_MANAGEMENT_FREJA_POLLING_JOB=${{project.FEATURE_MANAGEMENT_FREJA_POLLING_JOB}}
FEATURE_MANAGEMENT_VARA_IMPORT_JOB=${{project.FEATURE_MANAGEMENT_VARA_IMPORT_JOB}}
FEATURE_MANAGEMENT_SMS_JOBS=${{project.FEATURE_MANAGEMENT_SMS_JOBS}}

SMS_PASSWORD_BASIC_AUTH=${{project.SMS_PASSWORD_BASIC_AUTH}}
SMS_PASSWORD_XML=${{project.SMS_PASSWORD_XML}}

VARA_PASSWORD=${{project.VARA_PASSWORD}}
IMAGE_ANALYSIS_API_KEY=${{project.IMAGE_ANALYSIS_API_KEY}}
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

#[derive(Debug)]
enum DeletePreviewOutcome {
    Deleted,
    AlreadyAbsent,
}

async fn delete_preview_internal(
    dokploy_client: &DokployClient,
    api_key: &str,
    identifier: &str,
) -> anyhow::Result<DeletePreviewOutcome> {
    let Some(compose) = dokploy_client
        .find_compose_by_name(api_key, identifier)
        .await?
    else {
        return Ok(DeletePreviewOutcome::AlreadyAbsent);
    };

    dokploy_client
        .delete_compose(api_key, &compose.compose_id, true)
        .await?;
    Ok(DeletePreviewOutcome::Deleted)
}

async fn delete_preview_with_retry(
    dokploy_client: &DokployClient,
    api_key: &str,
    identifier: &str,
) -> anyhow::Result<DeletePreviewOutcome> {
    let max_attempts = DELETE_RETRY_DELAYS.len() + 1;

    for attempt in 1..=max_attempts {
        match delete_preview_internal(dokploy_client, api_key, identifier).await {
            Ok(outcome) => return Ok(outcome),
            Err(error) if attempt == max_attempts => return Err(error),
            Err(error) => {
                let delay = DELETE_RETRY_DELAYS[attempt - 1];
                tracing::warn!(
                    identifier,
                    attempt,
                    max_attempts,
                    retry_delay_ms = delay.as_millis(),
                    error = %error,
                    "Preview deletion attempt failed; retrying"
                );
                tokio::time::sleep(delay).await;
            }
        }
    }

    unreachable!("delete retry loop always returns")
}

async fn run_preview_deletion(
    dokploy_client: &DokployClient,
    api_key: &str,
    identifier: &str,
) -> anyhow::Result<DeletePreviewOutcome> {
    match delete_preview_with_retry(dokploy_client, api_key, identifier).await {
        Ok(outcome) => {
            tracing::info!(identifier, ?outcome, "Preview deletion completed");
            Ok(outcome)
        }
        Err(error) => {
            tracing::error!(
                identifier,
                attempts = DELETE_RETRY_DELAYS.len() + 1,
                error = %error,
                "Preview deletion ultimately failed"
            );
            Err(error)
        }
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
    let identifier = spinploy::compute_identifier(&body.pr_id, &body.git_branch);
    delete_preview_internal(&dokploy_client, &api_key, &identifier)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

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
            let identifier = spinploy::compute_identifier(&pr_id, &branch);
            let repo_id = repo_id.clone();
            let pull_request_id = payload.resource.pull_request.pull_request_id;

            tracing::info!(identifier, "Queued preview deletion from Azure PR comment");
            // Spinploy has no durable work queue. This detaches Dokploy from Azure's request
            // lifecycle, but a process shutdown can still interrupt the task before it finishes.
            tokio::spawn(async move {
                let reply = match run_preview_deletion(&dokploy_client, &api_key, &identifier).await
                {
                    Ok(outcome) => match outcome {
                        DeletePreviewOutcome::Deleted => "🗑️ Preview deleted".to_string(),
                        DeletePreviewOutcome::AlreadyAbsent => {
                            "🗑️ Preview was already absent".to_string()
                        }
                    },
                    Err(_) => format!(
                        "⚠️ Could not delete preview `{identifier}` after retries. Please try `/delete` again."
                    ),
                };

                if let Err(error) = azure_client
                    .reply_in_thread(&repo_id, pull_request_id, thread_id, &reply)
                    .await
                {
                    tracing::warn!(
                        identifier,
                        error = %error,
                        "Failed to post ADO reply for /delete"
                    );
                }
            });

            Ok(StatusCode::ACCEPTED.into_response())
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
            let identifier = spinploy::compute_identifier(&pr_id, &branch);
            tracing::info!(identifier, "Queued preview deletion for completed Azure PR");
            tokio::spawn(async move {
                let _ = run_preview_deletion(&dokploy_client, &api_key, &identifier).await;
            });
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

    let failed_e2e_runs = failed_e2e_run_names(&timeline);

    if failed_e2e_runs.is_empty() {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }

    tracing::info!(
        build_id,
        build_number = build.build_number.as_deref().unwrap_or(""),
        failed_e2e_runs = ?failed_e2e_runs,
        "Tracked E2E runs failed; checking prior builds for regression"
    );

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
                            if !has_tracked_e2e_runs(&prev_tl) {
                                tracing::debug!(
                                    build_id,
                                    prev_build_id = b.id,
                                    "Previous build missing tracked E2E runs; continuing search"
                                );
                                continue;
                            }

                            let prev_failed_e2e_runs = failed_e2e_run_names(&prev_tl);

                            if failed_e2e_runs.is_subset(&prev_failed_e2e_runs) {
                                tracing::info!(
                                    build_id,
                                    prev_build_id = b.id,
                                    prev_failed_e2e_runs = ?prev_failed_e2e_runs,
                                    "Tracked E2E runs already failing in previous build; suppressing Slack"
                                );
                                return Ok(StatusCode::NO_CONTENT.into_response());
                            }

                            tracing::info!(
                                build_id,
                                prev_build_id = b.id,
                                prev_failed_e2e_runs = ?prev_failed_e2e_runs,
                                current_failed_e2e_runs = ?failed_e2e_runs,
                                "Previous build did not fail the same tracked E2E runs; treating as new regression"
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
        "*:warning: Playwright E2E failed*\n\n• 🏗️ Build: *{}* (ID `{}`)\n• 🧪 Stage: `Playwright E2E Tests`\n• ▶️ Failed runs: `{}`\n• 👤 Commit author: *{}*",
        build_number,
        build_id,
        format_tracked_e2e_runs(&failed_e2e_runs),
        commit.author.name
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::mpsc;
    use tower::ServiceExt;

    #[derive(Clone, Copy)]
    enum DokployDeleteBehavior {
        Succeeds,
        Fails,
        AlreadyAbsent,
    }

    struct DokployFixture {
        behavior: DokployDeleteBehavior,
        project_requests: AtomicUsize,
        delete_requests: AtomicUsize,
        deploy_requests: AtomicUsize,
    }

    async fn spawn_test_server(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let address = listener.local_addr().expect("test server address");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test API");
        });
        format!("http://{address}")
    }

    async fn spawn_dokploy_fixture(
        behavior: DokployDeleteBehavior,
    ) -> (String, Arc<DokployFixture>) {
        let fixture = Arc::new(DokployFixture {
            behavior,
            project_requests: AtomicUsize::new(0),
            delete_requests: AtomicUsize::new(0),
            deploy_requests: AtomicUsize::new(0),
        });
        let app = Router::new()
            .route(
                "/project.all",
                get(|State(fixture): State<Arc<DokployFixture>>| async move {
                    fixture.project_requests.fetch_add(1, Ordering::SeqCst);
                    let composes = match fixture.behavior {
                        DokployDeleteBehavior::AlreadyAbsent => vec![],
                        DokployDeleteBehavior::Succeeds | DokployDeleteBehavior::Fails => {
                            vec![serde_json::json!({
                                "composeId": "compose-id",
                                "name": "pr-42",
                                "appName": "preview-pr-42",
                                "environmentId": "environment-id"
                            })]
                        }
                    };
                    Json(serde_json::json!([{
                        "projectId": "project-id",
                        "name": "test-project",
                        "organizationId": "organization-id",
                        "environments": [{
                            "environmentId": "environment-id",
                            "name": "test",
                            "projectId": "project-id",
                            "compose": composes
                        }]
                    }]))
                }),
            )
            .route(
                "/compose.delete",
                post(|State(fixture): State<Arc<DokployFixture>>| async move {
                    fixture.delete_requests.fetch_add(1, Ordering::SeqCst);
                    match fixture.behavior {
                        DokployDeleteBehavior::Fails => {
                            (StatusCode::BAD_GATEWAY, "Dokploy unavailable")
                        }
                        DokployDeleteBehavior::Succeeds | DokployDeleteBehavior::AlreadyAbsent => {
                            (StatusCode::OK, "")
                        }
                    }
                }),
            )
            .route(
                "/compose.deploy",
                post(|State(fixture): State<Arc<DokployFixture>>| async move {
                    fixture.deploy_requests.fetch_add(1, Ordering::SeqCst);
                    StatusCode::OK
                }),
            )
            .route(
                "/domain.byComposeId",
                get(|| async { Json(serde_json::json!([])) }),
            )
            .with_state(Arc::clone(&fixture));
        (spawn_test_server(app).await, fixture)
    }

    async fn spawn_azure_reply_recorder() -> (String, mpsc::UnboundedReceiver<String>) {
        let (reply_tx, reply_rx) = mpsc::unbounded_channel();
        let app = Router::new().route(
            "/{*path}",
            post(move |Json(body): Json<serde_json::Value>| {
                let reply_tx = reply_tx.clone();
                async move {
                    let content = body["content"].as_str().unwrap_or_default().to_string();
                    reply_tx.send(content).expect("record Azure reply");
                    StatusCode::CREATED
                }
            }),
        );
        (spawn_test_server(app).await, reply_rx)
    }

    fn test_config(dokploy_url: String) -> Config {
        Config {
            dokploy_url,
            project_id: "project-id".to_string(),
            environment_id: "environment-id".to_string(),
            custom_git_url: "git@example.test:repo.git".to_string(),
            custom_git_ssh_key_id: "ssh-key-id".to_string(),
            compose_path: "compose.yml".to_string(),
            base_domain: "example.test".to_string(),
            frontend_service_name: "frontend".to_string(),
            frontend_port: 3000,
            backend_service_name: "backend".to_string(),
            backend_port: 8080,
            azdo_org: "organization".to_string(),
            azdo_project: "project".to_string(),
            azdo_repository_id: "repository-id".to_string(),
            azdo_pat: "azure-secret".to_string(),
            slack_webhook_url: "https://example.test/slack".to_string(),
            auth_cache_ttl_secs: 60,
            auth_cache_negative_ttl_secs: 10,
            storage: None,
            deployed_preview_api_path: "https://example.test/previews".to_string(),
        }
    }

    fn test_state(config: Config, azure_base_url: &str) -> AppState {
        AppState {
            dokploy_client: Arc::new(DokployClient::new(&config.dokploy_url)),
            azure_client: Arc::new(AzureDevOpsClient::with_base_url(
                azure_base_url,
                &config.azdo_org,
                &config.azdo_project,
                &config.azdo_pat,
            )),
            docker_client: None,
            slack_client: Arc::new(
                SlackWebhookClient::new(&config.slack_webhook_url).expect("Slack test client"),
            ),
            auth_cache: Arc::new(AuthCache::new(60, 10, 16)),
            pr_title_cache: Arc::new(PrTitleCache::new(60, 16)),
            config,
        }
    }

    fn pr_comment_body(content: &str) -> serde_json::Value {
        serde_json::json!({
            "eventType": "ms.vss-code.git-pullrequest-comment-event",
            "resource": {
                "comment": {
                    "content": content,
                    "isDeleted": false,
                    "_links": {
                        "threads": { "href": "https://example.test/threads/7" }
                    }
                },
                "pullRequest": {
                    "pullRequestId": 42,
                    "sourceRefName": "refs/heads/fix/delete"
                }
            }
        })
    }

    async fn send_pr_comment_webhook(state: AppState, content: &str) -> axum::response::Response {
        state
            .auth_cache
            .insert("dokploy-secret".to_string(), AuthDecision::Valid)
            .await;
        let app = Router::new()
            .route("/webhooks/azure/pr-comment", post(azure_pr_comment_webhook))
            .with_state(state);
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri("/webhooks/azure/pr-comment")
                .header("content-type", "application/json")
                .header("x-api-key", "dokploy-secret")
                .body(Body::from(pr_comment_body(content).to_string()))
                .expect("PR comment webhook request"),
        )
        .await
        .expect("PR comment webhook response")
    }

    fn timeline_record(name: &str, result: Option<&str>) -> AzureTimelineRecord {
        AzureTimelineRecord {
            name: name.to_string(),
            result: result.map(str::to_string),
            record_type: None,
            state: None,
        }
    }

    fn timeline(records: impl IntoIterator<Item = AzureTimelineRecord>) -> AzureBuildTimeline {
        AzureBuildTimeline {
            records: records.into_iter().collect(),
        }
    }

    #[test]
    fn collects_failed_legacy_and_split_e2e_runs() {
        let timeline = timeline([
            timeline_record("Build release artifacts", Some("failed")),
            timeline_record(LEGACY_E2E_RUN_NAME, Some("failed")),
            timeline_record(MAIN_E2E_RUN_NAME, Some("failed")),
            timeline_record(JOURNAL_TEMPLATE_E2E_RUN_NAME, Some("succeeded")),
        ]);

        let failed = failed_e2e_run_names(&timeline);

        assert_eq!(
            failed,
            BTreeSet::from([LEGACY_E2E_RUN_NAME, MAIN_E2E_RUN_NAME])
        );
    }

    #[test]
    fn formats_failed_runs_in_tracked_order() {
        let runs = BTreeSet::from([JOURNAL_TEMPLATE_E2E_RUN_NAME, MAIN_E2E_RUN_NAME]);

        assert_eq!(
            format_tracked_e2e_runs(&runs),
            "Run main E2E tests`, `Run journal template E2E tests"
        );
    }

    #[test]
    fn ignores_untracked_failures_when_matching_e2e_runs() {
        let timeline = timeline([
            timeline_record("Playwright E2E main", Some("failed")),
            timeline_record("Publish E2E JUnit", Some("failed")),
            timeline_record("Verify deployed UAT targets", Some("failed")),
        ]);

        assert!(!has_tracked_e2e_runs(&timeline));
        assert!(failed_e2e_run_names(&timeline).is_empty());
    }

    #[test]
    fn previous_build_must_cover_current_failed_runs_to_suppress_slack() {
        let current_failed = failed_e2e_run_names(&timeline([
            timeline_record(MAIN_E2E_RUN_NAME, Some("failed")),
            timeline_record(JOURNAL_TEMPLATE_E2E_RUN_NAME, Some("failed")),
        ]));
        let previous_same = timeline([
            timeline_record(MAIN_E2E_RUN_NAME, Some("failed")),
            timeline_record(JOURNAL_TEMPLATE_E2E_RUN_NAME, Some("failed")),
        ]);
        let previous_partial = timeline([timeline_record(
            JOURNAL_TEMPLATE_E2E_RUN_NAME,
            Some("failed"),
        )]);

        assert!(has_tracked_e2e_runs(&previous_same));
        assert!(current_failed.is_subset(&failed_e2e_run_names(&previous_same)));
        assert!(!current_failed.is_subset(&failed_e2e_run_names(&previous_partial)));
    }

    #[tokio::test]
    async fn delete_comment_acknowledges_dokploy_failure() {
        let (dokploy_url, fixture) = spawn_dokploy_fixture(DokployDeleteBehavior::Fails).await;
        let (azure_base_url, mut replies) = spawn_azure_reply_recorder().await;
        let state = test_state(test_config(dokploy_url), &azure_base_url);

        let response = send_pr_comment_webhook(state, "/delete").await;

        assert_eq!(response.status(), StatusCode::ACCEPTED);

        let reply = tokio::time::timeout(Duration::from_secs(3), replies.recv())
            .await
            .expect("background deletion should finish")
            .expect("Azure reply should be recorded");
        assert!(reply.contains("Could not delete preview"));
        assert!(!reply.contains("Dokploy unavailable"));
        assert_eq!(fixture.project_requests.load(Ordering::SeqCst), 3);
        assert_eq!(fixture.delete_requests.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn delete_comment_deletes_preview_and_replies_after_success() {
        let (dokploy_url, fixture) = spawn_dokploy_fixture(DokployDeleteBehavior::Succeeds).await;
        let (azure_base_url, mut replies) = spawn_azure_reply_recorder().await;
        let state = test_state(test_config(dokploy_url), &azure_base_url);

        let response = send_pr_comment_webhook(state, "/delete").await;

        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let reply = tokio::time::timeout(Duration::from_secs(1), replies.recv())
            .await
            .expect("background deletion should finish")
            .expect("Azure reply should be recorded");
        assert_eq!(reply, "🗑️ Preview deleted");
        assert_eq!(fixture.project_requests.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.delete_requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn repeated_delete_comments_succeed_when_preview_is_already_absent() {
        let (dokploy_url, fixture) =
            spawn_dokploy_fixture(DokployDeleteBehavior::AlreadyAbsent).await;
        let (azure_base_url, mut replies) = spawn_azure_reply_recorder().await;
        let state = test_state(test_config(dokploy_url), &azure_base_url);

        for _ in 0..2 {
            let response = send_pr_comment_webhook(state.clone(), "/delete").await;
            assert_eq!(response.status(), StatusCode::ACCEPTED);
        }

        for _ in 0..2 {
            let reply = tokio::time::timeout(Duration::from_secs(1), replies.recv())
                .await
                .expect("background deletion should finish")
                .expect("Azure reply should be recorded");
            assert_eq!(reply, "🗑️ Preview was already absent");
        }
        assert_eq!(fixture.project_requests.load(Ordering::SeqCst), 2);
        assert_eq!(fixture.delete_requests.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn preview_comment_behavior_is_unchanged() {
        let (dokploy_url, fixture) = spawn_dokploy_fixture(DokployDeleteBehavior::Succeeds).await;
        let (azure_base_url, mut replies) = spawn_azure_reply_recorder().await;
        let state = test_state(test_config(dokploy_url), &azure_base_url);

        let response = send_pr_comment_webhook(state, "/preview").await;

        assert_eq!(response.status(), StatusCode::OK);
        let reply = tokio::time::timeout(Duration::from_secs(1), replies.recv())
            .await
            .expect("preview reply should finish")
            .expect("Azure reply should be recorded");
        assert!(reply.contains("Preview building"));
        assert_eq!(fixture.deploy_requests.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.delete_requests.load(Ordering::SeqCst), 0);
    }
}
