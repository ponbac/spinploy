use std::time::Duration;

use crate::api::{self, types};
use anyhow::{Context, Result, anyhow, bail};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::{info, instrument};

/// Lightweight wrapper around the Dokploy API.
///
/// Notes:
/// - API key is passed per-call and injected as `x-api-key` header.
/// - We call documented endpoints used by the legacy bash script.
/// - We intentionally keep request/response models minimal and dynamic (serde_json::Value)
///   to tolerate server-side changes without frequent client regenerations.
#[derive(Clone, Debug)]
pub struct DokployClient {
    base_url: String,
}

impl DokployClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
        }
    }

    pub fn from_env_url() -> Result<Self> {
        let base_url = std::env::var("DOKPLOY_URL").context("$DOKPLOY_URL not set")?;
        Ok(Self::new(base_url))
    }

    fn client_with_key(&self, api_key: &str) -> Result<api::Client> {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-api-key"),
            HeaderValue::from_str(api_key).context("invalid api key header")?,
        );

        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30))
            .default_headers(headers)
            .build()?;

        Ok(api::Client::new_with_client(&self.base_url, http))
    }

    pub async fn find_compose_id(&self, api_key: &str, compose_name: &str) -> Result<String> {
        let client = self.client_with_key(api_key)?;

        let projects = client.project_all().await?;

        todo!()
    }

    /// Delete preview deployment (if it exists). Always deletes volumes.
    #[instrument(skip(self, api_key))]
    pub async fn delete_preview(
        &self,
        api_key: &str,
        project_id: &str,
        environment_id: &str,
        compose_name: &str,
    ) -> Result<()> {
        let client = self.client_with_key(api_key)?;

        todo!()
    }
}
