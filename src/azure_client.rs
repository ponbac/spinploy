use std::time::Duration;

use crate::models::azure::{
    AzureBuildDetail, AzureBuildListItem, AzureBuildListResponse, AzureBuildTimeline, AzureCommit,
    AzureGitRefList, AzurePullRequestDetail,
};
use anyhow::{Context, Result, bail};

/// Minimal Azure DevOps REST client for posting PR thread comments
#[derive(Clone, Debug)]
pub struct AzureDevOpsClient {
    base_url: String,
    pub org: String,
    pub project: String,
    pat: String,
    client: reqwest::Client,
}

impl AzureDevOpsClient {
    pub fn new(org: impl AsRef<str>, project: impl AsRef<str>, pat: impl AsRef<str>) -> Self {
        Self::with_base_url("https://dev.azure.com", org, project, pat)
    }

    pub fn with_base_url(
        base_url: impl AsRef<str>,
        org: impl AsRef<str>,
        project: impl AsRef<str>,
        pat: impl AsRef<str>,
    ) -> Self {
        let reqw_client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build http client");
        Self {
            base_url: base_url.as_ref().trim_end_matches('/').to_string(),
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
            "{}/{}/{}/_apis/git/repositories/{}/pullRequests/{}/threads/{}/comments?api-version=7.1-preview.1",
            self.base_url, self.org, self.project, repo_id, pr_id, thread_id
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

    /// Fetch build details to obtain sourceVersion, repository id, build number and result.
    pub async fn get_build(&self, build_id: u64) -> Result<AzureBuildDetail> {
        let url = format!(
            "{}/{}/{}/_apis/build/builds/{}?api-version=7.1-preview.7",
            self.base_url, self.org, self.project, build_id
        );

        let resp = self
            .client
            .get(url)
            .basic_auth("", Some(&self.pat))
            .send()
            .await?
            .error_for_status()?
            .json::<AzureBuildDetail>()
            .await?;

        Ok(resp)
    }

    /// Fetch build timeline to inspect stage/job results.
    pub async fn get_build_timeline(&self, build_id: u64) -> Result<AzureBuildTimeline> {
        let url = format!(
            "{}/{}/{}/_apis/build/builds/{}/timeline?api-version=7.1-preview.2",
            self.base_url, self.org, self.project, build_id
        );

        let resp = self
            .client
            .get(url)
            .basic_auth("", Some(&self.pat))
            .send()
            .await?
            .error_for_status()?
            .json::<AzureBuildTimeline>()
            .await?;

        Ok(resp)
    }

    /// Fetch commit details to get commit author information.
    pub async fn get_commit(&self, repo_id: &str, commit_sha: &str) -> Result<AzureCommit> {
        let url = format!(
            "{}/{}/{}/_apis/git/repositories/{}/commits/{}?api-version=7.1-preview.1",
            self.base_url, self.org, self.project, repo_id, commit_sha
        );

        let resp = self
            .client
            .get(url)
            .basic_auth("", Some(&self.pat))
            .send()
            .await?
            .error_for_status()?
            .json::<AzureCommit>()
            .await?;

        Ok(resp)
    }

    /// List recent completed builds for a given definition and branch, newest first.
    pub async fn list_builds(
        &self,
        definition_id: u64,
        branch_name: &str,
        top: u64,
    ) -> Result<Vec<AzureBuildListItem>> {
        let url = format!(
            "{}/{}/{}/_apis/build/builds?api-version=7.1-preview.7",
            self.base_url, self.org, self.project
        );

        let resp = self
            .client
            .get(url)
            .basic_auth("", Some(&self.pat))
            .query(&[
                ("definitions", definition_id.to_string()),
                ("branchName", branch_name.to_string()),
                ("statusFilter", "completed".to_string()),
                ("queryOrder", "finishTimeDescending".to_string()),
                ("$top", top.to_string()),
            ])
            .send()
            .await?
            .error_for_status()?
            .json::<AzureBuildListResponse>()
            .await?;

        Ok(resp.value)
    }

    /// Fetch pull request details to get title.
    pub async fn get_pull_request(
        &self,
        repo_id: &str,
        pr_id: u64,
    ) -> Result<AzurePullRequestDetail> {
        let url = format!(
            "{}/{}/{}/_apis/git/repositories/{}/pullRequests/{}?api-version=7.1-preview.1",
            self.base_url, self.org, self.project, repo_id, pr_id
        );

        let resp = self
            .client
            .get(url)
            .basic_auth("", Some(&self.pat))
            .send()
            .await?
            .error_for_status()?
            .json::<AzurePullRequestDetail>()
            .await?;

        Ok(resp)
    }

    /// Resolve a branch name to the exact commit that should be built.
    pub async fn get_branch_head(&self, repo_id: &str, branch_name: &str) -> Result<String> {
        let branch_name = branch_name.trim_start_matches("refs/heads/");
        let url = format!(
            "{}/{}/{}/_apis/git/repositories/{}/refs",
            self.base_url, self.org, self.project, repo_id
        );

        let refs = self
            .client
            .get(url)
            .basic_auth("", Some(&self.pat))
            .query(&[
                ("filter", format!("heads/{branch_name}")),
                ("api-version", "7.1".to_string()),
            ])
            .send()
            .await?
            .error_for_status()?
            .json::<AzureGitRefList>()
            .await?;

        let expected_name = format!("refs/heads/{branch_name}");
        let matching = refs
            .value
            .into_iter()
            .filter(|git_ref| git_ref.name == expected_name)
            .collect::<Vec<_>>();
        match matching.as_slice() {
            [git_ref] => Ok(git_ref.object_id.clone()),
            [] => bail!("Azure branch {expected_name} was not found"),
            _ => bail!("Azure returned multiple refs for branch {expected_name}"),
        }
    }

    /// Download a repository snapshot for an exact commit as a zip archive.
    pub async fn download_repository_archive(
        &self,
        repo_id: &str,
        commit_sha: &str,
    ) -> Result<Vec<u8>> {
        let url = format!(
            "{}/{}/{}/_apis/git/repositories/{}/items",
            self.base_url, self.org, self.project, repo_id
        );

        let response = self
            .client
            .get(url)
            .basic_auth("", Some(&self.pat))
            .query(&[
                ("scopePath", "/"),
                ("recursionLevel", "Full"),
                ("includeContentMetadata", "false"),
                ("versionDescriptor.version", commit_sha),
                ("versionDescriptor.versionType", "commit"),
                ("$format", "zip"),
                ("download", "true"),
                ("api-version", "7.1"),
            ])
            .timeout(Duration::from_secs(10 * 60))
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await
            .context("failed to download Azure repository archive")?;

        Ok(response.to_vec())
    }
}
