use serde::{Deserialize, Serialize};

/// A project as listed by `project.all`.
///
/// Since Dokploy v0.31 the listing only carries ids, names and statuses for
/// nested services, so it is used to locate composes; full records come from
/// `compose.one`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub project_id: String,
    pub name: String,
    #[serde(default)]
    pub environments: Vec<EnvironmentSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentSummary {
    pub environment_id: String,
    pub name: String,
    #[serde(default)]
    pub compose: Vec<ComposeSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeSummary {
    pub compose_id: String,
    pub name: String,
}

/// A full compose record, as returned by `compose.one` and `compose.create`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Compose {
    pub compose_id: String,
    pub name: String,
    pub app_name: String,
    pub environment_id: String,
    #[serde(default)]
    pub domains: Vec<Domain>,
    #[serde(default)]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Domain {
    pub domain_id: String,
    pub host: String,
    pub service_name: String,
    pub compose_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateComposeRequest {
    pub name: String,
    pub environment_id: String,
    pub compose_type: String,
    pub app_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteComposeRequest {
    pub compose_id: String,
    pub delete_volumes: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateComposeRequest {
    pub compose_id: String,
    pub name: String,
    pub app_name: String,
    pub env: String,
    pub source_type: String,
    pub compose_type: String,
    pub custom_git_url: String,
    pub custom_git_branch: String,
    #[serde(rename = "customGitSSHKeyId")]
    pub custom_git_ssh_key_id: String,
    pub compose_path: String,
    pub environment_id: String,
    pub auto_deploy: bool,
    pub isolated_deployment: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRawComposeRequest {
    pub compose_id: String,
    pub name: String,
    pub app_name: String,
    pub env: String,
    pub source_type: String,
    pub compose_type: String,
    pub compose_file: String,
    pub environment_id: String,
    pub auto_deploy: bool,
    pub isolated_deployment: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainCreateRequest {
    pub host: String,
    pub path: String,
    pub port: u16,
    pub https: bool,
    pub certificate_type: String,
    pub compose_id: String,
    pub service_name: String,
    pub domain_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeDeployRequest {
    pub compose_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Deployment {
    pub deployment_id: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub log_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeDetail {
    pub compose_id: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub deployments: Vec<Deployment>,
}
