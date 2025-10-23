use std::time::Duration;

use crate::models::dokploy::{
    Compose, ComposeDeployRequest, CreateComposeRequest, DeleteComposeRequest, Domain,
    DomainCreateRequest, Project, UpdateComposeRequest,
};
use anyhow::{Context, Result, bail};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Serialize, de::DeserializeOwned};
// keep client lean; avoid verbose tracing here

/// Lightweight wrapper around the Dokploy API using manual reqwest calls.
#[derive(Clone, Debug)]
pub struct DokployClient {
    base_url: String,
    http: reqwest::Client,
}

impl DokployClient {
    pub fn new(base_url: impl AsRef<str>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build http client");
        Self {
            base_url: base_url.as_ref().trim_end_matches('/').to_string(),
            http,
        }
    }

    fn auth_headers(api_key: &str) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-api-key"),
            HeaderValue::from_str(api_key).context("invalid api key header")?,
        );
        Ok(headers)
    }

    fn join_url(&self, url: &str) -> String {
        format!("{}/{}", self.base_url, url.trim_start_matches('/'))
    }

    async fn get<T: DeserializeOwned>(&self, api_key: &str, url: &str) -> Result<T> {
        let resp = self
            .http
            .get(self.join_url(url))
            .headers(Self::auth_headers(api_key)?)
            .send()
            .await?
            .error_for_status()?;

        resp.json::<T>()
            .await
            .context("failed to deserialize response")
    }

    async fn post<T: DeserializeOwned>(
        &self,
        api_key: &str,
        url: &str,
        body: impl Serialize,
    ) -> Result<T> {
        let resp = self
            .http
            .post(self.join_url(url))
            .headers(Self::auth_headers(api_key)?)
            .json(&body)
            .send()
            .await?
            .error_for_status()?;

        resp.json::<T>()
            .await
            .context("failed to deserialize response")
    }

    /// POST helper for endpoints where the response body is irrelevant.
    async fn post_unit(&self, api_key: &str, url: &str, body: impl Serialize) -> Result<()> {
        self.http
            .post(self.join_url(url))
            .headers(Self::auth_headers(api_key)?)
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// Retrieve all projects with nested environments and compose definitions.
    pub async fn fetch_projects(&self, api_key: impl AsRef<str>) -> Result<Vec<Project>> {
        self.get::<Vec<Project>>(api_key.as_ref(), "project.all")
            .await
    }

    pub async fn find_compose_by_name(
        &self,
        api_key: impl AsRef<str> + std::fmt::Debug,
        compose_name: impl AsRef<str> + std::fmt::Debug,
    ) -> Result<Option<Compose>> {
        let projects = self.fetch_projects(api_key).await?;

        let matching_composes: Vec<_> = projects
            .into_iter()
            .flat_map(|project| project.environments.into_iter())
            .flat_map(|env| env.compose.into_iter())
            .filter(|compose| compose.name == compose_name.as_ref())
            .collect();

        match matching_composes.len() {
            0 => Ok(None),
            1 => Ok(Some(
                matching_composes
                    .into_iter()
                    .next()
                    .expect("single compose found"),
            )),
            _ => {
                let ids: Vec<_> = matching_composes.iter().map(|c| &*c.compose_id).collect();
                bail!(
                    "multiple composes named {:?} found with IDs {:?}",
                    compose_name.as_ref(),
                    ids
                )
            }
        }
    }

    /// Delete preview deployment (if it exists). Always deletes volumes.
    pub async fn delete_compose(
        &self,
        api_key: &str,
        compose_id: impl AsRef<str> + std::fmt::Debug,
        delete_volumes: bool,
    ) -> Result<()> {
        self.post_unit(
            api_key,
            "compose.delete",
            DeleteComposeRequest {
                compose_id: compose_id.as_ref().to_string(),
                delete_volumes,
            },
        )
        .await
    }

    pub async fn create_compose(
        &self,
        api_key: &str,
        environment_id: impl AsRef<str> + std::fmt::Debug,
        name: impl AsRef<str> + std::fmt::Debug,
        app_name: impl AsRef<str> + std::fmt::Debug,
    ) -> Result<Compose> {
        self.post::<Compose>(
            api_key,
            "compose.create",
            CreateComposeRequest {
                environment_id: environment_id.as_ref().to_string(),
                name: name.as_ref().to_string(),
                app_name: app_name.as_ref().to_string(),
                compose_type: "docker-compose".to_string(),
            },
        )
        .await
    }

    /// Update a compose definition.
    pub async fn update_compose(&self, api_key: &str, req: UpdateComposeRequest) -> Result<()> {
        self.post_unit(api_key, "compose.update", req).await
    }

    /// Trigger deployment of a compose.
    pub async fn deploy_compose(&self, api_key: &str, compose_id: impl AsRef<str>) -> Result<()> {
        let body = ComposeDeployRequest {
            compose_id: compose_id.as_ref().to_string(),
        };
        self.post_unit(api_key, "compose.deploy", body).await
    }

    /// List domains attached to a compose.
    pub async fn list_domains_by_compose_id(
        &self,
        api_key: &str,
        compose_id: impl AsRef<str>,
    ) -> Result<Vec<Domain>> {
        let url = format!("domain.byComposeId?composeId={}", compose_id.as_ref());
        let resp = self
            .http
            .get(self.join_url(&url))
            .headers(Self::auth_headers(api_key)?)
            .send()
            .await?
            .error_for_status()?;

        let body = resp.text().await?;
        if body.trim().is_empty() {
            return Ok(vec![]);
        }
        serde_json::from_str::<Vec<Domain>>(&body)
            .context("failed to deserialize list domains response")
    }

    /// Create a domain for a compose service.
    pub async fn create_domain(&self, api_key: &str, req: DomainCreateRequest) -> Result<()> {
        self.post_unit(api_key, "domain.create", req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client_with_api_key() -> (DokployClient, String) {
        crate::test_init_env();
        let client = DokployClient::new(std::env::var("DOKPLOY_URL").unwrap());
        let api_key = std::env::var("DOKPLOY_API_KEY").unwrap();
        (client, api_key)
    }

    #[tokio::test]
    async fn test_find_compose_id() {
        let (client, api_key) = client_with_api_key();

        let res = dbg!(client.find_compose_by_name(&api_key, "pr-1774").await);
        assert!(res.is_ok());
    }
}
