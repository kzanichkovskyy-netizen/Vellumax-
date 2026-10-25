use sqlx::{PgPool, postgres::PgPoolOptions};

pub async fn connect(url: &str) -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(url)
        .await
        .expect("failed to connect to database");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("failed to run migrations");
    pool
}
