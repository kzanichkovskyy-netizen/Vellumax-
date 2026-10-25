use axum::extract::{Path, Query, State};
use axum::routing::{get, patch};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};

#[derive(Serialize, sqlx::FromRow)]
pub struct Notification {
    id: Uuid,
    #[serde(rename = "type")]
    #[sqlx(rename = "type")]
    ntype: String,
    payload: serde_json::Value,
    read_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct ListQuery {
    unread: Option<bool>,
}

async fn list(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<Vec<Notification>>> {
    let rows = if q.unread == Some(true) {
        sqlx::query_as::<_, Notification>(
            "SELECT id, type, payload, read_at, created_at FROM notifications
             WHERE user_id = $1 AND read_at IS NULL ORDER BY created_at DESC LIMIT 100",
        )
        .bind(auth.id)
        .fetch_all(&state.db)
        .await?
    } else {
        sqlx::query_as::<_, Notification>(
            "SELECT id, type, payload, read_at, created_at FROM notifications
             WHERE user_id = $1 ORDER BY created_at DESC LIMIT 100",
        )
        .bind(auth.id)
        .fetch_all(&state.db)
        .await?
    };
    Ok(Json(rows))
}

async fn unread_count(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM notifications WHERE user_id = $1 AND read_at IS NULL",
    )
    .bind(auth.id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(serde_json::json!({ "unread": n })))
}

async fn mark_read(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<axum::http::StatusCode> {
    let res =
        sqlx::query("UPDATE notifications SET read_at = now() WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(auth.id)
            .execute(&state.db)
            .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn mark_all_read(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<axum::http::StatusCode> {
    sqlx::query("UPDATE notifications SET read_at = now() WHERE user_id = $1 AND read_at IS NULL")
        .bind(auth.id)
        .execute(&state.db)
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ---- internal producers ----

/// Notify a single user. Fire-and-forget: notification failure must not
/// break the triggering request.
pub(crate) async fn notify(
    state: &AppState,
    user_id: Uuid,
    ntype: &str,
    payload: serde_json::Value,
) {
    let _ =
        sqlx::query("INSERT INTO notifications (id, user_id, type, payload) VALUES ($1,$2,$3,$4)")
            .bind(Uuid::new_v4())
            .bind(user_id)
            .bind(ntype)
            .bind(payload)
            .execute(&state.db)
            .await
            .map_err(|e| tracing::warn!(?e, "notification insert failed"));
}

/// Notify every enrolled student of a class plus their parents.
pub(crate) async fn notify_class(
    state: &AppState,
    class_id: Uuid,
    ntype: &str,
    payload: serde_json::Value,
) {
    let _ = sqlx::query(
        "INSERT INTO notifications (id, user_id, type, payload)
         SELECT gen_random_uuid(), uid, $2, $3 FROM (
            SELECT student_id AS uid FROM enrollments WHERE class_id = $1
            UNION
            SELECT pl.parent_id FROM parent_links pl
            JOIN enrollments e ON e.student_id = pl.student_id
            WHERE e.class_id = $1
         ) t",
    )
    .bind(class_id)
    .bind(ntype)
    .bind(payload)
    .execute(&state.db)
    .await
    .map_err(|e| tracing::warn!(?e, "class notification insert failed"));
}

/// Notify a student and all linked parents.
pub(crate) async fn notify_student_and_parents(
    state: &AppState,
    student_id: Uuid,
    ntype: &str,
    payload: serde_json::Value,
) {
    let _ = sqlx::query(
        "INSERT INTO notifications (id, user_id, type, payload)
         SELECT gen_random_uuid(), uid, $2, $3 FROM (
            SELECT $1::uuid AS uid
            UNION SELECT parent_id FROM parent_links WHERE student_id = $1
         ) t",
    )
    .bind(student_id)
    .bind(ntype)
    .bind(payload)
    .execute(&state.db)
    .await
    .map_err(|e| tracing::warn!(?e, "student notification insert failed"));
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list))
        .route("/unread-count", get(unread_count))
        .route("/read-all", patch(mark_all_read))
        .route("/{id}/read", patch(mark_read))
}
