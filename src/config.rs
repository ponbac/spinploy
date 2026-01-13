use anyhow::{Context, Result};
use config::{Config as ConfigBuilder, Environment};
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub dokploy_url: String,
    pub project_id: String,
    pub environment_id: String,
    pub custom_git_url: String,
    pub custom_git_ssh_key_id: String,
    pub compose_path: String,
    pub base_domain: String,
    pub frontend_service_name: String,
    pub frontend_port: u16,
    pub backend_service_name: String,
    pub backend_port: u16,
    // Azure DevOps configuration for posting PR comments
    pub azdo_org: String,
    pub azdo_project: String,
    pub azdo_repository_id: String,
    pub azdo_pat: String,
    // Slack Incoming Webhook URL for alerts
    pub slack_webhook_url: String,
    // Authentication cache settings
    #[serde(default = "default_auth_cache_ttl")]
    pub auth_cache_ttl_secs: u64,
    #[serde(default = "default_auth_cache_negative_ttl")]
    pub auth_cache_negative_ttl_secs: u64,
    // Optional protected storage settings
    pub storage: Option<StorageConfig>,
}

fn default_auth_cache_ttl() -> u64 {
    60
}

fn default_auth_cache_negative_ttl() -> u64 {
    10
}

#[derive(Debug, Deserialize, Clone)]
pub struct StorageConfig {
    pub base_url: String,
    pub dir: String,
    pub token: String,
}

impl Config {
    pub fn load() -> Result<Self> {
        #[cfg(debug_assertions)]
        dotenvy::from_filename(".env.local")?;

        let config = ConfigBuilder::builder()
            .add_source(Environment::default().separator("__"))
            .build()
            .context("Failed to build configuration")?;

        config
            .try_deserialize()
            .context("Failed to deserialize configuration")
    }
}
