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
pub struct Event {
    id: Uuid,
    school_id: Uuid,
    title: String,
    description: Option<String>,
    start_at: DateTime<Utc>,
    end_at: Option<DateTime<Utc>>,
    audience: String,
    class_id: Option<Uuid>,
    #[serde(rename = "type")]
    #[sqlx(rename = "type")]
    etype: String,
    created_at: DateTime<Utc>,
}

#[derive(Deserialize, Validate)]
pub struct UpsertEvent {
    #[validate(length(min = 1, max = 300))]
    title: String,
    description: Option<String>,
    start_at: DateTime<Utc>,
    end_at: Option<DateTime<Utc>>,
    /// 'school' | 'class' | 'staff'
    audience: Option<String>,
    class_id: Option<Uuid>,
    #[serde(rename = "type")]
    etype: Option<String>,
}

async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<UpsertEvent>,
) -> AppResult<Json<Event>> {
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    body.validate()?;
    let audience = body.audience.as_deref().unwrap_or("school");
    if !["school", "class", "staff"].contains(&audience) {
        return Err(AppError::BadRequest("invalid audience".into()));
    }
    if audience == "class" && body.class_id.is_none() {
        return Err(AppError::BadRequest("class audience needs class_id".into()));
    }
    if auth.role == Role::Teacher && audience == "class" {
        if let Some(cid) = body.class_id {
            crate::classes::can_view_class(&state, &auth, cid).await?;
        }
    }
    let e = sqlx::query_as::<_, Event>(
        "INSERT INTO events (id, school_id, title, description, start_at, end_at, audience, class_id, type, created_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
         RETURNING id, school_id, title, description, start_at, end_at, audience, class_id, type, created_at",
    )
    .bind(Uuid::new_v4())
    .bind(auth.school()?)
    .bind(&body.title)
    .bind(&body.description)
    .bind(body.start_at)
    .bind(body.end_at)
    .bind(audience)
    .bind(body.class_id)
    .bind(body.etype.as_deref().unwrap_or("event"))
    .bind(auth.id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(e))
}

#[derive(Deserialize)]
pub struct EventsQuery {
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
}

async fn list(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<EventsQuery>,
) -> AppResult<Json<Vec<Event>>> {
    let from = q
        .from
        .unwrap_or_else(|| Utc::now() - chrono::Duration::days(30));
    let to =
        q.to.unwrap_or_else(|| Utc::now() + chrono::Duration::days(90));
    let rows = match auth.role {
        Role::Superadmin | Role::Admin | Role::Teacher => {
            sqlx::query_as::<_, Event>(
                "SELECT id, school_id, title, description, start_at, end_at, audience, class_id, type, created_at
                 FROM events WHERE school_id = $1 AND start_at >= $2 AND start_at < $3
                 ORDER BY start_at",
            )
            .bind(auth.school_id)
            .bind(from)
            .bind(to)
            .fetch_all(&state.db)
            .await?
        }
        Role::Student => {
            sqlx::query_as::<_, Event>(
                "SELECT e.id, e.school_id, e.title, e.description, e.start_at, e.end_at, e.audience, e.class_id, e.type, e.created_at
                 FROM events e
                 WHERE e.school_id = $1 AND e.start_at >= $2 AND e.start_at < $3
                   AND e.audience != 'staff' AND (
                    e.audience = 'school' OR EXISTS(
                        SELECT 1 FROM enrollments en
                        WHERE en.class_id = e.class_id AND en.student_id = $4))
                 ORDER BY e.start_at",
            )
            .bind(auth.school_id)
            .bind(from)
            .bind(to)
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Parent => {
            sqlx::query_as::<_, Event>(
                "SELECT e.id, e.school_id, e.title, e.description, e.start_at, e.end_at, e.audience, e.class_id, e.type, e.created_at
                 FROM events e
                 WHERE e.school_id = $1 AND e.start_at >= $2 AND e.start_at < $3
                   AND e.audience != 'staff' AND (
                    e.audience = 'school' OR EXISTS(
                        SELECT 1 FROM enrollments en
                        JOIN parent_links pl ON pl.student_id = en.student_id
                        WHERE en.class_id = e.class_id AND pl.parent_id = $4))
                 ORDER BY e.start_at",
            )
            .bind(auth.school_id)
            .bind(from)
            .bind(to)
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
    };
    Ok(Json(rows))
}

async fn update(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpsertEvent>,
) -> AppResult<Json<Event>> {
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    body.validate()?;
    let e = sqlx::query_as::<_, Event>(
        "UPDATE events SET title = $4, description = $5, start_at = $6, end_at = $7,
            audience = COALESCE($8, audience), class_id = $9, type = COALESCE($10, type)
         WHERE id = $1 AND school_id = $2 AND (created_by = $3 OR $11)
         RETURNING id, school_id, title, description, start_at, end_at, audience, class_id, type, created_at",
    )
    .bind(id)
    .bind(auth.school()?)
    .bind(auth.id)
    .bind(&body.title)
    .bind(&body.description)
    .bind(body.start_at)
    .bind(body.end_at)
    .bind(&body.audience)
    .bind(body.class_id)
    .bind(&body.etype)
    .bind(matches!(auth.role, Role::Admin | Role::Superadmin))
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(e))
}

async fn delete(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<axum::http::StatusCode> {
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    let res = sqlx::query(
        "DELETE FROM events WHERE id = $1 AND school_id = $2 AND (created_by = $3 OR $4)",
    )
    .bind(id)
    .bind(auth.school()?)
    .bind(auth.id)
    .bind(matches!(auth.role, Role::Admin | Role::Superadmin))
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
