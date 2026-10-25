#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub jwt_secret: String,
    pub port: u16,
    pub upload_dir: String,
    pub access_token_ttl_secs: i64,
    pub refresh_token_ttl_days: i64,
}

impl Config {
    pub fn from_env() -> Self {
        dotenvy::dotenv().ok();
        Self {
            database_url: std::env::var("DATABASE_URL").expect("DATABASE_URL is required"),
            jwt_secret: std::env::var("JWT_SECRET")
                .unwrap_or_else(|_| "dev-secret-change-me".into()),
            port: std::env::var("PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3000),
            upload_dir: std::env::var("UPLOAD_DIR").unwrap_or_else(|_| "uploads".into()),
            access_token_ttl_secs: std::env::var("ACCESS_TOKEN_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3600),
            refresh_token_ttl_days: 30,
        }
    }
}
