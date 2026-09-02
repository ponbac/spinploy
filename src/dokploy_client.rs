use std::time::Duration;

use crate::models::dokploy::{
    Compose, ComposeDeployRequest, ComposeDetail, CreateComposeRequest, DeleteComposeRequest,
    Domain, DomainCreateRequest, Project, UpdateComposeRequest, UpdateRawComposeRequest,
};
use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use regress::Regex;
use reqwest::Response;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Serialize, de::DeserializeOwned};
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, http::Request as WsRequest},
};
// keep client lean; avoid verbose tracing here

const ERROR_RESPONSE_BODY_LIMIT: usize = 2 * 1024;

async fn read_error_body(response: Response, api_key: &str) -> String {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::with_capacity(ERROR_RESPONSE_BODY_LIMIT);
    let mut truncated = false;

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => return format!("<failed to read response body: {error}>"),
        };
        let remaining = ERROR_RESPONSE_BODY_LIMIT.saturating_sub(bytes.len());
        if chunk.len() > remaining {
            bytes.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        bytes.extend_from_slice(&chunk);
    }

    let mut body = sanitize_error_body(&bytes, api_key);
    if body.is_empty() {
        body.push_str("<empty>");
    }
    if truncated {
        body.push_str("… <truncated>");
    }
    body
}

fn sanitize_error_body(bytes: &[u8], api_key: &str) -> String {
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    if !api_key.is_empty() {
        text = text.replace(api_key, "[REDACTED]");
    }
    text = redact_sensitive_assignments(&text);

    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(mut value) => {
            redact_sensitive_json(&mut value);
            value.to_string()
        }
        Err(_) => text.split_whitespace().collect::<Vec<_>>().join(" "),
    }
}

fn redact_sensitive_assignments(text: &str) -> String {
    let Ok(pattern) = Regex::with_flags(
        r#"((?:"|')?\b(?:api[_ -]?key|authorization|credentials?|password|passwd|pat|refresh[_ -]?token|secret|token)\b(?:"|')?\s*[:=]\s*)(?:"(?:\\.|[^"])*"|'(?:\\.|[^'])*'|[^,\s&}]*)"#,
        "i",
    ) else {
        return text.to_string();
    };

    pattern.replace_all(text, r#"$1"[REDACTED]""#)
}

fn redact_sensitive_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                if is_sensitive_field(key) {
                    *value = serde_json::Value::String("[REDACTED]".to_string());
                } else {
                    redact_sensitive_json(value);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                redact_sensitive_json(value);
            }
        }
        _ => {}
    }
}

fn is_sensitive_field(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['-', ' '], "_");
    matches!(
        normalized.as_str(),
        "api_key"
            | "apikey"
            | "authorization"
            | "credential"
            | "credentials"
            | "password"
            | "passwd"
            | "pat"
            | "refresh_token"
            | "secret"
            | "token"
    ) || normalized.ends_with("_secret")
        || normalized.ends_with("_token")
}

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

    async fn require_success(
        response: Response,
        endpoint: &str,
        api_key: &str,
    ) -> Result<Response> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }

        let body = read_error_body(response, api_key).await;
        bail!("Dokploy API {endpoint} failed with HTTP status {status}; response body: {body}")
    }

    async fn get<T: DeserializeOwned>(&self, api_key: &str, url: &str) -> Result<T> {
        let resp = self
            .http
            .get(self.join_url(url))
            .headers(Self::auth_headers(api_key)?)
            .send()
            .await?;
        let resp = Self::require_success(resp, url, api_key).await?;

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
        let resp = self.post_response(api_key, url, body).await?;
        let resp = Self::require_success(resp, url, api_key).await?;

        resp.json::<T>()
            .await
            .context("failed to deserialize response")
    }

    /// POST helper for endpoints where the response body is irrelevant.
    async fn post_unit(&self, api_key: &str, url: &str, body: impl Serialize) -> Result<()> {
        let resp = self.post_response(api_key, url, body).await?;
        Self::require_success(resp, url, api_key).await?;
        Ok(())
    }

    async fn post_response(
        &self,
        api_key: &str,
        url: &str,
        body: impl Serialize,
    ) -> Result<Response> {
        Ok(self
            .http
            .post(self.join_url(url))
            .headers(Self::auth_headers(api_key)?)
            .json(&body)
            .send()
            .await?)
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
        let response = self
            .post_response(
                api_key,
                "compose.delete",
                DeleteComposeRequest {
                    compose_id: compose_id.as_ref().to_string(),
                    delete_volumes,
                },
            )
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        Self::require_success(response, "compose.delete", api_key).await?;
        Ok(())
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

    /// Update a compose definition whose source is supplied directly rather than fetched by Dokploy.
    pub async fn update_raw_compose(
        &self,
        api_key: &str,
        req: UpdateRawComposeRequest,
    ) -> Result<()> {
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
            .await?;
        let resp = Self::require_success(resp, &url, api_key).await?;

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

    /// List composes in a given environment with a given app name prefix
    pub async fn list_composes_with_prefix(
        &self,
        api_key: &str,
        environment_id: &str,
        app_name_prefix: &str,
    ) -> Result<Vec<Compose>> {
        let projects = self.fetch_projects(api_key).await?;
        let mut comps = Vec::new();
        for project in projects.into_iter() {
            for env in project.environments.into_iter() {
                if env.environment_id == environment_id {
                    comps.extend(
                        env.compose
                            .into_iter()
                            .filter(|c| c.app_name.starts_with(app_name_prefix)),
                    );
                }
            }
        }
        Ok(comps)
    }

    /// Fetch a compose detail (compose.one)
    pub async fn get_compose_detail(
        &self,
        api_key: &str,
        compose_id: &str,
    ) -> Result<ComposeDetail> {
        let url = format!("compose.one?composeId={}", compose_id);
        self.get::<ComposeDetail>(api_key, &url).await
    }

    /// Stream deployment logs via WebSocket connection to Dokploy.
    /// Returns a receiver that yields log lines.
    pub async fn stream_deployment_logs(
        &self,
        api_key: &str,
        log_path: &str,
    ) -> Result<mpsc::Receiver<Result<String, String>>> {
        // Convert HTTP URL to WebSocket URL, stripping /api suffix since WebSocket is at root
        let base_without_api = self.base_url.trim_end_matches("/api").trim_end_matches("/");
        let ws_url = base_without_api
            .replace("https://", "wss://")
            .replace("http://", "ws://");

        let encoded_log_path = urlencoding::encode(log_path);
        let full_url = format!("{}/listen-deployment?logPath={}", ws_url, encoded_log_path);

        tracing::debug!(url = %full_url, "Connecting to Dokploy WebSocket");

        // Build request with x-api-key header for authentication
        // Host header extracted from base URL without protocol or /api path
        let host = base_without_api
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        let request = WsRequest::builder()
            .uri(&full_url)
            .header("x-api-key", api_key)
            .header("Host", host)
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header(
                "Sec-WebSocket-Key",
                tokio_tungstenite::tungstenite::handshake::client::generate_key(),
            )
            .body(())
            .context("Failed to build WebSocket request")?;

        let (ws_stream, _) = connect_async(request)
            .await
            .context("Failed to connect to Dokploy WebSocket")?;

        let (tx, rx) = mpsc::channel(256);
        let (_write, mut read) = ws_stream.split();

        // Spawn task to read from WebSocket and forward to channel
        tokio::spawn(async move {
            while let Some(msg_result) = read.next().await {
                match msg_result {
                    Ok(Message::Text(text)) if tx.send(Ok(text.to_string())).await.is_err() => {
                        break;
                    }
                    Ok(Message::Text(_)) => {}
                    Ok(Message::Close(_)) => {
                        break;
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string())).await;
                        break;
                    }
                    _ => {}
                }
            }
        });

        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, http::StatusCode, routing::post};

    fn client_with_api_key() -> (DokployClient, String) {
        crate::test_init_env();
        let client = DokployClient::new(std::env::var("DOKPLOY_URL").unwrap());
        let api_key = std::env::var("DOKPLOY_API_KEY").unwrap();
        (client, api_key)
    }

    async fn spawn_test_server(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let address = listener.local_addr().expect("test server address");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test API");
        });
        format!("http://{address}")
    }

    #[tokio::test]
    #[ignore] // Requires environment variables
    async fn test_find_compose_id() {
        let (client, api_key) = client_with_api_key();

        let res = dbg!(client.find_compose_by_name(&api_key, "pr-1774").await);
        assert!(res.is_ok());
    }

    #[test]
    fn sanitizes_sensitive_json_fields_and_known_credentials() {
        let body = br#"{
            "message": "request failed for known-api-key",
            "password": "do-not-log",
            "nested": { "access_token": "also-do-not-log" }
        }"#;

        let sanitized = sanitize_error_body(body, "known-api-key");

        assert!(sanitized.contains("request failed for [REDACTED]"));
        assert!(!sanitized.contains("known-api-key"));
        assert!(!sanitized.contains("do-not-log"));
        assert!(!sanitized.contains("also-do-not-log"));
    }

    #[test]
    fn sanitizes_sensitive_fields_in_truncated_responses() {
        let body = format!(
            r#"{{"password":"do-not-log","message":"{}""#,
            "x".repeat(ERROR_RESPONSE_BODY_LIMIT)
        );

        let sanitized = sanitize_error_body(
            &body.as_bytes()[..ERROR_RESPONSE_BODY_LIMIT],
            "known-api-key",
        );

        assert!(sanitized.contains(r#""password":"[REDACTED]""#));
        assert!(!sanitized.contains("do-not-log"));
    }

    #[tokio::test]
    async fn failed_request_reports_status_and_bounded_sanitized_body() {
        let api_key = "known-api-key";
        let response_body = format!("upstream echoed {api_key}\n{}", "x".repeat(4096));
        let app = Router::new().route(
            "/compose.delete",
            post(move || {
                let response_body = response_body.clone();
                async move { (StatusCode::BAD_GATEWAY, response_body) }
            }),
        );
        let client = DokployClient::new(spawn_test_server(app).await);

        let error = client
            .delete_compose(api_key, "compose-id", true)
            .await
            .expect_err("delete should fail")
            .to_string();

        assert!(error.contains("compose.delete"));
        assert!(error.contains("502 Bad Gateway"));
        assert!(error.contains("upstream echoed [REDACTED]"));
        assert!(error.contains("<truncated>"));
        assert!(!error.contains(api_key));
        assert!(error.len() < ERROR_RESPONSE_BODY_LIMIT + 256);
    }

    #[tokio::test]
    async fn delete_is_idempotent_when_compose_is_already_absent() {
        let app = Router::new().route(
            "/compose.delete",
            post(|| async { (StatusCode::NOT_FOUND, "compose not found") }),
        );
        let client = DokployClient::new(spawn_test_server(app).await);

        client
            .delete_compose("known-api-key", "already-deleted", true)
            .await
            .expect("404 delete should be idempotent success");
    }
}
