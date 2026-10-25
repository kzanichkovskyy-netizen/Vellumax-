use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use validator::Validate;

use crate::AppState;
use crate::auth::AuthUser;
use crate::classes::can_view_class;
use crate::error::{AppError, AppResult};
use crate::types::Role;

#[derive(Serialize, sqlx::FromRow)]
pub struct Quiz {
    id: Uuid,
    school_id: Uuid,
    class_id: Option<Uuid>,
    subject_id: Option<Uuid>,
    title: String,
    description: Option<String>,
    published: bool,
    created_by: Uuid,
    created_at: DateTime<Utc>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Question {
    id: Uuid,
    text: String,
    #[serde(rename = "type")]
    #[sqlx(rename = "type")]
    qtype: String,
    options: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    correct: Option<serde_json::Value>,
    points: i32,
    position: i32,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Attempt {
    id: Uuid,
    quiz_id: Uuid,
    student_id: Uuid,
    answers: serde_json::Value,
    score: Option<f64>,
    max_score: Option<f64>,
    submitted_at: DateTime<Utc>,
}

#[derive(Deserialize, Validate)]
pub struct CreateQuiz {
    class_id: Option<Uuid>,
    subject_id: Option<Uuid>,
    #[validate(length(min = 1, max = 300))]
    title: String,
    description: Option<String>,
    questions: Vec<CreateQuestion>,
}

#[derive(Deserialize, Validate)]
pub struct CreateQuestion {
    #[validate(length(min = 1))]
    text: String,
    /// 'single' | 'multiple' | 'text'
    #[serde(rename = "type")]
    qtype: String,
    /// option texts for single/multiple
    #[serde(default)]
    options: Vec<String>,
    /// correct option indices (single/multiple) or accepted answers (text)
    #[serde(default)]
    correct: serde_json::Value,
    #[serde(default = "one")]
    points: i32,
}

fn one() -> i32 {
    1
}

async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateQuiz>,
) -> AppResult<Json<Quiz>> {
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    body.validate()?;
    if let Some(cid) = body.class_id {
        can_view_class(&state, &auth, cid).await?;
    }
    for q in &body.questions {
        if !["single", "multiple", "text"].contains(&q.qtype.as_str()) {
            return Err(AppError::BadRequest(format!(
                "invalid question type {}",
                q.qtype
            )));
        }
        if q.qtype != "text" && q.options.is_empty() {
            return Err(AppError::BadRequest("choice questions need options".into()));
        }
    }

    let mut tx = state.db.begin().await?;
    let quiz = sqlx::query_as::<_, Quiz>(
        "INSERT INTO quizzes (id, school_id, class_id, subject_id, title, description, created_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7)
         RETURNING id, school_id, class_id, subject_id, title, description, published, created_by, created_at",
    )
    .bind(Uuid::new_v4())
    .bind(auth.school()?)
    .bind(body.class_id)
    .bind(body.subject_id)
    .bind(&body.title)
    .bind(&body.description)
    .bind(auth.id)
    .fetch_one(&mut *tx)
    .await?;

    for (i, q) in body.questions.iter().enumerate() {
        sqlx::query(
            "INSERT INTO quiz_questions (id, quiz_id, text, type, options, correct, points, position)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(Uuid::new_v4())
        .bind(quiz.id)
        .bind(&q.text)
        .bind(&q.qtype)
        .bind(serde_json::json!(q.options))
        .bind(&q.correct)
        .bind(q.points)
        .bind(i as i32)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Json(quiz))
}

async fn publish(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Quiz>> {
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    let quiz = sqlx::query_as::<_, Quiz>(
        "UPDATE quizzes SET published = TRUE
         WHERE id = $1 AND school_id = $2 AND (created_by = $3 OR $4)
         RETURNING id, school_id, class_id, subject_id, title, description, published, created_by, created_at",
    )
    .bind(id)
    .bind(auth.school()?)
    .bind(auth.id)
    .bind(matches!(auth.role, Role::Admin | Role::Superadmin))
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    if let Some(cid) = quiz.class_id {
        crate::notifications::notify_class(
            &state,
            cid,
            "quiz",
            serde_json::json!({ "quiz_id": quiz.id, "title": quiz.title }),
        )
        .await;
    }
    Ok(Json(quiz))
}

async fn list(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Vec<Quiz>>> {
    let rows = match auth.role {
        Role::Superadmin | Role::Admin => {
            sqlx::query_as::<_, Quiz>(
                "SELECT id, school_id, class_id, subject_id, title, description, published, created_by, created_at
                 FROM quizzes WHERE school_id = $1 ORDER BY created_at DESC",
            )
            .bind(auth.school_id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Teacher => {
            sqlx::query_as::<_, Quiz>(
                "SELECT id, school_id, class_id, subject_id, title, description, published, created_by, created_at
                 FROM quizzes WHERE school_id = $1 AND (created_by = $2 OR class_id IN (
                    SELECT class_id FROM class_subject_teachers WHERE teacher_id = $2))
                 ORDER BY created_at DESC",
            )
            .bind(auth.school_id)
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Student => {
            sqlx::query_as::<_, Quiz>(
                "SELECT q.id, q.school_id, q.class_id, q.subject_id, q.title, q.description, q.published, q.created_by, q.created_at
                 FROM quizzes q
                 WHERE q.published AND (
                    q.class_id IS NULL AND q.school_id = $1
                    OR EXISTS(SELECT 1 FROM enrollments e WHERE e.class_id = q.class_id AND e.student_id = $2))
                 ORDER BY q.created_at DESC",
            )
            .bind(auth.school_id)
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Parent => {
            sqlx::query_as::<_, Quiz>(
                "SELECT q.id, q.school_id, q.class_id, q.subject_id, q.title, q.description, q.published, q.created_by, q.created_at
                 FROM quizzes q
                 WHERE q.published AND q.school_id = $1 AND EXISTS(
                    SELECT 1 FROM enrollments e
                    JOIN parent_links pl ON pl.student_id = e.student_id
                    WHERE e.class_id = q.class_id AND pl.parent_id = $2)
                 ORDER BY q.created_at DESC",
            )
            .bind(auth.school_id)
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
    };
    Ok(Json(rows))
}

/// Quiz with questions. Students get it without correct answers.
async fn get_one(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let quiz = sqlx::query_as::<_, Quiz>(
        "SELECT id, school_id, class_id, subject_id, title, description, published, created_by, created_at
         FROM quizzes WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;

    let privileged = matches!(auth.role, Role::Superadmin | Role::Admin | Role::Teacher);
    if !privileged {
        // students see only published quizzes they're in scope of
        let visible = match auth.role {
            Role::Student if quiz.published => match quiz.class_id {
                None => auth.school_id == Some(quiz.school_id),
                Some(cid) => sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM enrollments WHERE class_id=$1 AND student_id=$2)",
                )
                .bind(cid)
                .bind(auth.id)
                .fetch_one(&state.db)
                .await?,
            },
            Role::Parent if quiz.published => match quiz.class_id {
                None => auth.school_id == Some(quiz.school_id),
                Some(cid) => {
                    sqlx::query_scalar(
                        "SELECT EXISTS(
                            SELECT 1 FROM enrollments e
                            JOIN parent_links pl ON pl.student_id = e.student_id
                            WHERE e.class_id = $1 AND pl.parent_id = $2)",
                    )
                    .bind(cid)
                    .bind(auth.id)
                    .fetch_one(&state.db)
                    .await?
                }
            },
            _ => false,
        };
        if !visible {
            return Err(AppError::Forbidden);
        }
    }

    let include_correct = privileged;
    let questions = if include_correct {
        sqlx::query_as::<_, Question>(
            "SELECT id, text, type, options, correct, points, position
             FROM quiz_questions WHERE quiz_id = $1 ORDER BY position",
        )
        .bind(id)
        .fetch_all(&state.db)
        .await?
    } else {
        sqlx::query_as::<_, Question>(
            "SELECT id, text, type, options, NULL AS correct, points, position
             FROM quiz_questions WHERE quiz_id = $1 ORDER BY position",
        )
        .bind(id)
        .fetch_all(&state.db)
        .await?
    };

    Ok(Json(serde_json::json!({
        "quiz": quiz,
        "questions": questions,
    })))
}

#[derive(Deserialize)]
pub struct SubmitAttempt {
    /// { "<question_id>": answer } — index for single, [indices] for multiple, string for text
    answers: serde_json::Value,
}

async fn submit(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<SubmitAttempt>,
) -> AppResult<Json<serde_json::Value>> {
    auth.require(&[Role::Student])?;
    let quiz = sqlx::query_as::<_, Quiz>(
        "SELECT id, school_id, class_id, subject_id, title, description, published, created_by, created_at
         FROM quizzes WHERE id = $1 AND published",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    if let Some(cid) = quiz.class_id {
        can_view_class(&state, &auth, cid).await?;
    }

    #[derive(sqlx::FromRow)]
    struct QRow {
        id: Uuid,
        qtype: String,
        correct: serde_json::Value,
        points: i32,
    }
    let questions = sqlx::query_as::<_, QRow>(
        "SELECT id, type AS qtype, correct, points FROM quiz_questions WHERE quiz_id = $1",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;

    let mut score = 0.0f64;
    let mut max = 0.0f64;
    for q in &questions {
        max += q.points as f64;
        let ans = &body.answers[q.id.to_string()];
        let right = match q.qtype.as_str() {
            "single" => q.correct.get(0) == Some(ans),
            "multiple" => {
                let mut got: Vec<i64> = serde_json::from_value(ans.clone()).unwrap_or_default();
                let mut want: Vec<i64> =
                    serde_json::from_value(q.correct.clone()).unwrap_or_default();
                got.sort_unstable();
                want.sort_unstable();
                !want.is_empty() && got == want
            }
            _ => {
                // text: exact (case-insensitive, trimmed) match against accepted answers
                let given = ans.as_str().unwrap_or("").trim().to_lowercase();
                !given.is_empty()
                    && q.correct
                        .as_array()
                        .map(|a| {
                            a.iter().any(|v| {
                                v.as_str()
                                    .map(|s| s.trim().to_lowercase() == given)
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false)
            }
        };
        if right {
            score += q.points as f64;
        }
    }

    let attempt = sqlx::query_as::<_, Attempt>(
        "INSERT INTO quiz_attempts (id, quiz_id, student_id, answers, score, max_score)
         VALUES ($1,$2,$3,$4,$5,$6)
         ON CONFLICT (quiz_id, student_id)
         DO UPDATE SET answers = EXCLUDED.answers, score = EXCLUDED.score,
                       max_score = EXCLUDED.max_score, submitted_at = now()
         RETURNING id, quiz_id, student_id, answers, score::float8, max_score::float8, submitted_at",
    )
    .bind(Uuid::new_v4())
    .bind(quiz.id)
    .bind(auth.id)
    .bind(&body.answers)
    .bind(score)
    .bind(max)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(serde_json::json!({
        "attempt": attempt,
        "score": score,
        "max_score": max,
    })))
}

async fn attempts(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Vec<Attempt>>> {
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    let rows = sqlx::query_as::<_, Attempt>(
        "SELECT id, quiz_id, student_id, answers, score::float8, max_score::float8, submitted_at
         FROM quiz_attempts WHERE quiz_id = $1 ORDER BY submitted_at",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(get_one))
        .route("/{id}/publish", post(publish))
        .route("/{id}/submit", post(submit))
        .route("/{id}/attempts", get(attempts))
}
