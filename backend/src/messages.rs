use axum::extract::{Path, State};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use validator::Validate;

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};

#[derive(Serialize, sqlx::FromRow)]
pub struct Message {
    id: Uuid,
    sender_id: Uuid,
    recipient_id: Uuid,
    body: String,
    read_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Conversation {
    partner_id: Uuid,
    first_name: String,
    last_name: String,
    last_message: String,
    last_at: DateTime<Utc>,
    unread: i64,
}

#[derive(Deserialize, Validate)]
pub struct SendMessage {
    recipient_id: Uuid,
    #[validate(length(min = 1, max = 4000))]
    body: String,
}

async fn send(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<SendMessage>,
) -> AppResult<Json<Message>> {
    body.validate()?;
    if body.recipient_id == auth.id {
        return Err(AppError::BadRequest("cannot message yourself".into()));
    }
    let same_school: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1 AND school_id = $2)")
            .bind(body.recipient_id)
            .bind(auth.school_id)
            .fetch_one(&state.db)
            .await?;
    if !same_school {
        return Err(AppError::NotFound);
    }
    let m = sqlx::query_as::<_, Message>(
        "INSERT INTO messages (id, school_id, sender_id, recipient_id, body)
         VALUES ($1,$2,$3,$4,$5)
         RETURNING id, sender_id, recipient_id, body, read_at, created_at",
    )
    .bind(Uuid::new_v4())
    .bind(auth.school()?)
    .bind(auth.id)
    .bind(body.recipient_id)
    .bind(&body.body)
    .fetch_one(&state.db)
    .await?;
    crate::notifications::notify(
        &state,
        body.recipient_id,
        "message",
        serde_json::json!({ "from": auth.id }),
    )
    .await;
    Ok(Json(m))
}

async fn thread(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(partner): Path<Uuid>,
) -> AppResult<Json<Vec<Message>>> {
    let msgs = sqlx::query_as::<_, Message>(
        "SELECT id, sender_id, recipient_id, body, read_at, created_at
         FROM messages
         WHERE (sender_id = $1 AND recipient_id = $2)
            OR (sender_id = $2 AND recipient_id = $1)
         ORDER BY created_at LIMIT 500",
    )
    .bind(auth.id)
    .bind(partner)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(msgs))
}

async fn mark_read(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(partner): Path<Uuid>,
) -> AppResult<axum::http::StatusCode> {
    sqlx::query(
        "UPDATE messages SET read_at = now()
         WHERE recipient_id = $1 AND sender_id = $2 AND read_at IS NULL",
    )
    .bind(auth.id)
    .bind(partner)
    .execute(&state.db)
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn conversations(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<Conversation>>> {
    let rows = sqlx::query_as::<_, Conversation>(
        "SELECT u.id AS partner_id, u.first_name, u.last_name,
                m.body AS last_message, m.created_at AS last_at,
                (SELECT COUNT(*) FROM messages
                  WHERE sender_id = u.id AND recipient_id = $1 AND read_at IS NULL) AS unread
         FROM users u
         JOIN messages m ON m.id = (
            SELECT id FROM messages
            WHERE (sender_id = $1 AND recipient_id = u.id)
               OR (sender_id = u.id AND recipient_id = $1)
            ORDER BY created_at DESC LIMIT 1)
         ORDER BY m.created_at DESC",
    )
    .bind(auth.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", post(send))
        .route("/conversations", get(conversations))
        .route("/with/{id}", get(thread))
        .route("/with/{id}/read", patch(mark_read))
}
