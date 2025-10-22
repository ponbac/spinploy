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
