use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use validator::Validate;

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::types::Role;

#[derive(Serialize, sqlx::FromRow)]
pub struct School {
    id: Uuid,
    name: String,
    settings: serde_json::Value,
    created_at: DateTime<Utc>,
}

#[derive(Deserialize, Validate)]
pub struct CreateSchool {
    #[validate(length(min = 1, max = 200))]
    name: String,
    #[serde(default)]
    settings: Option<serde_json::Value>,
}

#[derive(Deserialize, Validate)]
pub struct UpdateSchool {
    #[validate(length(min = 1, max = 200))]
    name: Option<String>,
    settings: Option<serde_json::Value>,
}

async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateSchool>,
) -> AppResult<Json<School>> {
    auth.require(&[Role::Superadmin])?;
    body.validate()?;
    let school = sqlx::query_as::<_, School>(
        "INSERT INTO schools (id, name, settings) VALUES ($1,$2,COALESCE($3::jsonb, '{\"grading\":{\"type\":\"numeric\",\"min\":1,\"max\":12}}'::jsonb))
         RETURNING id, name, settings, created_at",
    )
    .bind(Uuid::new_v4())
    .bind(&body.name)
    .bind(&body.settings)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(school))
}

async fn list(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Vec<School>>> {
    match auth.role {
        Role::Superadmin => {
            let rows = sqlx::query_as::<_, School>(
                "SELECT id, name, settings, created_at FROM schools ORDER BY name",
            )
            .fetch_all(&state.db)
            .await?;
            Ok(Json(rows))
        }
        _ => {
            let rows = sqlx::query_as::<_, School>(
                "SELECT id, name, settings, created_at FROM schools WHERE id = $1",
            )
            .bind(auth.school()?)
            .fetch_all(&state.db)
            .await?;
            Ok(Json(rows))
        }
    }
}

async fn get_one(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<School>> {
    if auth.role != Role::Superadmin && auth.school()? != id {
        return Err(AppError::Forbidden);
    }
    let school = sqlx::query_as::<_, School>(
        "SELECT id, name, settings, created_at FROM schools WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(school))
}

async fn update(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateSchool>,
) -> AppResult<Json<School>> {
    if auth.role != Role::Superadmin && auth.school()? != id {
        return Err(AppError::Forbidden);
    }
    auth.require(&[Role::Superadmin, Role::Admin])?;
    body.validate()?;
    let school = sqlx::query_as::<_, School>(
        "UPDATE schools SET
            name = COALESCE($2, name),
            settings = COALESCE($3, settings)
         WHERE id = $1
         RETURNING id, name, settings, created_at",
    )
    .bind(id)
    .bind(&body.name)
    .bind(&body.settings)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(school))
}

async fn delete(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<axum::http::StatusCode> {
    auth.require(&[Role::Superadmin])?;
    let res = sqlx::query("DELETE FROM schools WHERE id = $1")
        .bind(id)
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
        .route("/{id}", get(get_one).patch(update).delete(delete))
}
