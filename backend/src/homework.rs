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
pub struct Assignment {
    id: Uuid,
    school_id: Uuid,
    lesson_id: Uuid,
    title: String,
    description: Option<String>,
    due_date: NaiveDate,
    created_by: Uuid,
    created_at: DateTime<Utc>,
}

const ASSIGN_COLS: &str =
    "id, school_id, lesson_id, title, description, due_date, created_by, created_at";

#[derive(Serialize, sqlx::FromRow)]
pub struct Submission {
    id: Uuid,
    assignment_id: Uuid,
    student_id: Uuid,
    text: Option<String>,
    files: serde_json::Value,
    submitted_at: DateTime<Utc>,
    feedback: Option<String>,
}

#[derive(sqlx::FromRow)]
struct LessonScope {
    class_id: Uuid,
    teacher_id: Uuid,
}

/// Teacher assigned to the lesson (or admin) check. Returns the lesson's
/// class/subject/teacher for downstream use.
async fn lesson_scope(
    state: &AppState,
    auth: &AuthUser,
    lesson_id: Uuid,
) -> AppResult<LessonScope> {
    let l =
        sqlx::query_as::<_, LessonScope>("SELECT class_id, teacher_id FROM lessons WHERE id = $1")
            .bind(lesson_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or(AppError::NotFound)?;
    let allowed = match auth.role {
        Role::Superadmin | Role::Admin => true,
        Role::Teacher => l.teacher_id == auth.id,
        _ => false,
    };
    if allowed {
        Ok(l)
    } else {
        Err(AppError::Forbidden)
    }
}

#[derive(Deserialize, Validate)]
pub struct CreateAssignment {
    lesson_id: Uuid,
    #[validate(length(min = 1, max = 300))]
    title: String,
    description: Option<String>,
    due_date: NaiveDate,
}

async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateAssignment>,
) -> AppResult<Json<Assignment>> {
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    body.validate()?;
    let l = lesson_scope(&state, &auth, body.lesson_id).await?;
    let a = sqlx::query_as::<_, Assignment>(sqlx::AssertSqlSafe(format!(
        "INSERT INTO assignments (id, school_id, lesson_id, title, description, due_date, created_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING {ASSIGN_COLS}"
    )))
    .bind(Uuid::new_v4())
    .bind(auth.school()?)
    .bind(body.lesson_id)
    .bind(&body.title)
    .bind(&body.description)
    .bind(body.due_date)
    .bind(auth.id)
    .fetch_one(&state.db)
    .await?;
    crate::notifications::notify_class(
        &state,
        l.class_id,
        "homework",
        serde_json::json!({ "assignment_id": a.id, "title": a.title, "due_date": a.due_date }),
    )
    .await;
    Ok(Json(a))
}

#[derive(Deserialize)]
pub struct ListQuery {
    class_id: Option<Uuid>,
}

async fn list(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<Vec<Assignment>>> {
    let items = match auth.role {
        Role::Student => {
            sqlx::query_as::<_, Assignment>(sqlx::AssertSqlSafe(format!(
                "SELECT {ASSIGN_COLS} FROM assignments a
                 JOIN lessons l ON l.id = a.lesson_id
                 JOIN enrollments e ON e.class_id = l.class_id AND e.student_id = $1
                 ORDER BY a.due_date"
            )))
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Parent => {
            sqlx::query_as::<_, Assignment>(sqlx::AssertSqlSafe(format!(
                "SELECT {ASSIGN_COLS} FROM assignments a
                 JOIN lessons l ON l.id = a.lesson_id
                 JOIN enrollments e ON e.class_id = l.class_id
                 JOIN parent_links pl ON pl.student_id = e.student_id AND pl.parent_id = $1
                 ORDER BY a.due_date"
            )))
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Teacher => {
            sqlx::query_as::<_, Assignment>(sqlx::AssertSqlSafe(format!(
                "SELECT {ASSIGN_COLS} FROM assignments a
                 JOIN lessons l ON l.id = a.lesson_id
                 WHERE l.teacher_id = $1 OR a.created_by = $1
                 ORDER BY a.due_date"
            )))
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Admin | Role::Superadmin => {
            if let Some(cid) = q.class_id {
                sqlx::query_as::<_, Assignment>(sqlx::AssertSqlSafe(format!(
                    "SELECT {ASSIGN_COLS} FROM assignments a
                     JOIN lessons l ON l.id = a.lesson_id
                     WHERE l.class_id = $1 AND a.school_id = $2
                     ORDER BY a.due_date"
                )))
                .bind(cid)
                .bind(auth.school_id)
                .fetch_all(&state.db)
                .await?
            } else {
                sqlx::query_as::<_, Assignment>(sqlx::AssertSqlSafe(format!(
                    "SELECT {ASSIGN_COLS} FROM assignments a WHERE a.school_id = $1
                     ORDER BY a.due_date"
                )))
                .bind(auth.school()?)
                .fetch_all(&state.db)
                .await?
            }
        }
    };
    Ok(Json(items))
}

#[derive(Deserialize, Validate)]
pub struct UpdateAssignment {
    #[validate(length(min = 1, max = 300))]
    title: Option<String>,
    description: Option<String>,
    due_date: Option<NaiveDate>,
}

async fn update(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateAssignment>,
) -> AppResult<Json<Assignment>> {
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    body.validate()?;
    let lesson_id: Uuid =
        sqlx::query_scalar("SELECT lesson_id FROM assignments WHERE id = $1 AND school_id = $2")
            .bind(id)
            .bind(auth.school()?)
            .fetch_optional(&state.db)
            .await?
            .ok_or(AppError::NotFound)?;
    let l = lesson_scope(&state, &auth, lesson_id).await?;
    if auth.role == Role::Teacher && l.teacher_id != auth.id {
        return Err(AppError::Forbidden);
    }
    let a = sqlx::query_as::<_, Assignment>(sqlx::AssertSqlSafe(format!(
        "UPDATE assignments SET
            title = COALESCE($2, title),
            description = COALESCE($3, description),
            due_date = COALESCE($4, due_date)
         WHERE id = $1 RETURNING {ASSIGN_COLS}"
    )))
    .bind(id)
    .bind(&body.title)
    .bind(&body.description)
    .bind(body.due_date)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(a))
}

async fn delete(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<axum::http::StatusCode> {
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    let lesson_id: Uuid =
        sqlx::query_scalar("SELECT lesson_id FROM assignments WHERE id = $1 AND school_id = $2")
            .bind(id)
            .bind(auth.school()?)
            .fetch_optional(&state.db)
            .await?
            .ok_or(AppError::NotFound)?;
    let l = lesson_scope(&state, &auth, lesson_id).await?;
    if auth.role == Role::Teacher && l.teacher_id != auth.id {
        return Err(AppError::Forbidden);
    }
    sqlx::query("DELETE FROM assignments WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct SubmitWork {
    text: Option<String>,
    files: Option<serde_json::Value>,
}

async fn submit(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<SubmitWork>,
) -> AppResult<Json<Submission>> {
    auth.require(&[Role::Student])?;
    let enrolled: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM assignments a
            JOIN lessons l ON l.id = a.lesson_id
            JOIN enrollments e ON e.class_id = l.class_id
            WHERE a.id = $1 AND e.student_id = $2)",
    )
    .bind(id)
    .bind(auth.id)
    .fetch_one(&state.db)
    .await?;
    if !enrolled {
        return Err(AppError::Forbidden);
    }
    let s = sqlx::query_as::<_, Submission>(
        "INSERT INTO submissions (id, assignment_id, student_id, text, files)
         VALUES ($1,$2,$3,$4,COALESCE($5,'[]'))
         ON CONFLICT (assignment_id, student_id)
         DO UPDATE SET text = EXCLUDED.text, files = EXCLUDED.files, submitted_at = now()
         RETURNING id, assignment_id, student_id, text, files, submitted_at, feedback",
    )
    .bind(Uuid::new_v4())
    .bind(id)
    .bind(auth.id)
    .bind(&body.text)
    .bind(&body.files)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(s))
}

async fn list_submissions(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Vec<Submission>>> {
    let lesson_id: Uuid =
        sqlx::query_scalar("SELECT lesson_id FROM assignments WHERE id = $1 AND school_id = $2")
            .bind(id)
            .bind(auth.school()?)
            .fetch_optional(&state.db)
            .await?
            .ok_or(AppError::NotFound)?;
    match auth.role {
        Role::Admin | Role::Superadmin => {}
        Role::Teacher => {
            let l = lesson_scope(&state, &auth, lesson_id).await?;
            if l.teacher_id != auth.id {
                return Err(AppError::Forbidden);
            }
        }
        _ => return Err(AppError::Forbidden),
    }
    let subs = sqlx::query_as::<_, Submission>(
        "SELECT id, assignment_id, student_id, text, files, submitted_at, feedback
         FROM submissions WHERE assignment_id = $1 ORDER BY submitted_at",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(subs))
}

#[derive(Deserialize)]
pub struct GradeSubmission {
    feedback: Option<String>,
    /// Optional grade — creates a journal grade row of type 'homework'.
    grade_value: Option<f64>,
}

async fn grade_submission(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<GradeSubmission>,
) -> AppResult<axum::http::StatusCode> {
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    let row: Option<(Uuid, Uuid, Uuid, Uuid)> = sqlx::query_as(
        "SELECT s.assignment_id, s.student_id, l.id, l.subject_id
         FROM submissions s
         JOIN assignments a ON a.id = s.assignment_id
         JOIN lessons l ON l.id = a.lesson_id
         WHERE s.id = $1 AND a.school_id = $2",
    )
    .bind(id)
    .bind(auth.school()?)
    .fetch_optional(&state.db)
    .await?;
    let (_assignment_id, student_id, lesson_id, subject_id) = row.ok_or(AppError::NotFound)?;
    let l = lesson_scope(&state, &auth, lesson_id).await?;
    if auth.role == Role::Teacher && l.teacher_id != auth.id {
        return Err(AppError::Forbidden);
    }

    if let Some(fb) = &body.feedback {
        sqlx::query("UPDATE submissions SET feedback = $2 WHERE id = $1")
            .bind(id)
            .bind(fb)
            .execute(&state.db)
            .await?;
    }
    if let Some(v) = body.grade_value {
        crate::journal::validate_grade(&state, auth.school()?, v).await?;
        sqlx::query(
            "INSERT INTO grades (id, school_id, lesson_id, student_id, subject_id, teacher_id, value, type, date)
             VALUES ($1,$2,$3,$4,$5,$6,$7,'homework', CURRENT_DATE)
             ON CONFLICT (lesson_id, student_id, type) DO UPDATE SET value = EXCLUDED.value",
        )
        .bind(Uuid::new_v4())
        .bind(auth.school()?)
        .bind(lesson_id)
        .bind(student_id)
        .bind(subject_id)
        .bind(auth.id)
        .bind(v)
        .execute(&state.db)
        .await?;
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", axum::routing::patch(update).delete(delete))
        .route("/{id}/submit", axum::routing::post(submit))
        .route("/{id}/submissions", get(list_submissions))
        .route(
            "/submissions/{id}/grade",
            axum::routing::patch(grade_submission),
        )
}
