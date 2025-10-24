use std::time::Duration;

use anyhow::Result;

/// Minimal Azure DevOps REST client for posting PR thread comments
#[derive(Clone, Debug)]
pub struct AzureDevOpsClient {
    pub org: String,
    pub project: String,
    pat: String,
    client: reqwest::Client,
}

impl AzureDevOpsClient {
    pub fn new(org: impl AsRef<str>, project: impl AsRef<str>, pat: impl AsRef<str>) -> Self {
        let reqw_client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build http client");
        Self {
            org: org.as_ref().to_string(),
            project: project.as_ref().to_string(),
            pat: pat.as_ref().to_string(),
            client: reqw_client,
        }
    }

    /// Post a text reply inside an existing PR comment thread
    pub async fn reply_in_thread(
        &self,
        repo_id: &str,
        pr_id: u64,
        thread_id: u64,
        content: &str,
    ) -> Result<()> {
        let url = format!(
            "https://dev.azure.com/{}/{}/_apis/git/repositories/{}/pullRequests/{}/threads/{}/comments?api-version=7.1-preview.1",
            self.org, self.project, repo_id, pr_id, thread_id
        );

        let body = serde_json::json!({
            "content": content,
            "commentType": "text",
        });

        self.client
            .post(url)
            // PAT as Basic password; username can be empty
            .basic_auth("", Some(&self.pat))
            .json(&body)
            .send()
            .await?
            .error_for_status()?;

        Ok(())
    }
}
