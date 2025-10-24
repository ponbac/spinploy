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
pub struct AzureComment {
    pub content: String,
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
}
