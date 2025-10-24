use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct AzurePrCommentEvent {
    #[serde(rename = "eventType")]
    pub event_type: String,
    pub resource: AzureResource,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AzureResource {
    pub comment: AzureComment,
    pub pull_request: AzurePullRequest,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AzureComment {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub is_deleted: bool,
    #[serde(rename = "_links")]
    pub links: AzureCommentLinks,
}

#[derive(Debug, Deserialize)]
pub struct AzureCommentLinks {
    #[serde(rename = "self")]
    pub self_: Option<AzureHref>,
    #[serde(rename = "repository")]
    pub repository: Option<AzureHref>,
    pub threads: AzureHref,
}

#[derive(Debug, Deserialize)]
pub struct AzureHref {
    pub href: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AzurePullRequest {
    pub pull_request_id: u64,
    pub source_ref_name: String,
}

// Azure DevOps git.pullrequest.updated (PushNotification filtered) minimal payload
#[derive(Debug, Deserialize)]
pub struct AzurePrUpdatedEvent {
    #[serde(rename = "eventType")]
    pub event_type: String,
    pub resource: AzurePrUpdatedResource,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AzurePrUpdatedResource {
    pub pull_request_id: u64,
    pub source_ref_name: String,
    #[serde(default)]
    pub target_ref_name: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}
