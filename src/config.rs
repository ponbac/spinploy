use anyhow::{Context, Result};
use config::{Config as ConfigBuilder, Environment};
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub dokploy_url: String,
    pub environment_id: String,
    pub base_domain: String,
    pub frontend_service_name: String,
    pub frontend_port: u16,
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
    // Deployed Preview API path
    pub deployed_preview_api_path: String,
    // VM-local Aspire preview builder settings
    #[serde(default = "default_preview_work_dir")]
    pub preview_work_dir: String,
    #[serde(default = "default_preview_host_work_dir")]
    pub preview_host_work_dir: String,
    #[serde(default = "default_preview_host_cache_dir")]
    pub preview_host_cache_dir: String,
    #[serde(default = "default_preview_cache_dir")]
    pub preview_cache_dir: String,
    #[serde(default)]
    pub preview_builder_image: Option<String>,
    #[serde(default = "default_preview_apphost_path")]
    pub preview_apphost_path: String,
    #[serde(default = "default_preview_frontend_path")]
    pub preview_frontend_path: String,
    #[serde(default = "default_preview_build_timeout_secs")]
    pub preview_build_timeout_secs: u64,
    #[serde(default = "default_preview_readiness_timeout_secs")]
    pub preview_readiness_timeout_secs: u64,
}

fn default_auth_cache_ttl() -> u64 {
    60
}

fn default_auth_cache_negative_ttl() -> u64 {
    10
}

fn default_preview_work_dir() -> String {
    "/data/previews".to_string()
}

fn default_preview_host_work_dir() -> String {
    "/home/ponbac/shared/previews".to_string()
}

fn default_preview_host_cache_dir() -> String {
    "/home/ponbac/shared/preview-cache".to_string()
}

fn default_preview_cache_dir() -> String {
    "/data/preview-cache".to_string()
}

fn default_preview_apphost_path() -> String {
    "LD.Apport.Backend/LD.Apport.AppHost/LD.Apport.AppHost.csproj".to_string()
}

fn default_preview_frontend_path() -> String {
    "LD.Apport.Frontend".to_string()
}

fn default_preview_build_timeout_secs() -> u64 {
    45 * 60
}

fn default_preview_readiness_timeout_secs() -> u64 {
    10 * 60
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
