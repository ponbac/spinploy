use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewListResponse {
    pub previews: Vec<PreviewSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSummary {
    pub identifier: String,
    pub compose_id: String,
    pub pr_id: Option<String>,
    pub branch: String,
    pub status: PreviewStatus,
    pub created_at: Option<String>,
    pub last_deployed_at: Option<String>,
    pub frontend_url: Option<String>,
    pub backend_url: Option<String>,
    pub pr_url: Option<String>,
    pub containers: Vec<ContainerSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewDetailResponse {
    #[serde(flatten)]
    pub summary: PreviewSummary,
    pub deployments: Vec<DeploymentInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PreviewStatus {
    Building,
    Running,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerSummary {
    pub name: String,
    pub service: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentInfo {
    pub deployment_id: String,
    pub created_at: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub duration_seconds: Option<u64>,
}
