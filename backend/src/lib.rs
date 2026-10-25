pub mod analytics;
pub mod announcements;
pub mod auth;
pub mod classes;
pub mod config;
pub mod db;
pub mod documents;
pub mod error;
pub mod events;
pub mod finance;
pub mod homework;
pub mod journal;
pub mod leads;
pub mod locations;
pub mod materials;
pub mod meals;
pub mod messages;
pub mod notifications;
pub mod pickups;
pub mod polls;
pub mod quizzes;
pub mod rewards;
pub mod schedule;
pub mod schools;
pub mod subjects;
pub mod tasks;
pub mod transport;
pub mod types;
pub mod users;

use std::sync::Arc;

use axum::Router;
use sqlx::PgPool;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
}

pub fn build_router(state: AppState) -> Router {
    let api = Router::new()
        .nest("/auth", auth::router())
        .nest("/users", users::router())
        .nest("/schools", schools::router())
        .nest("/classes", classes::router())
        .nest("/subjects", subjects::router())
        .nest("/schedule", schedule::router())
        .nest("/homework", homework::router())
        .nest("/journal", journal::router())
        .nest("/materials", materials::router())
        .nest("/announcements", announcements::router())
        .nest("/messages", messages::router())
        .nest("/notifications", notifications::router())
        .nest("/finance", finance::router())
        .nest("/meals", meals::router())
        .nest("/quizzes", quizzes::router())
        .nest("/rewards", rewards::router())
        .nest("/events", events::router())
        .nest("/polls", polls::router())
        .nest("/tasks", tasks::router())
        .nest("/pickups", pickups::router())
        .nest("/documents", documents::router())
        .nest("/locations", locations::router())
        .nest("/transport", transport::router())
        .nest("/leads", leads::router())
        .nest("/analytics", analytics::router());

    let static_dir = state.config.static_dir.clone();
    Router::new()
        .nest("/api/v1", api)
        .route("/health", axum::routing::get(|| async { "ok" }))
        .fallback_service(tower_http::services::ServeDir::new(static_dir))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(RequestBodyLimitLayer::new(50 * 1024 * 1024))
        .with_state(state)
}
