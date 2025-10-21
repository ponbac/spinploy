use std::time::Duration;

use crate::api::{self, types};
use anyhow::{Context, Result, anyhow, bail};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::{info, instrument};

/// Lightweight wrapper around the Dokploy API.
///
/// Notes:
/// - API key is passed per-call and injected as `x-api-key` header.
/// - We call documented endpoints used by the legacy bash script.
/// - We intentionally keep request/response models minimal and dynamic (serde_json::Value)
///   to tolerate server-side changes without frequent client regenerations.
#[derive(Clone, Debug)]
pub struct DokployClient {
    base_url: String,
}

impl DokployClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
        }
    }

    pub fn from_env_url() -> Result<Self> {
        let base_url = std::env::var("DOKPLOY_URL").context("$DOKPLOY_URL not set")?;
        Ok(Self::new(base_url))
    }

    fn client_with_key(&self, api_key: &str) -> Result<api::Client> {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-api-key"),
            HeaderValue::from_str(api_key).context("invalid api key header")?,
        );

        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30))
            .default_headers(headers)
            .build()?;

        Ok(api::Client::new_with_client(&self.base_url, http))
    }

    /// Create or update a preview deployment and trigger a deploy
    #[instrument(skip(self, api_key, req), fields(identifier))]
    pub async fn deploy_preview(
        &self,
        api_key: &str,
        mut req: DeployPreviewRequest,
    ) -> Result<DeployPreviewResponse> {
        let identifier = compute_identifier(req.pr_number, req.branch_name.as_deref())
            .context("could not compute preview identifier (need pr_number or branch_name)")?;
        tracing::Span::current().record("identifier", tracing::field::display(&identifier));

        // Derive names/hosts if not provided
        let compose_name = req
            .compose_name
            .take()
            .unwrap_or_else(|| identifier.clone());
        let app_name_prefix = req
            .app_name_prefix
            .clone()
            .unwrap_or_else(|| "lerumpreviews-".to_string());
        let app_name = req
            .app_name
            .take()
            .unwrap_or_else(|| format!("{}{}", app_name_prefix, identifier));
        let (frontend_host, backend_host) = derive_hosts(
            req.frontend_host.as_deref(),
            req.backend_host.as_deref(),
            &identifier,
        );

        let frontend_service = req
            .frontend_service
            .take()
            .unwrap_or_else(|| "frontend".to_string());
        let backend_service = req
            .backend_service
            .take()
            .unwrap_or_else(|| "backend".to_string());
        let frontend_port = req.frontend_port.unwrap_or(3000);
        let backend_port = req.backend_port.unwrap_or(8080);
        let cookie_domain = req
            .cookie_domain
            .clone()
            .unwrap_or_else(|| ".d.bkmn.xyz".to_string());

        let env_string = format!(
            "APP_URL=https://{}\nBACKEND_API_URL=https://{}\nCOOKIE_DOMAIN={}",
            frontend_host, backend_host, cookie_domain
        );

        // Generated client configured per-call with API key
        let cli = self.client_with_key(api_key)?;

        // 1) Discover compose by name within project/environment
        let compose_id = find_compose_id(&cli, &req.project_id, &req.environment_id, &compose_name)
            .await
            .context("listing projects to locate compose")?;

        // 2) Create compose if missing
        let compose_id = match compose_id {
            Some(id) => {
                info!(%id, %compose_name, "compose exists");
                id
            }
            None => {
                info!(%compose_name, "compose not found, creating");
                let created = cli
                    .compose_create(&json!({
                        "name": compose_name,
                        "projectId": req.project_id,
                        "environmentId": req.environment_id,
                        "composeType": "docker-compose",
                        "appName": app_name,
                    }))
                    .await
                    .context("compose.create failed")?
                    .into_inner();
                // Accept various shapes
                let v: Value = serde_json::to_value(created).unwrap_or(Value::Null);
                v.get("composeId")
                    .or_else(|| v.get("id"))
                    .or_else(|| v.get("compose").and_then(|c| c.get("composeId")))
                    .and_then(Value::as_str)
                    .map(|s| s.to_string())
                    .ok_or_else(|| anyhow!("compose.create response missing composeId"))?
            }
        };

        // 3) Update compose (git config, env, flags)
        cli.compose_update(&json!({
            "composeId": compose_id,
            "name": compose_name,
            "appName": app_name,
            "env": env_string,
            "sourceType": "git",
            "composeType": "docker-compose",
            "customGitUrl": req.custom_git_url,
            "customGitBranch": req.custom_git_branch,
            "customGitSSHKeyId": req.custom_git_ssh_key_id,
            "composePath": req.compose_path,
            "environmentId": req.environment_id,
            "autoDeploy": true,
            "isolatedDeployment": true
        }))
        .await
        .context("compose.update failed")?;

        // 4) Upsert domains for frontend/backend
        upsert_domain_for_service(
            &cli,
            &compose_id,
            &frontend_host,
            &frontend_service,
            frontend_port,
        )
        .await
        .context("upserting frontend domain failed")?;

        upsert_domain_for_service(
            &cli,
            &compose_id,
            &backend_host,
            &backend_service,
            backend_port,
        )
        .await
        .context("upserting backend domain failed")?;

        // 5) Deploy
        cli.compose_deploy(&json!({ "composeId": compose_id }))
            .await
            .context("compose.deploy failed")?;

        Ok(DeployPreviewResponse {
            compose_id,
            frontend_url: format!("https://{}", frontend_host),
            backend_url: format!("https://{}", backend_host),
        })
    }

    /// Delete preview deployment (if it exists). Always deletes volumes.
    #[instrument(skip(self, api_key))]
    pub async fn delete_preview(
        &self,
        api_key: &str,
        project_id: &str,
        environment_id: &str,
        compose_name: &str,
    ) -> Result<()> {
        let cli = self.client_with_key(api_key)?;
        let compose_id = find_compose_id(&cli, project_id, environment_id, compose_name)
            .await
            .context("listing projects to locate compose")?;
        let Some(compose_id) = compose_id else {
            info!(%compose_name, "compose not found, nothing to delete");
            return Ok(());
        };
        cli.compose_delete(&json!({ "composeId": compose_id, "deleteVolumes": true }))
            .await
            .context("compose.delete failed")?;
        Ok(())
    }
}

// === Public request/response models ===

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeployPreviewRequest {
    pub project_id: String,
    pub environment_id: String,

    pub pr_number: Option<u64>,
    pub branch_name: Option<String>,

    pub custom_git_url: String,
    pub custom_git_branch: String,
    pub custom_git_ssh_key_id: String,
    pub compose_path: String,

    // Optional overrides
    pub compose_name: Option<String>,
    pub app_name: Option<String>,
    pub app_name_prefix: Option<String>,
    pub frontend_host: Option<String>,
    pub backend_host: Option<String>,
    pub frontend_service: Option<String>,
    pub frontend_port: Option<u16>,
    pub backend_service: Option<String>,
    pub backend_port: Option<u16>,
    pub cookie_domain: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeployPreviewResponse {
    pub compose_id: String,
    pub frontend_url: String,
    pub backend_url: String,
}

// === Internal helpers ===

fn derive_hosts(
    frontend_host: Option<&str>,
    backend_host: Option<&str>,
    identifier: &str,
) -> (String, String) {
    let fe = frontend_host
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{}.d.bkmn.xyz", identifier));
    let be = backend_host
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("api-{}.d.bkmn.xyz", identifier));
    (fe, be)
}

fn sanitize_branch_for_identifier(branch: &str) -> String {
    let lowered = branch.to_ascii_lowercase();
    lowered.replace('/', "-")
}

pub fn compute_identifier(pr_number: Option<u64>, branch_name: Option<&str>) -> Result<String> {
    if let Some(pr) = pr_number {
        return Ok(format!("pr-{}", pr));
    }
    let Some(branch) = branch_name else {
        bail!("missing pr_number and branch_name")
    };
    let safe = sanitize_branch_for_identifier(branch);
    if safe.is_empty() {
        bail!("branch name empty after sanitization")
    }
    Ok(format!("br-{}", safe))
}

// (previous Http adapter removed; we use the generated client)

// === Dokploy operations via generated client ===

async fn find_compose_id(
    cli: &api::Client,
    project_id: &str,
    environment_id: &str,
    compose_name: &str,
) -> Result<Option<String>> {
    let projects = cli.project_all().await?.into_inner();
    for p in projects.into_iter() {
        if p.project_id == project_id {
            for env in p.environments.unwrap_or_default().into_iter() {
                if env.environment_id == environment_id {
                    for c in env.compose.unwrap_or_default().into_iter() {
                        if c.name == compose_name {
                            return Ok(Some(c.compose_id));
                        }
                    }
                }
            }
        }
    }
    Ok(None)
}

async fn upsert_domain_for_service(
    cli: &api::Client,
    compose_id: &str,
    host: &str,
    service_name: &str,
    port: u16,
) -> Result<()> {
    let domains = cli
        .domain_by_compose_id(&json!({ "composeId": compose_id }))
        .await
        .map(|r| r.into_inner())
        .unwrap_or_default();

    if let Some(d) = domains.into_iter().find(|d| d.service_name == service_name) {
        cli.domain_update(&json!({
            "domainId": d.domain_id,
            "host": host,
            "path": "/",
            "port": port,
            "https": true,
            "certificateType": "none",
            "serviceName": service_name,
            "domainType": "compose"
        }))
        .await?;
    } else {
        cli.domain_create(&json!({
            "host": host,
            "path": "/",
            "port": port,
            "https": true,
            "certificateType": "none",
            "composeId": compose_id,
            "serviceName": service_name,
            "domainType": "compose"
        }))
        .await?;
    }
    Ok(())
}
