use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::types::Role;

#[derive(Serialize, sqlx::FromRow)]
pub struct PickupCall {
    id: Uuid,
    student_id: Uuid,
    requested_by: Uuid,
    note: Option<String>,
    status: String,
    created_at: DateTime<Utc>,
    done_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
pub struct CallRequest {
    student_id: Uuid,
    note: Option<String>,
}

/// "Покликати дитину" — a parent summons their child; the student, their
/// homeroom teacher and school admins get notified.
async fn call(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CallRequest>,
) -> AppResult<Json<PickupCall>> {
    let is_parent: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM parent_links WHERE parent_id=$1 AND student_id=$2)",
    )
    .bind(auth.id)
    .bind(body.student_id)
    .fetch_one(&state.db)
    .await?;
    if !(is_parent || matches!(auth.role, Role::Admin | Role::Superadmin)) {
        return Err(AppError::Forbidden);
    }

    let p = sqlx::query_as::<_, PickupCall>(
        "INSERT INTO pickup_calls (id, student_id, requested_by, note)
         VALUES ($1,$2,$3,$4)
         RETURNING id, student_id, requested_by, note, status, created_at, done_at",
    )
    .bind(Uuid::new_v4())
    .bind(body.student_id)
    .bind(auth.id)
    .bind(&body.note)
    .fetch_one(&state.db)
    .await?;

    // notify the student + homeroom teachers of their classes + school admins
    let payload = serde_json::json!({ "pickup_id": p.id, "student_id": body.student_id });
    sqlx::query(
        "INSERT INTO notifications (id, user_id, type, payload)
         SELECT gen_random_uuid(), uid, 'pickup', $2 FROM (
            SELECT $1::uuid AS uid
            UNION SELECT c.homeroom_teacher_id FROM classes c
              JOIN enrollments e ON e.class_id = c.id
              WHERE e.student_id = $1 AND c.homeroom_teacher_id IS NOT NULL
            UNION SELECT u.id FROM users u
              WHERE u.school_id = (SELECT school_id FROM users WHERE id = $1)
                AND u.role = 'admin'
         ) t",
    )
    .bind(body.student_id)
    .bind(&payload)
    .execute(&state.db)
    .await
    .ok();

    Ok(Json(p))
}

async fn list(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Vec<PickupCall>>> {
    let rows = match auth.role {
        Role::Superadmin | Role::Admin | Role::Teacher => sqlx::query_as::<_, PickupCall>(
            "SELECT p.id, p.student_id, p.requested_by, p.note, p.status, p.created_at, p.done_at
                 FROM pickup_calls p JOIN users u ON u.id = p.student_id
                 WHERE u.school_id = $1 ORDER BY p.created_at DESC LIMIT 100",
        )
        .bind(auth.school_id)
        .fetch_all(&state.db)
        .await?,
        Role::Student => {
            sqlx::query_as::<_, PickupCall>(
                "SELECT id, student_id, requested_by, note, status, created_at, done_at
                 FROM pickup_calls WHERE student_id = $1 ORDER BY created_at DESC LIMIT 50",
            )
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Parent => sqlx::query_as::<_, PickupCall>(
            "SELECT p.id, p.student_id, p.requested_by, p.note, p.status, p.created_at, p.done_at
                 FROM pickup_calls p JOIN parent_links pl ON pl.student_id = p.student_id
                 WHERE pl.parent_id = $1 ORDER BY p.created_at DESC LIMIT 50",
        )
        .bind(auth.id)
        .fetch_all(&state.db)
        .await?,
    };
    Ok(Json(rows))
}

async fn done(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<PickupCall>> {
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher, Role::Student])?;
    let p = sqlx::query_as::<_, PickupCall>(
        "UPDATE pickup_calls SET status = 'done', done_at = now()
         WHERE id = $1 AND status = 'pending'
           AND ($2 OR student_id = $3)
         RETURNING id, student_id, requested_by, note, status, created_at, done_at",
    )
    .bind(id)
    .bind(matches!(
        auth.role,
        Role::Admin | Role::Superadmin | Role::Teacher
    ))
    .bind(auth.id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(p))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(call))
        .route("/{id}/done", post(done))
}
