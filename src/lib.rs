pub mod dokploy_client;
pub mod models;

pub use dokploy_client::*;
pub use models::*;

/// Computes the identifier for Dokploy preview deployments.
/// Prefers PR number if provided, otherwise uses sanitized branch name.
/// Returns "pr-{pr_number}" or "br-{sanitized_branch}".
pub fn compute_identifier(
    pr_number: Option<String>,
    branch_name: Option<String>,
) -> Result<String, String> {
    if let Some(pr) = pr_number
        && !pr.is_empty()
    {
        return Ok(format!("pr-{}", pr));
    }
    if let Some(branch) = branch_name
        && !branch.is_empty()
    {
        let sanitized = branch.replace("/", "-").to_lowercase();
        return Ok(format!("br-{}", sanitized));
    }
    Err("Could not determine identifier: no PR number or branch name provided".to_string())
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
            compute_identifier(Some("42".to_string()), None).unwrap(),
            "pr-42"
        );
        assert_eq!(
            compute_identifier(None, Some("feature/branch".to_string())).unwrap(),
            "br-feature-branch"
        );
        assert_eq!(
            compute_identifier(None, Some("MAIN".to_string())).unwrap(),
            "br-main"
        );
        assert!(compute_identifier(None, None).is_err());
        assert!(compute_identifier(Some("".to_string()), None).is_err());
        assert!(compute_identifier(None, Some("".to_string())).is_err());
    }
}
