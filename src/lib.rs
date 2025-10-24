pub mod azure_client;
pub mod config;
pub mod dokploy_client;
pub mod models;
pub mod slash_cmd;

pub use config::Config;
pub use dokploy_client::DokployClient;
pub use models::dokploy::*;
pub use slash_cmd::*;

/// Computes the identifier for Dokploy preview deployments.
/// Prefers PR number if provided, otherwise uses sanitized branch name.
/// Returns "pr-{pr_number}" or "br-{sanitized_branch}".
pub fn compute_identifier(pr_number: &Option<String>, branch_name: &str) -> String {
    if let Some(pr) = pr_number
        && !pr.is_empty()
    {
        return format!("pr-{}", pr);
    }

    let sanitized = branch_name.replace("/", "-").to_lowercase();
    format!("br-{}", sanitized)
}

pub fn parse_ts(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

/// Strips the common Git refs/heads/ prefix from a branch ref if present.
/// Returns the original string when the prefix is absent.
pub fn strip_refs_heads(s: &str) -> String {
    s.strip_prefix("refs/heads/").unwrap_or(s).to_string()
}

/// Test-only helper to ensure required Dokploy env vars are loaded.
/// If `DOKPLOY_URL` or `DOKPLOY_API_KEY` are missing, it attempts to
/// load them from a `.env.local` file at the crate root. Existing
/// environment variables are never overwritten.
pub fn test_init_env() {
    let need_url = std::env::var("DOKPLOY_URL").is_err();
    let need_key = std::env::var("DOKPLOY_API_KEY").is_err();
    if need_url || need_key {
        let _ = dotenvy::from_filename(".env.local");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_identifier() {
        assert_eq!(
            compute_identifier(&None, "feature/branch"),
            "br-feature-branch"
        );
        assert_eq!(
            compute_identifier(&Some("42".to_string()), "feature/branch"),
            "pr-42"
        );
        assert_eq!(compute_identifier(&None, "MAIN"), "br-main");
        assert_eq!(compute_identifier(&Some("42".to_string()), "MAIN"), "pr-42");
    }

    #[test]
    fn test_strip_refs_heads() {
        assert_eq!(strip_refs_heads("refs/heads/main"), "main");
        assert_eq!(strip_refs_heads("refs/heads/feature/cool"), "feature/cool");
        assert_eq!(strip_refs_heads("main"), "main");
        assert_eq!(strip_refs_heads(""), "");
    }
}
