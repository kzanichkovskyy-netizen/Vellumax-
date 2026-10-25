use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Validation(#[from] validator::ValidationErrors),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

pub type AppResult<T> = Result<T, AppError>;

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            AppError::NotFound => (StatusCode::NOT_FOUND, "not found".into()),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized".into()),
            AppError::Forbidden => (StatusCode::FORBIDDEN, "forbidden".into()),
            AppError::Conflict(m) => (StatusCode::CONFLICT, m),
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            AppError::Validation(e) => (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()),
            AppError::Db(sqlx::Error::RowNotFound) => (StatusCode::NOT_FOUND, "not found".into()),
            AppError::Db(e) => {
                if let sqlx::Error::Database(dbe) = &e {
                    match dbe.code().as_deref() {
                        Some("23505") => {
                            return (
                                StatusCode::CONFLICT,
                                Json(serde_json::json!({"error": "already exists"})),
                            )
                                .into_response();
                        }
                        Some("23503") => {
                            return (
                                StatusCode::BAD_REQUEST,
                                Json(serde_json::json!({"error": "referenced entity not found"})),
                            )
                                .into_response();
                        }
                        _ => {}
                    }
                }
                tracing::error!(?e, "database error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
            }
        };
        (status, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}
