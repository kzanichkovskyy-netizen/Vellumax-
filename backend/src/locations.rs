use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use validator::Validate;

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::types::Role;

#[derive(Serialize, sqlx::FromRow)]
pub struct Location {
    id: Uuid,
    name: String,
    address: Option<String>,
}

#[derive(Deserialize, Validate)]
pub struct UpsertLocation {
    #[validate(length(min = 1, max = 200))]
    name: String,
    address: Option<String>,
}

async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<UpsertLocation>,
) -> AppResult<Json<Location>> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    body.validate()?;
    let l = sqlx::query_as::<_, Location>(
        "INSERT INTO locations (id, school_id, name, address) VALUES ($1,$2,$3,$4)
         RETURNING id, name, address",
    )
    .bind(Uuid::new_v4())
    .bind(auth.school()?)
    .bind(&body.name)
    .bind(&body.address)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(l))
}

async fn list(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Vec<Location>>> {
    let rows = sqlx::query_as::<_, Location>(
        "SELECT id, name, address FROM locations WHERE school_id = $1 ORDER BY name",
    )
    .bind(auth.school()?)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

async fn update(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpsertLocation>,
) -> AppResult<Json<Location>> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    body.validate()?;
    let l = sqlx::query_as::<_, Location>(
        "UPDATE locations SET name = $3, address = $4 WHERE id = $1 AND school_id = $2
         RETURNING id, name, address",
    )
    .bind(id)
    .bind(auth.school()?)
    .bind(&body.name)
    .bind(&body.address)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(l))
}

async fn delete(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<axum::http::StatusCode> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    let res = sqlx::query("DELETE FROM locations WHERE id = $1 AND school_id = $2")
        .bind(id)
        .bind(auth.school()?)
        .execute(&state.db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", axum::routing::patch(update).delete(delete))
}
