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
pub struct Subject {
    id: Uuid,
    school_id: Uuid,
    name: String,
    created_at: DateTime<Utc>,
}

#[derive(Deserialize, Validate)]
pub struct UpsertSubject {
    #[validate(length(min = 1, max = 200))]
    name: String,
}

async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<UpsertSubject>,
) -> AppResult<Json<Subject>> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    body.validate()?;
    let subject = sqlx::query_as::<_, Subject>(
        "INSERT INTO subjects (id, school_id, name) VALUES ($1,$2,$3)
         RETURNING id, school_id, name, created_at",
    )
    .bind(Uuid::new_v4())
    .bind(auth.school()?)
    .bind(&body.name)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(subject))
}

async fn list(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Vec<Subject>>> {
    let subjects = match auth.role {
        Role::Superadmin => {
            sqlx::query_as::<_, Subject>(
                "SELECT id, school_id, name, created_at FROM subjects ORDER BY school_id, name",
            )
            .fetch_all(&state.db)
            .await?
        }
        _ => {
            sqlx::query_as::<_, Subject>(
                "SELECT id, school_id, name, created_at FROM subjects
                 WHERE school_id = $1 ORDER BY name",
            )
            .bind(auth.school_id)
            .fetch_all(&state.db)
            .await?
        }
    };
    Ok(Json(subjects))
}

async fn update(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpsertSubject>,
) -> AppResult<Json<Subject>> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    body.validate()?;
    let subject = sqlx::query_as::<_, Subject>(
        "UPDATE subjects SET name = $2 WHERE id = $1 AND school_id = $3
         RETURNING id, school_id, name, created_at",
    )
    .bind(id)
    .bind(&body.name)
    .bind(auth.school()?)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(subject))
}

async fn delete(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<axum::http::StatusCode> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    let res = sqlx::query("DELETE FROM subjects WHERE id = $1 AND school_id = $2")
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
        .route("/{id}", get(get_one).patch(update).delete(delete))
}

async fn get_one(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Subject>> {
    let subject = match auth.role {
        Role::Superadmin => sqlx::query_as::<_, Subject>(
            "SELECT id, school_id, name, created_at FROM subjects WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&state.db)
        .await?,
        _ => sqlx::query_as::<_, Subject>(
            "SELECT id, school_id, name, created_at FROM subjects
             WHERE id = $1 AND school_id = $2",
        )
        .bind(id)
        .bind(auth.school_id)
        .fetch_optional(&state.db)
        .await?,
    }
    .ok_or(AppError::NotFound)?;
    Ok(Json(subject))
}
