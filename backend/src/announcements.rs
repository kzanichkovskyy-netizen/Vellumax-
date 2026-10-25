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
pub struct Announcement {
    id: Uuid,
    school_id: Uuid,
    author_id: Uuid,
    title: String,
    body: String,
    audience: String,
    class_id: Option<Uuid>,
    role: Option<String>,
    published_at: DateTime<Utc>,
}

#[derive(Deserialize, Validate)]
pub struct CreateAnnouncement {
    #[validate(length(min = 1, max = 300))]
    title: String,
    #[validate(length(min = 1))]
    body: String,
    /// 'school' (default), 'class' (requires class_id), 'role' (requires role)
    audience: Option<String>,
    class_id: Option<Uuid>,
    role: Option<String>,
}

#[derive(Deserialize, Validate)]
pub struct UpdateAnnouncement {
    #[validate(length(min = 1, max = 300))]
    title: Option<String>,
    body: Option<String>,
}

async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateAnnouncement>,
) -> AppResult<Json<Announcement>> {
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    body.validate()?;

    let audience = body.audience.as_deref().unwrap_or("school");
    match (audience, body.class_id, body.role.as_deref()) {
        ("school", _, _) if body.class_id.is_some() || body.role.is_some() => {
            return Err(AppError::BadRequest(
                "audience 'school' takes no class_id or role".into(),
            ));
        }
        ("class", Some(_), None) => {}
        ("role", None, Some(r)) if ["admin", "teacher", "student", "parent"].contains(&r) => {}
        ("school", None, None) => {}
        _ => {
            return Err(AppError::BadRequest(
                "audience 'class' needs class_id; 'role' needs role".into(),
            ));
        }
    }
    // teachers may only post school-wide or to classes they teach — keep it simple:
    // teachers post any announcement; admins own moderation.

    let a = sqlx::query_as::<_, Announcement>(
        "INSERT INTO announcements (id, school_id, author_id, title, body, audience, class_id, role)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
         RETURNING id, school_id, author_id, title, body, audience, class_id, role, published_at",
    )
    .bind(Uuid::new_v4())
    .bind(auth.school()?)
    .bind(auth.id)
    .bind(&body.title)
    .bind(&body.body)
    .bind(audience)
    .bind(body.class_id)
    .bind(&body.role)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(a))
}

async fn list(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Vec<Announcement>>> {
    let rows = match auth.role {
        Role::Superadmin => {
            sqlx::query_as::<_, Announcement>(
                "SELECT id, school_id, author_id, title, body, audience, class_id, role, published_at
                 FROM announcements ORDER BY published_at DESC LIMIT 100",
            )
            .fetch_all(&state.db)
            .await?
        }
        Role::Admin | Role::Teacher => {
            sqlx::query_as::<_, Announcement>(
                "SELECT id, school_id, author_id, title, body, audience, class_id, role, published_at
                 FROM announcements WHERE school_id = $1 ORDER BY published_at DESC LIMIT 100",
            )
            .bind(auth.school_id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Student => {
            sqlx::query_as::<_, Announcement>(
                "SELECT a.id, a.school_id, a.author_id, a.title, a.body, a.audience, a.class_id, a.role, a.published_at
                 FROM announcements a
                 WHERE a.school_id = $1 AND (
                    a.audience = 'school'
                    OR (a.audience = 'role' AND a.role = 'student')
                    OR (a.audience = 'class' AND EXISTS(
                        SELECT 1 FROM enrollments e WHERE e.class_id = a.class_id AND e.student_id = $2))
                 ) ORDER BY a.published_at DESC LIMIT 100",
            )
            .bind(auth.school_id)
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Parent => {
            sqlx::query_as::<_, Announcement>(
                "SELECT a.id, a.school_id, a.author_id, a.title, a.body, a.audience, a.class_id, a.role, a.published_at
                 FROM announcements a
                 WHERE a.school_id = $1 AND (
                    a.audience = 'school'
                    OR (a.audience = 'role' AND a.role = 'parent')
                    OR (a.audience = 'class' AND EXISTS(
                        SELECT 1 FROM enrollments e
                        JOIN parent_links pl ON pl.student_id = e.student_id
                        WHERE e.class_id = a.class_id AND pl.parent_id = $2))
                 ) ORDER BY a.published_at DESC LIMIT 100",
            )
            .bind(auth.school_id)
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
    Json(body): Json<UpdateAnnouncement>,
) -> AppResult<Json<Announcement>> {
    body.validate()?;
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    let a = sqlx::query_as::<_, Announcement>(
        "UPDATE announcements SET title = COALESCE($4, title), body = COALESCE($5, body)
         WHERE id = $1 AND school_id = $2 AND (author_id = $3 OR $6)
         RETURNING id, school_id, author_id, title, body, audience, class_id, role, published_at",
    )
    .bind(id)
    .bind(auth.school()?)
    .bind(auth.id)
    .bind(&body.title)
    .bind(&body.body)
    .bind(matches!(auth.role, Role::Admin | Role::Superadmin))
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(a))
}

async fn delete(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<axum::http::StatusCode> {
    let res = sqlx::query(
        "DELETE FROM announcements WHERE id = $1 AND school_id = $2 AND (author_id = $3 OR $4)",
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
