pub mod api {
    progenitor::generate_api!("openapi.json");
}

use std::time::Duration;

use anyhow::Context;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

pub fn build_client_from_env() -> anyhow::Result<api::Client> {
    // Load variables from .env.local if present. Existing env vars take precedence.
    let _ = dotenvy::from_filename(".env.local");

    let baseurl = std::env::var("DOKPLOY_URL").context("$DOKPLOY_URL not set")?;
    let api_key = std::env::var("DOKPLOY_API_KEY").context("$DOKPLOY_API_KEY not set")?;

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-api-key"),
        HeaderValue::from_str(&api_key).context("invalid API key value")?,
    );

    let http_client = reqwest::ClientBuilder::new()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(15))
        .default_headers(headers)
        .build()?;

    let client = api::Client::new_with_client(baseurl.as_str(), http_client);
    Ok(client)
}
