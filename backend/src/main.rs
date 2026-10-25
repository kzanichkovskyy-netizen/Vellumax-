use std::sync::Arc;

use classnode_backend::{AppState, build_router, config::Config, db};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "classnode_backend=info,tower_http=info".into()),
        )
        .init();

    let config = Config::from_env();
    let pool = db::connect(&config.database_url).await;

    std::fs::create_dir_all(&config.upload_dir).expect("failed to create upload dir");

    let state = AppState {
        db: pool,
        config: Arc::new(config),
    };
    let addr = format!("0.0.0.0:{}", state.config.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("failed to bind");
    tracing::info!("listening on {addr}");
    axum::serve(listener, build_router(state))
        .await
        .expect("server error");
}
