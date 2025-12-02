use std::collections::HashMap;

use bollard::container::{ListContainersOptions, LogsOptions};
use bollard::Docker;
use futures_util::StreamExt;
use tokio::sync::mpsc;

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
    pub id: String,
    pub names: Vec<String>,
    pub image: String,
    pub state: String,
    pub status: String,
}
