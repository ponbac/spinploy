use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub project_id: String,
    pub name: String,
    pub organization_id: String,
    #[serde(default)]
    pub environments: Vec<Environment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Environment {
    pub environment_id: String,
    pub name: String,
    pub project_id: String,
    #[serde(default)]
    pub compose: Vec<Compose>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Compose {
    pub compose_id: String,
    pub name: String,
    pub app_name: String,
    pub environment_id: String,
    #[serde(default)]
    pub domains: Vec<Domain>,
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
    pub custom_git_ssh_key_id: String,
    pub compose_path: String,
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
