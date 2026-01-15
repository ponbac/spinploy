pub mod previews;
pub mod types;

use axum::{
    routing::get,
    Router,
};

use crate::AppState;

/// Create router for all API endpoints
pub fn preview_routes() -> Router<AppState> {
    Router::new()
        .route("/previews", get(previews::list_previews))
        .route("/previews/{identifier}", get(previews::get_preview_detail))
        .route(
            "/previews/{identifier}/containers/{service}/logs",
            get(previews::stream_preview_container_logs),
        )
}
