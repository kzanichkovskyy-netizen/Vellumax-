use axum::extract::{Path, Query, State};
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
pub struct Lead {
    id: Uuid,
    parent_name: String,
    parent_phone: Option<String>,
    parent_email: Option<String>,
    child_name: Option<String>,
    notes: Option<String>,
    status: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

const LEAD_COLS: &str = "id, parent_name, parent_phone, parent_email, child_name, notes, status, created_at, updated_at";

#[derive(Deserialize, Validate)]
pub struct CreateLead {
    #[validate(length(min = 1, max = 200))]
    parent_name: String,
    parent_phone: Option<String>,
    #[validate(email)]
    parent_email: Option<String>,
    child_name: Option<String>,
    notes: Option<String>,
}

#[derive(Deserialize, Validate)]
pub struct UpdateLead {
    parent_name: Option<String>,
    parent_phone: Option<String>,
    #[validate(email)]
    parent_email: Option<String>,
    child_name: Option<String>,
    notes: Option<String>,
    status: Option<String>,
}

#[derive(Deserialize)]
pub struct LeadQuery {
    status: Option<String>,
}

fn valid_status(s: &str) -> bool {
    ["new", "contacted", "trial", "enrolled", "rejected"].contains(&s)
}

async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateLead>,
) -> AppResult<Json<Lead>> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    body.validate()?;
    let l = sqlx::query_as::<_, Lead>(sqlx::AssertSqlSafe(format!(
        "INSERT INTO leads (id, school_id, parent_name, parent_phone, parent_email, child_name, notes, created_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING {LEAD_COLS}"
    )))
    .bind(Uuid::new_v4())
    .bind(auth.school()?)
    .bind(&body.parent_name)
    .bind(&body.parent_phone)
    .bind(&body.parent_email)
    .bind(&body.child_name)
    .bind(&body.notes)
    .bind(auth.id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(l))
}

async fn list(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<LeadQuery>,
) -> AppResult<Json<Vec<Lead>>> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    let rows = sqlx::query_as::<_, Lead>(sqlx::AssertSqlSafe(format!(
        "SELECT {LEAD_COLS} FROM leads WHERE school_id = $1
           AND ($2::text IS NULL OR status = $2) ORDER BY updated_at DESC LIMIT 500"
    )))
    .bind(auth.school()?)
    .bind(&q.status)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

async fn update(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateLead>,
) -> AppResult<Json<Lead>> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    body.validate()?;
    if let Some(s) = &body.status {
        if !valid_status(s) {
            return Err(AppError::BadRequest("invalid status".into()));
        }
    }
    let l = sqlx::query_as::<_, Lead>(sqlx::AssertSqlSafe(format!(
        "UPDATE leads SET
            parent_name = COALESCE($3, parent_name),
            parent_phone = COALESCE($4, parent_phone),
            parent_email = COALESCE($5, parent_email),
            child_name = COALESCE($6, child_name),
            notes = COALESCE($7, notes),
            status = COALESCE($8, status),
            updated_at = now()
         WHERE id = $1 AND school_id = $2 RETURNING {LEAD_COLS}"
    )))
    .bind(id)
    .bind(auth.school()?)
    .bind(&body.parent_name)
    .bind(&body.parent_phone)
    .bind(&body.parent_email)
    .bind(&body.child_name)
    .bind(&body.notes)
    .bind(&body.status)
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
    let res = sqlx::query("DELETE FROM leads WHERE id = $1 AND school_id = $2")
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
