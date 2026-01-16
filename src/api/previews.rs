use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::stream::Stream;
use serde::Deserialize;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

use crate::AppState;

use super::types::*;

/// Query parameters for log streaming
#[derive(Deserialize)]
pub struct LogParams {
    #[serde(default = "default_tail")]
    pub tail: usize,
    #[serde(default = "default_follow")]
    pub follow: bool,
}

fn default_tail() -> usize {
    100
}

fn default_follow() -> bool {
    true
}

/// Parse preview identifier to extract PR ID if present
/// Returns (pr_id, identifier)
fn parse_preview_identifier(identifier: &str) -> (Option<String>, String) {
    if let Some(pr_num) = identifier.strip_prefix("pr-") {
        return (Some(pr_num.to_string()), identifier.to_string());
    }
    (None, identifier.to_string())
}

/// Get container name for a preview service
fn get_container_name(app_name: &str, service: &str) -> String {
    // Dokploy uses isolated deployment with pattern: {app_name}-{service}-1
    format!("{}-{}-1", app_name, service)
}

/// Build PR URL from config
fn build_pr_url(state: &AppState, pr_id: &str) -> String {
    format!(
        "https://dev.azure.com/{}/{}/_git/{}/pullrequest/{}",
        state.config.azdo_org, state.config.azdo_project, state.config.azdo_repository_id, pr_id
    )
}

/// Determine preview status based on deployment and container state
async fn determine_preview_status(
    state: &AppState,
    compose_detail: &spinploy::models::dokploy::ComposeDetail,
    app_name: &str,
) -> PreviewStatus {
    // Check latest deployment status first
    if let Some(latest_deployment) = compose_detail.deployments.first() {
        // Check deployment status from Dokploy
        if let Some(status) = &latest_deployment.status {
            match status.as_str() {
                "error" => return PreviewStatus::Failed,
                "running" => return PreviewStatus::Building,
                "done" => return PreviewStatus::Running,
                _ => {} // Unknown status, fall through to container check
            }
        }

        // Fallback: check timestamps if no status field
        if latest_deployment.finished_at.is_none() && latest_deployment.started_at.is_some() {
            return PreviewStatus::Building;
        }
    }

    // Check Docker containers if client available
    if let Some(docker_client) = &state.docker_client {
        match docker_client.list_containers(Some(app_name)).await {
            Ok(containers) => {
                if containers.is_empty() {
                    return PreviewStatus::Unknown;
                }

                let all_running = containers.iter().all(|c| c.state == "running");
                if all_running {
                    PreviewStatus::Running
                } else {
                    PreviewStatus::Failed
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, app_name, "Failed to list containers");
                PreviewStatus::Unknown
            }
        }
    } else {
        // No Docker client, try to infer from deployments
        if !compose_detail.deployments.is_empty() {
            PreviewStatus::Running
        } else {
            PreviewStatus::Unknown
        }
    }
}

/// Calculate duration in seconds between two timestamps
fn calculate_duration(started_at: &Option<String>, finished_at: &Option<String>) -> Option<u64> {
    let started = started_at.as_ref().and_then(|s| crate::parse_ts(s))?;
    let finished = finished_at.as_ref().and_then(|s| crate::parse_ts(s))?;

    let duration = finished.signed_duration_since(started);
    Some(duration.num_seconds().max(0) as u64)
}

/// GET /api/previews - List all active preview deployments
pub async fn list_previews(
    crate::ApiKey(api_key): crate::ApiKey,
    State(state): State<AppState>,
) -> Result<Json<PreviewListResponse>, (StatusCode, String)> {
    let composes = state
        .dokploy_client
        .list_composes_with_prefix(&api_key, &state.config.environment_id, "preview-")
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to list composes");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to list previews".to_string(),
            )
        })?;

    let mut previews = Vec::new();

    for compose in composes {
        let identifier = compose.name.clone();
        let (pr_id, _) = parse_preview_identifier(&identifier);

        // Get compose detail for deployment history
        let compose_detail = state
            .dokploy_client
            .get_compose_detail(&api_key, &compose.compose_id)
            .await
            .map_err(|e| {
                tracing::warn!(
                    error = %e,
                    compose_id = &compose.compose_id,
                    "Failed to get compose detail"
                );
                e
            })
            .ok();

        let status = if let Some(ref detail) = compose_detail {
            determine_preview_status(&state, detail, &compose.app_name).await
        } else {
            PreviewStatus::Unknown
        };

        let last_deployed_at = compose_detail
            .as_ref()
            .and_then(|d| d.deployments.first())
            .and_then(|dep| {
                dep.finished_at
                    .clone()
                    .or_else(|| dep.started_at.clone())
                    .or_else(|| dep.created_at.clone())
            });

        // Get domains
        let domains = state
            .dokploy_client
            .list_domains_by_compose_id(&api_key, &compose.compose_id)
            .await
            .unwrap_or_default();

        let frontend_url = domains
            .iter()
            .find(|d| d.service_name == state.config.frontend_service_name)
            .map(|d| format!("https://{}", d.host));

        let backend_url = domains
            .iter()
            .find(|d| d.service_name == state.config.backend_service_name)
            .map(|d| format!("https://{}", d.host));

        let pr_url = pr_id.as_ref().map(|id| build_pr_url(&state, id));

        // Get container info
        let containers = if let Some(docker_client) = &state.docker_client {
            docker_client
                .list_containers(Some(&compose.app_name))
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|c| {
                    let service = c
                        .names
                        .first()
                        .and_then(|name| {
                            // Extract service name from container name pattern: preview-{id}-{service}-1
                            let parts: Vec<&str> =
                                name.trim_start_matches('/').split('-').collect();
                            if parts.len() >= 4 {
                                Some(parts[parts.len() - 2].to_string())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| "unknown".to_string());

                    ContainerSummary {
                        name: c
                            .names
                            .first()
                            .unwrap_or(&c.id)
                            .trim_start_matches('/')
                            .to_string(),
                        service,
                        state: c.state.clone(),
                    }
                })
                .collect()
        } else {
            vec![]
        };

        // Extract branch from app_name (format: "preview-{identifier}")
        let branch = identifier.clone();

        previews.push(PreviewSummary {
            identifier,
            compose_id: compose.compose_id,
            pr_id,
            branch,
            status,
            created_at: compose.created_at,
            last_deployed_at,
            frontend_url,
            backend_url,
            pr_url,
            containers,
        });
    }

    // Sort by most recent deployment (newest first)
    previews.sort_by(|a, b| {
        let a_time = a.last_deployed_at.as_ref().or(a.created_at.as_ref());
        let b_time = b.last_deployed_at.as_ref().or(b.created_at.as_ref());
        b_time.cmp(&a_time)
    });

    Ok(Json(PreviewListResponse { previews }))
}

/// GET /api/previews/{identifier} - Get detailed info for a specific preview
pub async fn get_preview_detail(
    crate::ApiKey(api_key): crate::ApiKey,
    State(state): State<AppState>,
    Path(identifier): Path<String>,
) -> Result<Json<PreviewDetailResponse>, (StatusCode, String)> {
    let compose = state
        .dokploy_client
        .find_compose_by_name(&api_key, &identifier)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, identifier, "Failed to find compose");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to find preview".to_string(),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("Preview '{}' not found", identifier),
            )
        })?;

    let (pr_id, _) = parse_preview_identifier(&identifier);

    // Get compose detail for deployment history
    let compose_detail = state
        .dokploy_client
        .get_compose_detail(&api_key, &compose.compose_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, compose_id = &compose.compose_id, "Failed to get compose detail");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to get preview details".to_string(),
            )
        })?;

    let status = determine_preview_status(&state, &compose_detail, &compose.app_name).await;

    let last_deployed_at = compose_detail.deployments.first().and_then(|dep| {
        dep.finished_at
            .clone()
            .or_else(|| dep.started_at.clone())
            .or_else(|| dep.created_at.clone())
    });

    // Get domains
    let domains = state
        .dokploy_client
        .list_domains_by_compose_id(&api_key, &compose.compose_id)
        .await
        .unwrap_or_default();

    let frontend_url = domains
        .iter()
        .find(|d| d.service_name == state.config.frontend_service_name)
        .map(|d| format!("https://{}", d.host));

    let backend_url = domains
        .iter()
        .find(|d| d.service_name == state.config.backend_service_name)
        .map(|d| format!("https://{}", d.host));

    let pr_url = pr_id.as_ref().map(|id| build_pr_url(&state, id));

    // Get container info
    let containers = if let Some(docker_client) = &state.docker_client {
        docker_client
            .list_containers(Some(&compose.app_name))
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|c| {
                let service = c
                    .names
                    .first()
                    .and_then(|name| {
                        let parts: Vec<&str> = name.trim_start_matches('/').split('-').collect();
                        if parts.len() >= 4 {
                            Some(parts[parts.len() - 2].to_string())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| "unknown".to_string());

                ContainerSummary {
                    name: c
                        .names
                        .first()
                        .unwrap_or(&c.id)
                        .trim_start_matches('/')
                        .to_string(),
                    service,
                    state: c.state.clone(),
                }
            })
            .collect()
    } else {
        vec![]
    };

    // Extract branch from identifier
    let branch = identifier.clone();

    // Convert deployments to DeploymentInfo with duration
    let deployments = compose_detail
        .deployments
        .iter()
        .map(|d| DeploymentInfo {
            deployment_id: d.deployment_id.clone(),
            status: d.status.clone(),
            created_at: d.created_at.clone(),
            started_at: d.started_at.clone(),
            finished_at: d.finished_at.clone(),
            duration_seconds: calculate_duration(&d.started_at, &d.finished_at),
        })
        .collect();

    let summary = PreviewSummary {
        identifier,
        compose_id: compose.compose_id,
        pr_id,
        branch,
        status,
        created_at: compose.created_at,
        last_deployed_at,
        frontend_url,
        backend_url,
        pr_url,
        containers,
    };

    Ok(Json(PreviewDetailResponse {
        summary,
        deployments,
    }))
}

/// GET /api/previews/{identifier}/containers/{service}/logs - Stream container logs via SSE
pub async fn stream_preview_container_logs(
    crate::ApiKey(api_key): crate::ApiKey,
    State(state): State<AppState>,
    Path((identifier, service)): Path<(String, String)>,
    Query(params): Query<LogParams>,
) -> Result<Sse<impl Stream<Item = Result<Event, String>>>, (StatusCode, String)> {
    let docker_client = state.docker_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Docker client not available".to_string(),
        )
    })?;

    // Fetch compose to get the actual app_name (includes random suffix from Dokploy)
    let compose = state
        .dokploy_client
        .find_compose_by_name(&api_key, &identifier)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, identifier, "Failed to find compose for logs");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to find preview: {}", e),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("Preview '{}' not found", identifier),
            )
        })?;

    // Get container name using actual app_name from Dokploy
    let container_name = get_container_name(&compose.app_name, &service);

    tracing::info!(
        identifier,
        service,
        container_name,
        tail = params.tail,
        follow = params.follow,
        "Streaming container logs"
    );

    // Stream logs via Docker client
    let receiver = docker_client
        .stream_logs(&container_name, params.tail as u64, params.follow)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, container_name, "Failed to stream logs");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to stream logs: {}", e),
            )
        })?;

    let stream = ReceiverStream::new(receiver).map(|line_result| {
        line_result
            .map(|line| Event::default().data(line))
            .map_err(|err| err.to_string())
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
