use axum::extract::{Path, State};
use axum::routing::{get, post};
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
pub struct Poll {
    id: Uuid,
    question: String,
    options: serde_json::Value,
    audience: String,
    class_id: Option<Uuid>,
    ends_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

#[derive(Deserialize, Validate)]
pub struct CreatePoll {
    #[validate(length(min = 1, max = 500))]
    question: String,
    options: Vec<String>,
    /// 'school' | 'parents' | 'teachers' | 'class'
    audience: Option<String>,
    class_id: Option<Uuid>,
    ends_at: Option<DateTime<Utc>>,
}

async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreatePoll>,
) -> AppResult<Json<Poll>> {
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    body.validate()?;
    if body.options.len() < 2 {
        return Err(AppError::BadRequest("poll needs at least 2 options".into()));
    }
    let audience = body.audience.as_deref().unwrap_or("school");
    if !["school", "parents", "teachers", "class"].contains(&audience) {
        return Err(AppError::BadRequest("invalid audience".into()));
    }
    if audience == "class" && body.class_id.is_none() {
        return Err(AppError::BadRequest("class audience needs class_id".into()));
    }
    let p = sqlx::query_as::<_, Poll>(
        "INSERT INTO polls (id, school_id, question, options, audience, class_id, ends_at, created_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
         RETURNING id, question, options, audience, class_id, ends_at, created_at",
    )
    .bind(Uuid::new_v4())
    .bind(auth.school()?)
    .bind(&body.question)
    .bind(serde_json::json!(body.options))
    .bind(audience)
    .bind(body.class_id)
    .bind(body.ends_at)
    .bind(auth.id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(p))
}

fn can_see(auth: &AuthUser, audience: &str, class_member: bool) -> bool {
    match audience {
        "school" => true,
        "parents" => matches!(auth.role, Role::Parent | Role::Admin | Role::Superadmin),
        "teachers" => matches!(auth.role, Role::Teacher | Role::Admin | Role::Superadmin),
        "class" => class_member || matches!(auth.role, Role::Admin | Role::Superadmin),
        _ => false,
    }
}

async fn class_member_of(state: &AppState, auth: &AuthUser, class_id: Option<Uuid>) -> bool {
    let Some(cid) = class_id else { return false };
    match auth.role {
        Role::Student => sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM enrollments WHERE class_id=$1 AND student_id=$2)",
        )
        .bind(cid)
        .bind(auth.id)
        .fetch_one(&state.db)
        .await
        .unwrap_or(false),
        Role::Parent => sqlx::query_scalar(
            "SELECT EXISTS(
                    SELECT 1 FROM enrollments e
                    JOIN parent_links pl ON pl.student_id = e.student_id
                    WHERE e.class_id = $1 AND pl.parent_id = $2)",
        )
        .bind(cid)
        .bind(auth.id)
        .fetch_one(&state.db)
        .await
        .unwrap_or(false),
        Role::Teacher => sqlx::query_scalar(
            "SELECT EXISTS(
                    SELECT 1 FROM class_subject_teachers WHERE class_id=$1 AND teacher_id=$2
                    UNION SELECT 1 FROM classes WHERE id=$1 AND homeroom_teacher_id=$2)",
        )
        .bind(cid)
        .bind(auth.id)
        .fetch_one(&state.db)
        .await
        .unwrap_or(false),
        _ => false,
    }
}

async fn list(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Vec<Poll>>> {
    let all = sqlx::query_as::<_, Poll>(
        "SELECT id, question, options, audience, class_id, ends_at, created_at
         FROM polls WHERE school_id = $1 ORDER BY created_at DESC LIMIT 100",
    )
    .bind(auth.school()?)
    .fetch_all(&state.db)
    .await?;
    let mut visible = Vec::new();
    for p in all {
        let member = class_member_of(&state, &auth, p.class_id).await;
        if can_see(&auth, &p.audience, member) {
            visible.push(p);
        }
    }
    Ok(Json(visible))
}

#[derive(Deserialize)]
pub struct Vote {
    option_index: usize,
}

async fn vote(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<Vote>,
) -> AppResult<axum::http::StatusCode> {
    let p = sqlx::query_as::<_, Poll>(
        "SELECT id, question, options, audience, class_id, ends_at, created_at
         FROM polls WHERE id = $1 AND school_id = $2",
    )
    .bind(id)
    .bind(auth.school()?)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;

    let member = class_member_of(&state, &auth, p.class_id).await;
    if !can_see(&auth, &p.audience, member) {
        return Err(AppError::Forbidden);
    }
    if let Some(end) = p.ends_at {
        if end < Utc::now() {
            return Err(AppError::BadRequest("poll has ended".into()));
        }
    }
    let n_opts = p.options.as_array().map(|a| a.len()).unwrap_or(0);
    if body.option_index >= n_opts {
        return Err(AppError::BadRequest("invalid option".into()));
    }
    sqlx::query(
        "INSERT INTO poll_votes (poll_id, user_id, option_index) VALUES ($1,$2,$3)
         ON CONFLICT (poll_id, user_id) DO UPDATE SET option_index = EXCLUDED.option_index",
    )
    .bind(id)
    .bind(auth.id)
    .bind(body.option_index as i32)
    .execute(&state.db)
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn results(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let p = sqlx::query_as::<_, Poll>(
        "SELECT id, question, options, audience, class_id, ends_at, created_at
         FROM polls WHERE id = $1 AND school_id = $2",
    )
    .bind(id)
    .bind(auth.school()?)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    let member = class_member_of(&state, &auth, p.class_id).await;
    if !can_see(&auth, &p.audience, member) {
        return Err(AppError::Forbidden);
    }
    let counts: Vec<(i32, i64)> = sqlx::query_as(
        "SELECT option_index, COUNT(*) FROM poll_votes WHERE poll_id = $1 GROUP BY option_index",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let mut result = serde_json::Map::new();
    let options = p.options.as_array().cloned().unwrap_or_default();
    for (i, opt) in options.iter().enumerate() {
        let n = counts
            .iter()
            .find(|(idx, _)| *idx == i as i32)
            .map(|(_, c)| *c)
            .unwrap_or(0);
        result.insert(opt.as_str().unwrap_or(&i.to_string()).to_string(), n.into());
    }
    let mine: Option<i32> = sqlx::query_scalar(
        "SELECT option_index FROM poll_votes WHERE poll_id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(auth.id)
    .fetch_optional(&state.db)
    .await?;
    Ok(Json(serde_json::json!({
        "poll": p,
        "results": result,
        "total_votes": counts.iter().map(|(_, c)| c).sum::<i64>(),
        "my_vote": mine,
    })))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}/vote", post(vote))
        .route("/{id}/results", get(results))
}
