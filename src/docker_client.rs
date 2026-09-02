use std::collections::HashMap;

use bollard::Docker;
use bollard::container::{ListContainersOptions, LogsOptions};
use futures_util::StreamExt;
use secrecy::SecretString;
use tokio::sync::mpsc;
use url::Url;

/// A wrapper around the Docker client for container log streaming.
#[derive(Clone)]
pub struct DockerClient {
    docker: Docker,
}

impl DockerClient {
    /// Creates a new DockerClient connecting to the local Docker socket.
    /// Expects /var/run/docker.sock to be mounted.
    pub fn new() -> Result<Self, bollard::errors::Error> {
        let docker = Docker::connect_with_socket_defaults()?;
        Ok(Self { docker })
    }

    /// Streams logs from a container by name.
    /// Returns a receiver that yields log lines as they arrive.
    ///
    /// # Arguments
    /// * `container_name` - The container name (not ID)
    /// * `tail` - Number of lines to return from the end of the logs (0 = all)
    /// * `follow` - Whether to follow the log stream (like `tail -f`)
    pub async fn stream_logs(
        &self,
        container_name: &str,
        tail: u64,
        follow: bool,
    ) -> Result<mpsc::Receiver<Result<String, String>>, String> {
        // Verify container exists first
        self.docker
            .inspect_container(container_name, None)
            .await
            .map_err(|e| format!("Container '{}' not found: {}", container_name, e))?;

        let (tx, rx) = mpsc::channel(100);

        let options = LogsOptions::<String> {
            follow,
            stdout: true,
            stderr: true,
            tail: if tail > 0 {
                tail.to_string()
            } else {
                "all".to_string()
            },
            timestamps: true,
            ..Default::default()
        };

        let docker = self.docker.clone();
        let container = container_name.to_string();

        tokio::spawn(async move {
            let mut stream = docker.logs(&container, Some(options));

            while let Some(result) = stream.next().await {
                let msg = match result {
                    Ok(output) => Ok(output.to_string()),
                    Err(e) => Err(format!("Log stream error: {}", e)),
                };

                if tx.send(msg).await.is_err() {
                    // Receiver dropped, stop streaming
                    break;
                }
            }
        });

        Ok(rx)
    }

    /// Reads the browser login token emitted by an Aspire dashboard container.
    ///
    /// The token remains wrapped as a secret so callers cannot accidentally
    /// include it in debug output or structured logs.
    pub async fn aspire_dashboard_login_token(
        &self,
        container_name: &str,
    ) -> Result<Option<SecretString>, String> {
        self.docker
            .inspect_container(container_name, None)
            .await
            .map_err(|error| format!("Aspire dashboard container not found: {error}"))?;

        let options = LogsOptions::<String> {
            follow: false,
            stdout: true,
            stderr: true,
            tail: "1000".to_string(),
            timestamps: false,
            ..Default::default()
        };
        let mut logs = self.docker.logs(container_name, Some(options));
        let mut token = None;

        while let Some(output) = logs.next().await {
            let output = output.map_err(|error| {
                format!("Failed to read Aspire dashboard container logs: {error}")
            })?;
            if let Some(parsed) = parse_aspire_dashboard_login_token(&output.to_string()) {
                token = Some(parsed);
            }
        }

        Ok(token)
    }

    /// Lists all containers matching a name filter.
    pub async fn list_containers(
        &self,
        name_filter: Option<&str>,
    ) -> Result<Vec<ContainerInfo>, String> {
        let mut filters = HashMap::new();
        if let Some(name) = name_filter {
            filters.insert("name", vec![name]);
        }

        let options = ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        };

        let containers = self
            .docker
            .list_containers(Some(options))
            .await
            .map_err(|e| format!("Failed to list containers: {}", e))?;

        Ok(containers
            .into_iter()
            .map(|c| ContainerInfo {
                compose_service: c
                    .labels
                    .as_ref()
                    .and_then(|labels| labels.get("com.docker.compose.service"))
                    .cloned(),
                id: c.id.unwrap_or_default(),
                names: c.names.unwrap_or_default(),
                image: c.image.unwrap_or_default(),
                state: c.state.unwrap_or_default(),
                status: c.status.unwrap_or_default(),
            })
            .collect())
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ContainerInfo {
    pub compose_service: Option<String>,
    pub id: String,
    pub names: Vec<String>,
    pub image: String,
    pub state: String,
    pub status: String,
}

fn parse_aspire_dashboard_login_token(line: &str) -> Option<SecretString> {
    let (_, login_url) = line.split_once("Login URL:")?;
    let login_url = Url::parse(login_url.trim()).ok()?;
    login_url.query_pairs().find_map(|(key, value)| {
        (key == "t" && !value.is_empty()).then(|| SecretString::from(value.into_owned()))
    })
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret as _;

    use super::*;

    #[test]
    fn parses_aspire_dashboard_browser_token() {
        let token = parse_aspire_dashboard_login_token(
            "      - Login URL:  http://localhost:18888/login?t=test-token%2Bvalue",
        )
        .expect("login URL should contain a token");

        assert_eq!(token.expose_secret(), "test-token+value");
    }

    #[test]
    fn ignores_dashboard_log_lines_without_a_browser_token() {
        assert!(parse_aspire_dashboard_login_token("Aspire Dashboard").is_none());
        assert!(
            parse_aspire_dashboard_login_token("      - Login URL:  http://localhost:18888/login")
                .is_none()
        );
    }
}
