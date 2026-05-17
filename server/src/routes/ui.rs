use axum::{
    response::{Html, IntoResponse},
};

/// Serves the single-page, responsive Web UI directly from static embedded resources.
pub async fn ui_handler() -> impl IntoResponse {
    Html(include_str!("../../static/index.html"))
}
