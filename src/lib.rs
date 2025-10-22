pub mod dokploy_client;
pub mod models;

pub use dokploy_client::*;
pub use models::*;

/// Test-only helper to ensure required Dokploy env vars are loaded.
/// If `DOKPLOY_URL` or `DOKPLOY_API_KEY` are missing, it attempts to
/// load them from a `.env.local` file at the crate root. Existing
/// environment variables are never overwritten.
#[cfg(test)]
pub fn test_init_env() {
    let need_url = std::env::var("DOKPLOY_URL").is_err();
    let need_key = std::env::var("DOKPLOY_API_KEY").is_err();
    if need_url || need_key {
        let _ = dotenvy::from_filename(".env.local");
    }
}
