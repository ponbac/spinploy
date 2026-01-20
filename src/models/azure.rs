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

// Azure DevOps build.completed webhook payload
#[derive(Debug, Deserialize)]
pub struct AzureBuildCompletedEvent {
    #[serde(rename = "eventType")]
    pub event_type: String,
    pub resource: AzureBuildResource,
}

#[derive(Debug, Deserialize)]
pub struct AzureBuildResource {
    pub id: u64,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub result: Option<String>,
}

// Azure DevOps REST: build detail
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AzureBuildDetail {
    pub id: u64,
    pub source_version: String,
    #[serde(default)]
    pub source_branch: Option<String>,
    #[serde(default)]
    pub definition: Option<AzureBuildDefinition>,
    #[serde(default)]
    pub build_number: Option<String>,
    #[serde(default)]
    pub repository: Option<AzureBuildRepository>,
    #[serde(default, rename = "_links")]
    pub links: Option<AzureBuildLinks>,
    #[serde(default)]
    pub result: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AzureBuildRepository {
    pub id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AzureBuildDefinition {
    pub id: u64,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AzureBuildLinks {
    #[serde(default)]
    pub web: Option<AzureHref>,
}

// Azure DevOps REST: build timeline
#[derive(Debug, Deserialize)]
pub struct AzureBuildTimeline {
    #[serde(default)]
    pub records: Vec<AzureTimelineRecord>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AzureTimelineRecord {
    pub name: String,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default, rename = "type")]
    pub record_type: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
}

// Azure DevOps REST: commit detail
#[derive(Debug, Deserialize)]
pub struct AzureCommit {
    pub author: AzureCommitAuthor,
}

#[derive(Debug, Deserialize)]
pub struct AzureCommitAuthor {
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
}

// Azure DevOps REST: build list (minimal)
#[derive(Debug, Deserialize)]
pub struct AzureBuildListResponse {
    #[serde(default)]
    pub value: Vec<AzureBuildListItem>,
}

#[derive(Debug, Deserialize)]
pub struct AzureBuildListItem {
    pub id: u64,
}

// Azure DevOps REST: pull request detail
#[derive(Debug, Deserialize)]
pub struct AzurePullRequestDetail {
    pub title: String,
}
