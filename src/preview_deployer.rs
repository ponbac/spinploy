use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_yaml::{Mapping, Value};
use tokio::process::Command;
use tokio::sync::{Mutex, RwLock, Semaphore};

use crate::azure_client::AzureDevOpsClient;
use crate::config::Config;
use crate::dokploy_client::DokployClient;
use crate::models::dokploy::{Compose, DomainCreateRequest, UpdateRawComposeRequest};

const PREVIEW_LIMIT: usize = 3;
const PROJECT_ENVIRONMENT: &str = r#"COOKIE_DOMAIN=${{project.COOKIE_DOMAIN}}
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
IMAGE_ANALYSIS_API_KEY=${{project.IMAGE_ANALYSIS_API_KEY}}"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewReconcileStatus {
    Building,
    Running,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewReconcileSnapshot {
    pub identifier: String,
    pub git_branch: String,
    pub pr_id: Option<String>,
    pub status: PreviewReconcileStatus,
    pub requested_at: String,
}

#[derive(Clone, Debug)]
pub struct QueuedPreview {
    pub identifier: String,
    pub frontend_domain: String,
    pub dashboard_domain: String,
}

#[derive(Clone, Debug)]
struct PreviewJob {
    identifier: String,
    git_branch: String,
    pr_id: Option<String>,
    api_key: String,
    generation: u64,
}

#[derive(Default)]
struct QueueState {
    desired: HashMap<String, PreviewJob>,
    active: HashSet<String>,
    deleting: HashSet<String>,
    generations: HashMap<String, u64>,
}

pub struct PreviewDeployer {
    config: Config,
    azure_client: Arc<AzureDevOpsClient>,
    dokploy_client: Arc<DokployClient>,
    queue: Mutex<QueueState>,
    statuses: RwLock<HashMap<String, PreviewReconcileSnapshot>>,
    operation_lock: Semaphore,
    readiness_client: reqwest::Client,
}

impl PreviewDeployer {
    pub fn new(
        config: Config,
        azure_client: Arc<AzureDevOpsClient>,
        dokploy_client: Arc<DokployClient>,
    ) -> Result<Self> {
        ensure!(
            Path::new(&config.preview_work_dir).is_absolute(),
            "PREVIEW_WORK_DIR must be absolute"
        );
        ensure!(
            Path::new(&config.preview_host_work_dir).is_absolute(),
            "PREVIEW_HOST_WORK_DIR must be absolute"
        );
        ensure!(
            Path::new(&config.preview_cache_dir).is_absolute(),
            "PREVIEW_CACHE_DIR must be absolute"
        );
        ensure!(
            Path::new(&config.preview_host_cache_dir).is_absolute(),
            "PREVIEW_HOST_CACHE_DIR must be absolute"
        );

        let readiness_client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(15))
            .build()
            .context("failed to create preview readiness client")?;

        Ok(Self {
            config,
            azure_client,
            dokploy_client,
            queue: Mutex::new(QueueState::default()),
            statuses: RwLock::new(HashMap::new()),
            operation_lock: Semaphore::new(1),
            readiness_client,
        })
    }

    /// Queue a reconcile and coalesce any newer request for the same preview.
    /// A single global permit intentionally limits this VM to one artifact build at a time.
    pub async fn enqueue(
        self: &Arc<Self>,
        api_key: String,
        git_branch: String,
        pr_id: Option<String>,
    ) -> Result<QueuedPreview> {
        self.enqueue_internal(api_key, git_branch, pr_id, true)
            .await?
            .context("manual preview request was unexpectedly ignored")
    }

    /// Queue a new revision only when the preview already exists or is still being built.
    /// PR update webhooks use this path so they can never create previews automatically.
    pub async fn enqueue_existing(
        self: &Arc<Self>,
        api_key: String,
        git_branch: String,
        pr_id: Option<String>,
    ) -> Result<Option<QueuedPreview>> {
        let identifier = crate::compute_identifier(&pr_id, &git_branch);
        validate_identifier(&identifier)?;

        let is_active = {
            let queue = self.queue.lock().await;
            if queue.deleting.contains(&identifier) {
                return Ok(None);
            }
            queue.active.contains(&identifier)
        };
        if !is_active
            && self
                .dokploy_client
                .find_compose_by_name(&api_key, &identifier)
                .await?
                .is_none()
        {
            return Ok(None);
        }

        self.enqueue_internal(api_key, git_branch, pr_id, false)
            .await
    }

    async fn enqueue_internal(
        self: &Arc<Self>,
        api_key: String,
        git_branch: String,
        pr_id: Option<String>,
        is_manual_request: bool,
    ) -> Result<Option<QueuedPreview>> {
        let identifier = crate::compute_identifier(&pr_id, &git_branch);
        validate_identifier(&identifier)?;
        let snapshot = PreviewReconcileSnapshot {
            identifier: identifier.clone(),
            git_branch: git_branch.clone(),
            pr_id: pr_id.clone(),
            status: PreviewReconcileStatus::Building,
            requested_at: chrono::Utc::now().to_rfc3339(),
        };

        let should_start_worker = {
            let mut queue = self.queue.lock().await;
            if queue.deleting.contains(&identifier) {
                if is_manual_request {
                    // An explicit /preview after /delete is the newest user intent.
                    queue.deleting.remove(&identifier);
                } else {
                    return Ok(None);
                }
            }
            let generation = queue
                .generations
                .entry(identifier.clone())
                .and_modify(|value| *value += 1)
                .or_insert(1)
                .to_owned();
            queue.desired.insert(
                identifier.clone(),
                PreviewJob {
                    identifier: identifier.clone(),
                    git_branch,
                    pr_id,
                    api_key,
                    generation,
                },
            );
            queue.active.insert(identifier.clone())
        };

        self.statuses
            .write()
            .await
            .insert(identifier.clone(), snapshot);

        if should_start_worker {
            let deployer = Arc::clone(self);
            let worker_identifier = identifier.clone();
            tokio::spawn(async move {
                deployer.run_worker(worker_identifier).await;
            });
        }

        Ok(Some(QueuedPreview {
            frontend_domain: self.frontend_domain(&identifier),
            dashboard_domain: self.dashboard_domain(&identifier),
            identifier,
        }))
    }

    pub async fn status(&self, identifier: &str) -> Option<PreviewReconcileStatus> {
        self.statuses
            .read()
            .await
            .get(identifier)
            .map(|snapshot| snapshot.status)
    }

    pub async fn snapshot(&self, identifier: &str) -> Option<PreviewReconcileSnapshot> {
        self.statuses.read().await.get(identifier).cloned()
    }

    pub async fn snapshots(&self) -> Vec<PreviewReconcileSnapshot> {
        self.statuses.read().await.values().cloned().collect()
    }

    pub async fn is_ready(&self, identifier: &str) -> bool {
        self.check_health(&self.frontend_url(identifier)).await
    }

    /// Cancel queued work, wait for any active reconcile to leave its critical section,
    /// and delete the Dokploy compose including volumes.
    pub async fn delete(&self, api_key: &str, identifier: &str) -> Result<bool> {
        validate_identifier(identifier)?;
        {
            let mut queue = self.queue.lock().await;
            queue.desired.remove(identifier);
            queue.active.remove(identifier);
            queue.deleting.insert(identifier.to_string());
            queue
                .generations
                .entry(identifier.to_string())
                .and_modify(|value| *value += 1)
                .or_insert(1);
        }

        let _permit = self
            .operation_lock
            .acquire()
            .await
            .context("preview operation lock closed")?;
        let deletion = self.delete_compose_if_present(api_key, identifier).await;
        if deletion.is_err() {
            self.queue.lock().await.deleting.remove(identifier);
            return deletion;
        }
        let deleted = deletion.expect("successful deletion contains its outcome");
        self.cleanup_preview_networks(identifier).await;
        self.remove_preview_workspace(identifier).await;
        self.cleanup_preview_images(identifier, None).await;
        let remove_status = {
            let mut queue = self.queue.lock().await;
            queue.deleting.remove(identifier);
            !queue.desired.contains_key(identifier)
        };
        if remove_status {
            self.statuses.write().await.remove(identifier);
        }
        Ok(deleted)
    }

    async fn run_worker(self: Arc<Self>, identifier: String) {
        loop {
            let job = {
                let mut queue = self.queue.lock().await;
                match queue.desired.remove(&identifier) {
                    Some(job) => job,
                    None => {
                        queue.active.remove(&identifier);
                        return;
                    }
                }
            };

            let permit = match self.operation_lock.acquire().await {
                Ok(permit) => permit,
                Err(error) => {
                    tracing::error!(identifier, error = %error, "Preview operation lock closed");
                    return;
                }
            };

            if !self.is_current(&job).await {
                drop(permit);
                continue;
            }

            tracing::info!(
                identifier,
                branch = job.git_branch,
                generation = job.generation,
                "Starting VM-local Aspire preview reconcile"
            );
            let result = self.reconcile(&job).await;
            drop(permit);

            if self.is_current(&job).await {
                let status = match &result {
                    Ok(true) => PreviewReconcileStatus::Running,
                    Ok(false) => PreviewReconcileStatus::Building,
                    Err(_) => PreviewReconcileStatus::Failed,
                };
                if let Some(snapshot) = self.statuses.write().await.get_mut(&identifier) {
                    snapshot.status = status;
                }
            }

            match result {
                Ok(true) => tracing::info!(identifier, "Preview reconcile completed and is ready"),
                Ok(false) => tracing::info!(
                    identifier,
                    "Preview reconcile superseded by a newer revision"
                ),
                Err(error) => {
                    tracing::error!(identifier, error = %error, "Preview reconcile failed")
                }
            }
        }
    }

    async fn reconcile(&self, job: &PreviewJob) -> Result<bool> {
        let revision = self.resolve_revision(job).await?;
        validate_revision(&revision)?;
        tracing::info!(
            identifier = job.identifier,
            revision,
            "Resolved preview revision"
        );

        self.reserve_preview_slot(job).await?;

        if !self.is_current(job).await {
            return Ok(false);
        }

        let source = self
            .download_and_extract_source(&job.identifier, &revision)
            .await?;
        if !self.build_artifacts(job, &revision, &source).await? {
            return Ok(false);
        }

        if !self.is_current(job).await {
            return Ok(false);
        }

        let compose_path = source
            .inside
            .join(".spinploy/artifacts/docker-compose.yaml");
        let compose_file = tokio::fs::read_to_string(&compose_path)
            .await
            .with_context(|| format!("failed to read {}", compose_path.display()))?;
        let compose_file = normalize_compose_labels_for_dokploy(&compose_file)?;
        ensure!(
            compose_file.contains("spinploy.managed"),
            "generated Compose file is missing Spinploy ownership labels"
        );

        self.replace_dokploy_compose(job, &revision, compose_file)
            .await?;

        if !self.is_current(job).await {
            return Ok(false);
        }

        if !self.wait_until_ready(job, &revision).await? {
            return Ok(false);
        }
        self.cleanup_preview_images(&job.identifier, Some(&revision))
            .await;
        if let Err(error) = tokio::fs::remove_dir_all(&source.inside).await {
            tracing::warn!(
                identifier = job.identifier,
                revision,
                error = %error,
                "Failed to clean successful preview source workspace"
            );
        }
        Ok(true)
    }

    async fn resolve_revision(&self, job: &PreviewJob) -> Result<String> {
        if let Some(pr_id) = &job.pr_id {
            let pr_id = pr_id.parse::<u64>().context("PR id must be numeric")?;
            let pull_request = self
                .azure_client
                .get_pull_request(&self.config.azdo_repository_id, pr_id)
                .await
                .with_context(|| format!("failed to fetch Azure PR {pr_id}"))?;
            return Ok(pull_request.last_merge_source_commit.commit_id);
        }

        self.azure_client
            .get_branch_head(&self.config.azdo_repository_id, &job.git_branch)
            .await
    }

    async fn download_and_extract_source(
        &self,
        identifier: &str,
        revision: &str,
    ) -> Result<SourcePaths> {
        let inside = PathBuf::from(&self.config.preview_work_dir)
            .join(identifier)
            .join(revision)
            .join("source");
        let host = PathBuf::from(&self.config.preview_host_work_dir)
            .join(identifier)
            .join(revision)
            .join("source");

        if tokio::fs::try_exists(&inside).await? {
            tokio::fs::remove_dir_all(&inside)
                .await
                .with_context(|| format!("failed to clear {}", inside.display()))?;
        }
        tokio::fs::create_dir_all(&inside)
            .await
            .with_context(|| format!("failed to create {}", inside.display()))?;

        let archive = self
            .azure_client
            .download_repository_archive(&self.config.azdo_repository_id, revision)
            .await
            .with_context(|| format!("failed to download Azure revision {revision}"))?;
        let extraction_path = inside.clone();
        tokio::task::spawn_blocking(move || extract_zip_archive(&archive, &extraction_path))
            .await
            .context("source extraction task failed")??;

        let apphost = inside.join(&self.config.preview_apphost_path);
        ensure!(
            tokio::fs::try_exists(&apphost).await?,
            "archive does not contain expected AppHost {}",
            self.config.preview_apphost_path
        );

        Ok(SourcePaths { inside, host })
    }

    async fn build_artifacts(
        &self,
        job: &PreviewJob,
        revision: &str,
        source: &SourcePaths,
    ) -> Result<bool> {
        let identifier = &job.identifier;
        let inside_nuget_cache = PathBuf::from(&self.config.preview_cache_dir).join("nuget");
        let host_nuget_cache = PathBuf::from(&self.config.preview_host_cache_dir).join("nuget");
        let inside_pnpm_cache = PathBuf::from(&self.config.preview_cache_dir).join("pnpm");
        let host_pnpm_cache = PathBuf::from(&self.config.preview_host_cache_dir).join("pnpm");
        tokio::fs::create_dir_all(&inside_nuget_cache)
            .await
            .with_context(|| format!("failed to create {}", inside_nuget_cache.display()))?;
        tokio::fs::create_dir_all(&inside_pnpm_cache)
            .await
            .with_context(|| format!("failed to create {}", inside_pnpm_cache.display()))?;

        let builder_name = format!(
            "spinploy-builder-{}-{}",
            identifier,
            revision.chars().take(12).collect::<String>()
        );
        let _ = Command::new("docker")
            .args(["rm", "--force", &builder_name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
        let builder_image = self.builder_image().await?;

        let script = r#"set -euo pipefail
apphost="$1"
output="$2"
app_url="$3"
revision="$4"
identifier="$5"
frontend="$6"
mkdir -p "$output"
pnpm --dir "$frontend" install --frozen-lockfile --store-dir /workspace/.pnpm-store
aspire do prepare-preview \
  --apphost "$apphost" \
  --output-path "$output" \
  --pipeline-log-level information \
  --non-interactive \
  --nologo \
  -- \
  --Apport:Preview=true \
  "--Apport:Preview:AppUrl=$app_url" \
  "--Apport:Preview:Revision=$revision" \
  "--Apport:Preview:Identifier=$identifier""#;

        let source_mount = format!("{}:/workspace", source.host.display());
        let nuget_mount = format!("{}:/root/.nuget/packages", host_nuget_cache.display());
        let pnpm_mount = format!("{}:/workspace/.pnpm-store", host_pnpm_cache.display());
        let app_url = self.frontend_url(identifier);
        let mut command = Command::new("docker");
        command
            .args(["run", "--rm", "--name", &builder_name])
            .args(["--label", "spinploy.builder=true"])
            .args(["--volume", "/var/run/docker.sock:/var/run/docker.sock"])
            .args(["--volume", &source_mount])
            .args(["--volume", &nuget_mount])
            .args(["--volume", &pnpm_mount])
            .args(["--workdir", "/workspace"])
            .args(["--env", "CI=true"])
            .args(["--env", "DOTNET_CLI_TELEMETRY_OPTOUT=1"])
            .args(["--entrypoint", "/bin/bash"])
            .arg(&builder_image)
            .args([
                "-lc",
                script,
                "preview-build",
                &self.config.preview_apphost_path,
                ".spinploy/artifacts",
                &app_url,
                revision,
                identifier,
                &self.config.preview_frontend_path,
            ])
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);

        let mut child = command.spawn().context("failed to start preview builder")?;
        let timeout = Duration::from_secs(self.config.preview_build_timeout_secs);
        let deadline = tokio::time::Instant::now() + timeout;
        let status = loop {
            if let Some(status) = child
                .try_wait()
                .context("failed while waiting for preview builder")?
            {
                break status;
            }
            if !self.is_current(job).await {
                let _ = child.kill().await;
                let _ = Command::new("docker")
                    .args(["rm", "--force", &builder_name])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .await;
                return Ok(false);
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = child.kill().await;
                let _ = Command::new("docker")
                    .args(["rm", "--force", &builder_name])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .await;
                bail!("preview build exceeded {} seconds", timeout.as_secs());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        };
        ensure!(status.success(), "preview builder exited with {status}");
        Ok(true)
    }

    async fn builder_image(&self) -> Result<String> {
        if let Some(image) = self
            .config
            .preview_builder_image
            .as_deref()
            .filter(|image| !image.trim().is_empty())
        {
            return Ok(image.to_string());
        }

        let hostname = std::env::var("HOSTNAME")
            .context("PREVIEW_BUILDER_IMAGE is unset and HOSTNAME is unavailable")?;
        let output = Command::new("docker")
            .args(["inspect", "--format", "{{.Config.Image}}", &hostname])
            .output()
            .await
            .context("failed to inspect the Spinploy container image")?;
        ensure!(
            output.status.success(),
            "PREVIEW_BUILDER_IMAGE is unset and the current container image could not be inspected"
        );
        let image = String::from_utf8(output.stdout)
            .context("Docker returned a non-UTF-8 image name")?
            .trim()
            .to_string();
        ensure!(
            !image.is_empty(),
            "Docker returned an empty builder image name"
        );
        Ok(image)
    }

    async fn replace_dokploy_compose(
        &self,
        job: &PreviewJob,
        revision: &str,
        compose_file: String,
    ) -> Result<String> {
        self.delete_compose_if_present(&job.api_key, &job.identifier)
            .await?;
        self.cleanup_preview_networks(&job.identifier).await;

        let app_name = format!("preview-{}", job.identifier);
        let compose = self
            .dokploy_client
            .create_compose(
                &job.api_key,
                &self.config.environment_id,
                &job.identifier,
                &app_name,
            )
            .await
            .context("failed to create Dokploy compose")?;
        let compose_file = add_traefik_network_labels_for_dokploy(
            &compose_file,
            &compose.app_name,
            &[&self.config.frontend_service_name, "preview-dashboard"],
        )?;

        self.dokploy_client
            .update_raw_compose(
                &job.api_key,
                UpdateRawComposeRequest {
                    compose_id: compose.compose_id.clone(),
                    name: job.identifier.clone(),
                    app_name,
                    env: self.compose_environment(&job.identifier, revision),
                    source_type: "raw".to_string(),
                    compose_type: "docker-compose".to_string(),
                    compose_file,
                    environment_id: self.config.environment_id.clone(),
                    auto_deploy: false,
                    isolated_deployment: true,
                },
            )
            .await
            .context("failed to configure raw Dokploy compose")?;

        self.create_domain(
            &job.api_key,
            &compose.compose_id,
            &self.config.frontend_service_name,
            self.config.frontend_port,
            self.frontend_domain(&job.identifier),
        )
        .await?;
        self.create_domain(
            &job.api_key,
            &compose.compose_id,
            "preview-dashboard",
            18888,
            self.dashboard_domain(&job.identifier),
        )
        .await?;

        self.dokploy_client
            .deploy_compose(&job.api_key, &compose.compose_id)
            .await
            .context("failed to deploy Dokploy compose")?;
        Ok(compose.compose_id)
    }

    async fn create_domain(
        &self,
        api_key: &str,
        compose_id: &str,
        service_name: &str,
        port: u16,
        host: String,
    ) -> Result<()> {
        self.dokploy_client
            .create_domain(
                api_key,
                DomainCreateRequest {
                    compose_id: compose_id.to_string(),
                    service_name: service_name.to_string(),
                    domain_type: "compose".to_string(),
                    host,
                    path: "/".to_string(),
                    port,
                    https: true,
                    certificate_type: "none".to_string(),
                },
            )
            .await
            .context("failed to create Dokploy domain")
    }

    async fn delete_compose_if_present(&self, api_key: &str, identifier: &str) -> Result<bool> {
        let Some(compose) = self.find_compose_with_retry(api_key, identifier).await? else {
            return Ok(false);
        };

        let mut last_error = None;
        for attempt in 1..=3 {
            let delete_error = self
                .dokploy_client
                .delete_compose(api_key, &compose.compose_id, true)
                .await
                .err();
            let poll_count = if delete_error.is_some() { 2 } else { 20 };
            if let Some(error) = delete_error {
                last_error = Some(error);
            }
            for _ in 0..poll_count {
                match self.find_compose_with_retry(api_key, identifier).await {
                    Ok(None) => return Ok(true),
                    Ok(Some(_)) => {}
                    Err(error) => {
                        last_error = Some(error.context("failed to confirm Compose deletion"));
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }

            if attempt < 3 {
                tokio::time::sleep(Duration::from_millis(250 * attempt)).await;
            }
        }

        match last_error {
            Some(error) => Err(error.context("Dokploy compose deletion failed")),
            None => bail!("Dokploy compose still exists after deletion"),
        }
    }

    async fn find_compose_with_retry(
        &self,
        api_key: &str,
        identifier: &str,
    ) -> Result<Option<Compose>> {
        let mut last_error = None;
        for attempt in 1..=3 {
            match self
                .dokploy_client
                .find_compose_by_name(api_key, identifier)
                .await
            {
                Ok(compose) => return Ok(compose),
                Err(error) => {
                    tracing::warn!(
                        identifier,
                        attempt,
                        error = %error,
                        "Dokploy Compose lookup failed"
                    );
                    last_error = Some(error);
                }
            }

            if attempt < 3 {
                tokio::time::sleep(Duration::from_millis(250 * attempt)).await;
            }
        }

        Err(last_error
            .expect("a failed retry loop records its last error")
            .context("Dokploy Compose lookup failed after retries"))
    }

    async fn wait_until_ready(&self, job: &PreviewJob, revision: &str) -> Result<bool> {
        let url = self.frontend_url(&job.identifier);
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(self.config.preview_readiness_timeout_secs);

        while tokio::time::Instant::now() < deadline {
            if !self.is_current(job).await {
                return Ok(false);
            }
            if self.check_health(&url).await && self.check_revision(&url, revision).await {
                return Ok(true);
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }

        bail!(
            "preview did not become healthy at {} within {} seconds",
            url,
            self.config.preview_readiness_timeout_secs
        )
    }

    /// Remove enough old previews to reserve one of the three slots before any
    /// source download or artifact build starts for a new preview.
    async fn reserve_preview_slot(&self, job: &PreviewJob) -> Result<()> {
        if self
            .dokploy_client
            .find_compose_by_name(&job.api_key, &job.identifier)
            .await?
            .is_some()
        {
            return Ok(());
        }

        let composes = self
            .dokploy_client
            .list_composes_with_prefix(&job.api_key, &self.config.environment_id, "preview-")
            .await
            .context("failed to list previews before reserving a build slot")?;

        let to_delete = previews_to_remove_before_creation(composes.len());
        if to_delete == 0 {
            return Ok(());
        }

        let mut detailed = futures::future::join_all(composes.into_iter().map(|compose| async {
            let detail = self
                .dokploy_client
                .get_compose_detail(&job.api_key, &compose.compose_id)
                .await;
            (compose, detail)
        }))
        .await;
        detailed.sort_by_key(|(compose, detail)| {
            detail
                .as_ref()
                .ok()
                .and_then(|detail| {
                    detail
                        .deployments
                        .iter()
                        .filter_map(|deployment| {
                            deployment
                                .finished_at
                                .as_deref()
                                .or(deployment.started_at.as_deref())
                                .or(deployment.created_at.as_deref())
                        })
                        .filter_map(crate::parse_ts)
                        .max()
                })
                .or_else(|| compose.created_at.as_deref().and_then(crate::parse_ts))
        });

        for (compose, _) in detailed.into_iter().take(to_delete) {
            self.mark_preview_deleting(&compose.name).await;
            let deletion = self
                .delete_compose_if_present(&job.api_key, &compose.name)
                .await
                .with_context(|| format!("failed to remove oldest preview {}", compose.name));
            if let Err(error) = deletion {
                self.finish_pruned_preview(&compose.name).await;
                return Err(error);
            }
            self.cleanup_preview_networks(&compose.name).await;
            self.remove_preview_workspace(&compose.name).await;
            self.cleanup_preview_images(&compose.name, None).await;
            self.finish_pruned_preview(&compose.name).await;
        }

        Ok(())
    }

    async fn mark_preview_deleting(&self, identifier: &str) {
        let mut queue = self.queue.lock().await;
        queue.desired.remove(identifier);
        queue.active.remove(identifier);
        queue.deleting.insert(identifier.to_string());
        queue
            .generations
            .entry(identifier.to_string())
            .and_modify(|value| *value += 1)
            .or_insert(1);
    }

    async fn finish_pruned_preview(&self, identifier: &str) {
        let remove_status = {
            let mut queue = self.queue.lock().await;
            queue.deleting.remove(identifier);
            !queue.desired.contains_key(identifier)
        };
        if remove_status {
            self.statuses.write().await.remove(identifier);
        }
    }

    async fn remove_preview_workspace(&self, identifier: &str) {
        if let Err(error) = validate_identifier(identifier) {
            tracing::warn!(identifier, error = %error, "Refusing to remove an invalid preview workspace");
            return;
        }
        let path = PathBuf::from(&self.config.preview_work_dir).join(identifier);
        if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
            return;
        }
        if let Err(error) = tokio::fs::remove_dir_all(&path).await {
            tracing::warn!(
                identifier,
                error = %error,
                "Failed to remove preview workspace"
            );
        }
    }

    async fn cleanup_preview_networks(&self, identifier: &str) {
        if let Err(error) = validate_identifier(identifier) {
            tracing::warn!(identifier, error = %error, "Refusing to remove networks for an invalid preview identifier");
            return;
        }

        let output = match Command::new("docker")
            .args(["network", "ls", "--format", "{{.Name}}"])
            .output()
            .await
        {
            Ok(output) if output.status.success() => output,
            Ok(_) | Err(_) => {
                tracing::warn!(identifier, "Failed to list stale preview networks");
                return;
            }
        };
        let Ok(networks) = String::from_utf8(output.stdout) else {
            tracing::warn!(identifier, "Docker returned non-UTF-8 network names");
            return;
        };

        for network in networks
            .lines()
            .filter(|network| is_preview_network_name(identifier, network))
        {
            let members = match docker_network_members(network).await {
                Ok(members) => members,
                Err(error) => {
                    tracing::warn!(identifier, network, error = %error, "Failed to inspect stale preview network");
                    continue;
                }
            };
            let unrelated_members = members
                .iter()
                .filter(|member| member.as_str() != "dokploy-traefik")
                .cloned()
                .collect::<Vec<_>>();
            if !unrelated_members.is_empty() {
                tracing::warn!(
                    identifier,
                    network,
                    members = ?unrelated_members,
                    "Refusing to remove a preview network with unrelated containers"
                );
                continue;
            }

            if members.iter().any(|member| member == "dokploy-traefik") {
                let status = Command::new("docker")
                    .args([
                        "network",
                        "disconnect",
                        "--force",
                        network,
                        "dokploy-traefik",
                    ])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .await;
                if !matches!(status, Ok(status) if status.success()) {
                    tracing::warn!(
                        identifier,
                        network,
                        "Failed to disconnect Dokploy Traefik from stale preview network"
                    );
                    continue;
                }
            }

            let status = Command::new("docker")
                .args(["network", "rm", network])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
            if !matches!(status, Ok(status) if status.success()) {
                tracing::warn!(
                    identifier,
                    network,
                    "Failed to remove stale preview network"
                );
            }
        }
    }

    async fn cleanup_preview_images(&self, identifier: &str, keep_revision: Option<&str>) {
        let reference_filter = format!("reference=preview-{identifier}-*");
        let output = match Command::new("docker")
            .args([
                "image",
                "ls",
                "--filter",
                &reference_filter,
                "--format",
                "{{.Repository}}:{{.Tag}}",
            ])
            .output()
            .await
        {
            Ok(output) if output.status.success() => output,
            Ok(_) | Err(_) => {
                tracing::warn!(identifier, "Failed to list stale preview images");
                return;
            }
        };

        let Ok(references) = String::from_utf8(output.stdout) else {
            tracing::warn!(identifier, "Docker returned non-UTF-8 image references");
            return;
        };
        for reference in references.lines().filter(|reference| {
            keep_revision
                .map(|revision| {
                    !reference.ends_with(&format!(":{}", revision.to_ascii_lowercase()))
                })
                .unwrap_or(true)
        }) {
            let status = Command::new("docker")
                .args(["image", "rm", reference])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
            if !matches!(status, Ok(status) if status.success()) {
                tracing::warn!(
                    identifier,
                    image = reference,
                    "Failed to remove stale preview image"
                );
            }
        }
    }

    async fn check_health(&self, frontend_url: &str) -> bool {
        self.readiness_client
            .get(format!("{frontend_url}/api/healthz"))
            .send()
            .await
            .map(|response| response.status().is_success())
            .unwrap_or(false)
    }

    async fn check_revision(&self, frontend_url: &str, expected_revision: &str) -> bool {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct BuildInfo {
            build_id: String,
        }

        match self
            .readiness_client
            .get(format!("{frontend_url}/build-info.json"))
            .query(&[("revision", expected_revision)])
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => response
                .json::<BuildInfo>()
                .await
                .map(|info| info.build_id.eq_ignore_ascii_case(expected_revision))
                .unwrap_or(false),
            _ => false,
        }
    }

    async fn is_current(&self, job: &PreviewJob) -> bool {
        self.queue
            .lock()
            .await
            .generations
            .get(&job.identifier)
            .copied()
            == Some(job.generation)
    }

    fn compose_environment(&self, identifier: &str, revision: &str) -> String {
        let image_tag = revision.to_ascii_lowercase();
        let password_fragment = revision.chars().take(36).collect::<String>();
        format!(
            "API_IMAGE=preview-{identifier}-api:{image_tag}\n\
        FRONTEND_IMAGE=preview-{identifier}-frontend:{image_tag}\n\
        DATABASE_IMPORT_IMAGE=preview-{identifier}-database-import:{image_tag}\n\
        DATABASE_SEED_IMAGE=preview-{identifier}-database-seed:{image_tag}\n\
        API_PORT={}\n\
        PREVIEW_SQL_PASSWORD=Aa1!{password_fragment}\n\
        EMAIL_OVERRIDE_RECIPIENT=apport@spinit.se\n\
        SMS_CUSTOMER_ID_XML=LERUMS_DJSH\n\
        SMS_USER_ID_BASIC_AUTH=lerumsdjsh\n\
VARA_IMPORT_JOB_CRON_EXPRESSION=\"0 4 * * *\"\n\
        {PROJECT_ENVIRONMENT}\n",
            self.config.backend_port
        )
    }

    fn frontend_domain(&self, identifier: &str) -> String {
        format!("{}.{}", identifier, self.config.base_domain)
    }

    fn dashboard_domain(&self, identifier: &str) -> String {
        format!("dashboard-{}.{}", identifier, self.config.base_domain)
    }

    fn frontend_url(&self, identifier: &str) -> String {
        format!("https://{}", self.frontend_domain(identifier))
    }
}

struct SourcePaths {
    inside: PathBuf,
    host: PathBuf,
}

fn validate_identifier(identifier: &str) -> Result<()> {
    ensure!(
        !identifier.is_empty() && identifier.len() <= 100,
        "preview identifier must contain 1 to 100 characters"
    );
    ensure!(
        identifier
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-'),
        "preview identifier may only contain ASCII letters, digits and hyphens"
    );
    Ok(())
}

fn validate_revision(revision: &str) -> Result<()> {
    ensure!(
        revision.len() == 40
            && revision
                .chars()
                .all(|character| character.is_ascii_hexdigit()),
        "Azure revision must be a 40-character hexadecimal commit SHA"
    );
    Ok(())
}

fn previews_to_remove_before_creation(existing_count: usize) -> usize {
    existing_count
        .saturating_add(1)
        .saturating_sub(PREVIEW_LIMIT)
}

fn is_preview_network_name(identifier: &str, network: &str) -> bool {
    network
        .strip_prefix(&format!("preview-{identifier}-"))
        .is_some_and(|suffix| {
            !suffix.is_empty()
                && suffix
                    .chars()
                    .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
        })
}

async fn docker_network_members(network: &str) -> Result<Vec<String>> {
    let output = Command::new("docker")
        .args([
            "network",
            "inspect",
            "--format",
            "{{range .Containers}}{{println .Name}}{{end}}",
            network,
        ])
        .output()
        .await
        .with_context(|| format!("failed to inspect Docker network {network}"))?;
    ensure!(
        output.status.success(),
        "Docker could not inspect network {network}"
    );
    let members =
        String::from_utf8(output.stdout).context("Docker returned non-UTF-8 network members")?;
    Ok(parse_docker_network_members(&members))
}

fn parse_docker_network_members(members: &str) -> Vec<String> {
    members
        .lines()
        .map(str::trim)
        .filter(|member| !member.is_empty())
        .map(str::to_string)
        .collect()
}

/// Dokploy only appends its Traefik labels when a Compose service's labels use
/// list syntax. Aspire emits the equivalent mapping syntax, so normalise that
/// representation at the integration boundary before uploading the document.
fn normalize_compose_labels_for_dokploy(compose: &str) -> Result<String> {
    let mut document: Value =
        serde_yaml::from_str(compose).context("generated Compose file is invalid YAML")?;
    let services = document
        .get_mut("services")
        .and_then(Value::as_mapping_mut)
        .context("generated Compose file has no services mapping")?;

    for service in services.values_mut() {
        let Some(service) = service.as_mapping_mut() else {
            continue;
        };
        let Some(labels) = service.get_mut("labels") else {
            continue;
        };
        let Value::Mapping(mapping) = labels else {
            continue;
        };

        *labels = Value::Sequence(compose_label_list(mapping)?);
    }

    serde_yaml::to_string(&document).context("failed to serialise generated Compose file")
}

fn compose_label_list(labels: &Mapping) -> Result<Vec<Value>> {
    labels
        .iter()
        .map(|(key, value)| {
            let key = key.as_str().context("Compose label key must be a string")?;
            let value = match value {
                Value::String(value) => value.clone(),
                Value::Bool(value) => value.to_string(),
                Value::Number(value) => value.to_string(),
                Value::Null => String::new(),
                _ => bail!("Compose label value for {key} must be a scalar"),
            };
            Ok(Value::String(format!("{key}={value}")))
        })
        .collect()
}

fn add_traefik_network_labels_for_dokploy(
    compose: &str,
    network: &str,
    service_names: &[&str],
) -> Result<String> {
    ensure!(!network.is_empty(), "Dokploy app network must not be empty");
    let mut document: Value =
        serde_yaml::from_str(compose).context("generated Compose file is invalid YAML")?;
    let services = document
        .get_mut("services")
        .and_then(Value::as_mapping_mut)
        .context("generated Compose file has no services mapping")?;

    for service_name in service_names {
        let service = services
            .get_mut(*service_name)
            .and_then(Value::as_mapping_mut)
            .with_context(|| format!("generated Compose file has no {service_name} service"))?;
        let labels = service
            .entry(Value::String("labels".to_string()))
            .or_insert_with(|| Value::Sequence(Vec::new()))
            .as_sequence_mut()
            .with_context(|| format!("Compose labels for {service_name} must use list syntax"))?;
        labels.retain(|label| {
            label
                .as_str()
                .is_none_or(|label| !label.starts_with("traefik.docker.network="))
        });
        labels.push(Value::String(format!("traefik.docker.network={network}")));
    }

    serde_yaml::to_string(&document).context("failed to serialise generated Compose file")
}

fn extract_zip_archive(archive_bytes: &[u8], destination: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(Cursor::new(archive_bytes))
        .context("Azure repository response is not a valid zip archive")?;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let relative_path = entry
            .enclosed_name()
            .with_context(|| format!("archive contains unsafe path {}", entry.name()))?;
        let output_path = destination.join(relative_path);

        if entry.is_dir() {
            std::fs::create_dir_all(&output_path)?;
            continue;
        }

        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut output = std::fs::File::create(&output_path)?;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = entry.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read])?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_safe_preview_identifiers() {
        assert!(validate_identifier("pr-42").is_ok());
        assert!(validate_identifier("br-feature-name").is_ok());
        assert!(validate_identifier("../escape").is_err());
        assert!(validate_identifier("with_underscore").is_err());
    }

    #[test]
    fn validates_full_commit_shas() {
        assert!(validate_revision("0123456789abcdef0123456789abcdef01234567").is_ok());
        assert!(validate_revision("01234567").is_err());
        assert!(validate_revision("z123456789abcdef0123456789abcdef01234567").is_err());
    }

    #[test]
    fn reserves_capacity_before_creating_a_fourth_preview() {
        assert_eq!(previews_to_remove_before_creation(0), 0);
        assert_eq!(previews_to_remove_before_creation(2), 0);
        assert_eq!(previews_to_remove_before_creation(3), 1);
        assert_eq!(previews_to_remove_before_creation(4), 2);
    }

    #[test]
    fn matches_only_isolated_networks_for_the_exact_preview() {
        assert!(is_preview_network_name("pr-42", "preview-pr-42-hh9avq"));
        assert!(!is_preview_network_name("pr-42", "preview-pr-42-"));
        assert!(!is_preview_network_name("pr-42", "preview-pr-420-hh9avq"));
        assert!(!is_preview_network_name("pr-42", "preview-pr-42-unsafe_"));
        assert!(!is_preview_network_name("pr-42", "preview-pr-42-HH9AVQ"));
    }

    #[test]
    fn ignores_blank_docker_network_members() {
        assert!(parse_docker_network_members("\n").is_empty());
        assert_eq!(
            parse_docker_network_members("dokploy-traefik\n\n"),
            vec!["dokploy-traefik"]
        );
    }

    #[test]
    fn normalizes_aspire_label_mappings_for_dokploy() {
        let compose = r#"
services:
  frontend:
    image: preview-frontend:abc
    labels:
      spinploy.managed: "true"
      spinploy.revision: abc
  sql:
    image: sql
    labels:
      spinploy.managed: true
"#;

        let normalized = normalize_compose_labels_for_dokploy(compose).unwrap();
        let document: Value = serde_yaml::from_str(&normalized).unwrap();
        let services = document["services"].as_mapping().unwrap();

        assert_eq!(
            services["frontend"]["labels"],
            Value::Sequence(vec![
                Value::String("spinploy.managed=true".to_string()),
                Value::String("spinploy.revision=abc".to_string()),
            ])
        );
        assert_eq!(
            services["sql"]["labels"],
            Value::Sequence(vec![Value::String("spinploy.managed=true".to_string())])
        );
    }

    #[test]
    fn preserves_existing_label_lists() {
        let compose = r#"
services:
  frontend:
    labels:
      - spinploy.managed=true
"#;

        let normalized = normalize_compose_labels_for_dokploy(compose).unwrap();
        let document: Value = serde_yaml::from_str(&normalized).unwrap();

        assert_eq!(
            document["services"]["frontend"]["labels"],
            Value::Sequence(vec![Value::String("spinploy.managed=true".to_string())])
        );
    }

    #[test]
    fn selects_the_reachable_dokploy_network_for_public_services() {
        let compose = r#"
services:
  frontend:
    networks:
      - aspire
      - dokploy
    labels:
      spinploy.managed: "true"
  preview-dashboard:
    networks:
      - aspire
      - dokploy
    labels:
      spinploy.managed: "true"
  sql:
    networks:
      - aspire
"#;
        let normalized = normalize_compose_labels_for_dokploy(compose).unwrap();

        let routed = add_traefik_network_labels_for_dokploy(
            &normalized,
            "preview-pr-42-abc123",
            &["frontend", "preview-dashboard"],
        )
        .unwrap();

        let document: Value = serde_yaml::from_str(&routed).unwrap();
        for service in ["frontend", "preview-dashboard"] {
            assert!(
                document["services"][service]["labels"]
                    .as_sequence()
                    .unwrap()
                    .contains(&Value::String(
                        "traefik.docker.network=preview-pr-42-abc123".to_string()
                    ))
            );
        }
        assert!(document["services"]["sql"].get("labels").is_none());
    }
}
