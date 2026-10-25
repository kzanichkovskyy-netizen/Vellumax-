use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use validator::Validate;

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::types::Role;

#[derive(Serialize, sqlx::FromRow)]
pub struct Task {
    id: Uuid,
    creator_id: Uuid,
    assignee_id: Option<Uuid>,
    title: String,
    description: Option<String>,
    due_date: Option<NaiveDate>,
    status: String,
    done_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

const TASK_COLS: &str =
    "id, creator_id, assignee_id, title, description, due_date, status, done_at, created_at";

#[derive(Deserialize, Validate)]
pub struct CreateTask {
    assignee_id: Option<Uuid>,
    #[validate(length(min = 1, max = 300))]
    title: String,
    description: Option<String>,
    due_date: Option<NaiveDate>,
}

async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateTask>,
) -> AppResult<Json<Task>> {
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    body.validate()?;
    if let Some(aid) = body.assignee_id {
        let ok: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM users WHERE id = $1 AND school_id = $2)",
        )
        .bind(aid)
        .bind(auth.school()?)
        .fetch_one(&state.db)
        .await?;
        if !ok {
            return Err(AppError::BadRequest("assignee not in your school".into()));
        }
    }
    let t = sqlx::query_as::<_, Task>(sqlx::AssertSqlSafe(format!(
        "INSERT INTO tasks (id, school_id, creator_id, assignee_id, title, description, due_date)
         VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING {TASK_COLS}"
    )))
    .bind(Uuid::new_v4())
    .bind(auth.school()?)
    .bind(auth.id)
    .bind(body.assignee_id)
    .bind(&body.title)
    .bind(&body.description)
    .bind(body.due_date)
    .fetch_one(&state.db)
    .await?;
    if let Some(aid) = body.assignee_id {
        if aid != auth.id {
            crate::notifications::notify(
                &state,
                aid,
                "task",
                serde_json::json!({ "task_id": t.id, "title": t.title, "from": auth.id }),
            )
            .await;
        }
    }
    Ok(Json(t))
}

#[derive(Deserialize)]
pub struct TaskQuery {
    status: Option<String>,
    mine: Option<bool>,
}

async fn list(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<TaskQuery>,
) -> AppResult<Json<Vec<Task>>> {
    let rows = match auth.role {
        Role::Superadmin | Role::Admin => {
            if q.mine == Some(true) {
                sqlx::query_as::<_, Task>(sqlx::AssertSqlSafe(format!(
                    "SELECT {TASK_COLS} FROM tasks WHERE school_id = $1 AND assignee_id = $2
                     AND ($3::text IS NULL OR status = $3) ORDER BY created_at DESC LIMIT 200"
                )))
                .bind(auth.school_id)
                .bind(auth.id)
                .bind(&q.status)
                .fetch_all(&state.db)
                .await?
            } else {
                sqlx::query_as::<_, Task>(sqlx::AssertSqlSafe(format!(
                    "SELECT {TASK_COLS} FROM tasks WHERE school_id = $1
                     AND ($2::text IS NULL OR status = $2) ORDER BY created_at DESC LIMIT 200"
                )))
                .bind(auth.school_id)
                .bind(&q.status)
                .fetch_all(&state.db)
                .await?
            }
        }
        _ => {
            // staff and students see tasks assigned to them or created by them
            sqlx::query_as::<_, Task>(sqlx::AssertSqlSafe(format!(
                "SELECT {TASK_COLS} FROM tasks WHERE school_id = $1
                   AND (assignee_id = $2 OR creator_id = $2)
                   AND ($3::text IS NULL OR status = $3) ORDER BY created_at DESC LIMIT 200"
            )))
            .bind(auth.school()?)
            .bind(auth.id)
            .bind(&q.status)
            .fetch_all(&state.db)
            .await?
        }
    };
    Ok(Json(rows))
}

#[derive(Deserialize)]
pub struct UpdateTask {
    status: Option<String>,
    title: Option<String>,
    description: Option<String>,
    due_date: Option<NaiveDate>,
}

async fn update(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateTask>,
) -> AppResult<Json<Task>> {
    if let Some(s) = &body.status {
        if !["open", "done", "cancelled"].contains(&s.as_str()) {
            return Err(AppError::BadRequest("invalid status".into()));
        }
    }
    let t = sqlx::query_as::<_, Task>(sqlx::AssertSqlSafe(format!(
        "UPDATE tasks SET
            status = COALESCE($4, status),
            done_at = CASE WHEN $4 = 'done' THEN now() WHEN $4 = 'open' THEN NULL ELSE done_at END,
            title = COALESCE($5, title),
            description = COALESCE($6, description),
            due_date = COALESCE($7, due_date)
         WHERE id = $1 AND school_id = $2
           AND (assignee_id = $3 OR creator_id = $3 OR $8)
         RETURNING {TASK_COLS}"
    )))
    .bind(id)
    .bind(auth.school()?)
    .bind(auth.id)
    .bind(&body.status)
    .bind(&body.title)
    .bind(&body.description)
    .bind(body.due_date)
    .bind(matches!(auth.role, Role::Admin | Role::Superadmin))
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(t))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", axum::routing::patch(update))
}
