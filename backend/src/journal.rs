use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::AppState;
use crate::auth::AuthUser;
use crate::classes::can_view_class;
use crate::error::{AppError, AppResult};
use crate::types::Role;
use crate::users::UserView;

/// Validate a grade value against the school's configured scale
/// (`schools.settings.grading { min, max }`, defaults 1..12).
pub(crate) async fn validate_grade(
    state: &AppState,
    school_id: Uuid,
    value: f64,
) -> AppResult<()> {
    let settings: serde_json::Value =
        sqlx::query_scalar("SELECT settings FROM schools WHERE id = $1")
            .bind(school_id)
            .fetch_one(&state.db)
            .await?;
    let g = &settings["grading"];
    let min = g["min"].as_f64().unwrap_or(1.0);
    let max = g["max"].as_f64().unwrap_or(12.0);
    if value < min || value > max {
        return Err(AppError::BadRequest(format!(
            "grade must be between {min} and {max}"
        )));
    }
    Ok(())
}

/// Caller must be admin or the teacher of the given lesson.
async fn can_grade_lesson(state: &AppState, auth: &AuthUser, lesson_id: Uuid) -> AppResult<Uuid> {
    let row: Option<(Uuid, Uuid)> =
        sqlx::query_as("SELECT school_id, teacher_id FROM lessons WHERE id = $1")
            .bind(lesson_id)
            .fetch_optional(&state.db)
            .await?;
    let (school_id, teacher_id) = row.ok_or(AppError::NotFound)?;
    let ok = match auth.role {
        Role::Superadmin => true,
        Role::Admin => auth.school_id == Some(school_id),
        Role::Teacher => teacher_id == auth.id,
        _ => false,
    };
    if ok {
        Ok(school_id)
    } else {
        Err(AppError::Forbidden)
    }
}

/// Whether the caller may view a student's journal data.
async fn can_view_student(state: &AppState, auth: &AuthUser, student_id: Uuid) -> AppResult<()> {
    let ok = match auth.role {
        Role::Superadmin | Role::Admin => true,
        Role::Student => auth.id == student_id,
        Role::Parent => {
            sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM parent_links WHERE parent_id=$1 AND student_id=$2)",
            )
            .bind(auth.id)
            .bind(student_id)
            .fetch_one(&state.db)
            .await?
        }
        Role::Teacher => {
            sqlx::query_scalar(
                "SELECT EXISTS(
                    SELECT 1 FROM enrollments e
                    JOIN class_subject_teachers cst ON cst.class_id = e.class_id
                    WHERE e.student_id = $1 AND cst.teacher_id = $2)",
            )
            .bind(student_id)
            .bind(auth.id)
            .fetch_one(&state.db)
            .await?
        }
    };
    if ok {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Grade {
    id: Uuid,
    lesson_id: Option<Uuid>,
    student_id: Uuid,
    subject_id: Uuid,
    teacher_id: Uuid,
    value: f64,
    #[sqlx(rename = "type")]
    grade_type: String,
    date: NaiveDate,
    comment: Option<String>,
}

#[derive(Deserialize)]
pub struct GradeEntry {
    student_id: Uuid,
    value: f64,
    #[serde(default = "default_grade_type")]
    #[serde(rename = "type")]
    grade_type: String,
    comment: Option<String>,
}

fn default_grade_type() -> String {
    "classwork".into()
}

#[derive(Deserialize)]
pub struct BulkGrades {
    entries: Vec<GradeEntry>,
}

async fn bulk_grades(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(lesson_id): Path<Uuid>,
    Json(body): Json<BulkGrades>,
) -> AppResult<axum::http::StatusCode> {
    let school_id = can_grade_lesson(&state, &auth, lesson_id).await?;
    let subject_id: Uuid =
        sqlx::query_scalar("SELECT subject_id FROM lessons WHERE id = $1")
            .bind(lesson_id)
            .fetch_one(&state.db)
            .await?;
    for e in &body.entries {
        validate_grade(&state, school_id, e.value).await?;
        sqlx::query(
            "INSERT INTO grades (id, school_id, lesson_id, student_id, subject_id, teacher_id, value, type, date, comment)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8, CURRENT_DATE, $9)
             ON CONFLICT (lesson_id, student_id, type)
             DO UPDATE SET value = EXCLUDED.value, comment = EXCLUDED.comment",
        )
        .bind(Uuid::new_v4())
        .bind(school_id)
        .bind(lesson_id)
        .bind(e.student_id)
        .bind(subject_id)
        .bind(auth.id)
        .bind(e.value)
        .bind(&e.grade_type)
        .bind(&e.comment)
        .execute(&state.db)
        .await?;
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct AttendanceEntry {
    student_id: Uuid,
    status: String,
    comment: Option<String>,
}

#[derive(Deserialize)]
pub struct BulkAttendance {
    entries: Vec<AttendanceEntry>,
}

async fn bulk_attendance(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(lesson_id): Path<Uuid>,
    Json(body): Json<BulkAttendance>,
) -> AppResult<axum::http::StatusCode> {
    can_grade_lesson(&state, &auth, lesson_id).await?;
    for e in &body.entries {
        sqlx::query(
            "INSERT INTO attendance (lesson_id, student_id, status, comment)
             VALUES ($1,$2,$3,$4)
             ON CONFLICT (lesson_id, student_id)
             DO UPDATE SET status = EXCLUDED.status, comment = EXCLUDED.comment",
        )
        .bind(lesson_id)
        .bind(e.student_id)
        .bind(&e.status)
        .bind(&e.comment)
        .execute(&state.db)
        .await?;
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct GradesQuery {
    subject_id: Option<Uuid>,
}

async fn student_grades(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(student_id): Path<Uuid>,
    Query(q): Query<GradesQuery>,
) -> AppResult<Json<Vec<Grade>>> {
    can_view_student(&state, &auth, student_id).await?;
    let grades = match q.subject_id {
        Some(sub) => {
            sqlx::query_as::<_, Grade>(
                "SELECT id, lesson_id, student_id, subject_id, teacher_id,
                        value::float8 AS value, type, date, comment
                 FROM grades WHERE student_id = $1 AND subject_id = $2
                 ORDER BY date DESC",
            )
            .bind(student_id)
            .bind(sub)
            .fetch_all(&state.db)
            .await?
        }
        None => {
            sqlx::query_as::<_, Grade>(
                "SELECT id, lesson_id, student_id, subject_id, teacher_id,
                        value::float8 AS value, type, date, comment
                 FROM grades WHERE student_id = $1 ORDER BY date DESC",
            )
            .bind(student_id)
            .fetch_all(&state.db)
            .await?
        }
    };
    Ok(Json(grades))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct AttendanceRow {
    lesson_id: Uuid,
    student_id: Uuid,
    status: String,
    comment: Option<String>,
    date: NaiveDate,
    subject_id: Uuid,
    lesson_number: i32,
}

async fn student_attendance(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(student_id): Path<Uuid>,
    Query(q): Query<GradesQuery>,
) -> AppResult<Json<Vec<AttendanceRow>>> {
    can_view_student(&state, &auth, student_id).await?;
    let rows = match q.subject_id {
        Some(sub) => {
            sqlx::query_as::<_, AttendanceRow>(
                "SELECT a.lesson_id, a.student_id, a.status, a.comment,
                        l.date, l.subject_id, l.lesson_number
                 FROM attendance a JOIN lessons l ON l.id = a.lesson_id
                 WHERE a.student_id = $1 AND l.subject_id = $2 ORDER BY l.date DESC",
            )
            .bind(student_id)
            .bind(sub)
            .fetch_all(&state.db)
            .await?
        }
        None => {
            sqlx::query_as::<_, AttendanceRow>(
                "SELECT a.lesson_id, a.student_id, a.status, a.comment,
                        l.date, l.subject_id, l.lesson_number
                 FROM attendance a JOIN lessons l ON l.id = a.lesson_id
                 WHERE a.student_id = $1 ORDER BY l.date DESC",
            )
            .bind(student_id)
            .fetch_all(&state.db)
            .await?
        }
    };
    Ok(Json(rows))
}

#[derive(Serialize)]
pub struct JournalResponse {
    students: Vec<UserView>,
    lessons: Vec<serde_json::Value>,
    grades: Vec<Grade>,
    attendance: Vec<AttendanceRow>,
}

#[derive(Deserialize)]
pub struct JournalQuery {
    subject_id: Uuid,
}

async fn class_journal(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(class_id): Path<Uuid>,
    Query(q): Query<JournalQuery>,
) -> AppResult<Json<JournalResponse>> {
    can_view_class(&state, &auth, class_id).await?;

    let students = sqlx::query_as::<_, UserView>(
        "SELECT u.id, u.school_id, u.email, u.role, u.first_name, u.last_name, u.phone,
                u.avatar_url, u.is_active, u.created_at
         FROM users u JOIN enrollments e ON e.student_id = u.id
         WHERE e.class_id = $1 ORDER BY u.last_name",
    )
    .bind(class_id)
    .fetch_all(&state.db)
    .await?;

    let lessons: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id', id, 'date', date, 'lesson_number', lesson_number, 'topic', topic)
         FROM lessons WHERE class_id = $1 AND subject_id = $2 ORDER BY date, lesson_number",
    )
    .bind(class_id)
    .bind(q.subject_id)
    .fetch_all(&state.db)
    .await?;

    let grades = sqlx::query_as::<_, Grade>(
        "SELECT g.id, g.lesson_id, g.student_id, g.subject_id, g.teacher_id,
                g.value::float8 AS value, g.type, g.date, g.comment
         FROM grades g JOIN lessons l ON l.id = g.lesson_id
         WHERE l.class_id = $1 AND g.subject_id = $2 ORDER BY g.date",
    )
    .bind(class_id)
    .bind(q.subject_id)
    .fetch_all(&state.db)
    .await?;

    let attendance = sqlx::query_as::<_, AttendanceRow>(
        "SELECT a.lesson_id, a.student_id, a.status, a.comment,
                l.date, l.subject_id, l.lesson_number
         FROM attendance a JOIN lessons l ON l.id = a.lesson_id
         WHERE l.class_id = $1 AND l.subject_id = $2 ORDER BY l.date",
    )
    .bind(class_id)
    .bind(q.subject_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(JournalResponse {
        students,
        lessons,
        grades,
        attendance,
    }))
}

#[derive(Deserialize)]
pub struct UpdateGrade {
    value: Option<f64>,
    comment: Option<String>,
}

async fn update_grade(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateGrade>,
) -> AppResult<axum::http::StatusCode> {
    let row: Option<(Uuid, Uuid)> =
        sqlx::query_as("SELECT school_id, teacher_id FROM grades WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    let (school_id, teacher_id) = row.ok_or(AppError::NotFound)?;
    let ok = match auth.role {
        Role::Superadmin => true,
        Role::Admin => auth.school_id == Some(school_id),
        Role::Teacher => teacher_id == auth.id,
        _ => false,
    };
    if !ok {
        return Err(AppError::Forbidden);
    }
    if let Some(v) = body.value {
        validate_grade(&state, school_id, v).await?;
    }
    sqlx::query(
        "UPDATE grades SET value = COALESCE($2, value), comment = COALESCE($3, comment)
         WHERE id = $1",
    )
    .bind(id)
    .bind(body.value)
    .bind(&body.comment)
    .execute(&state.db)
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn delete_grade(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<axum::http::StatusCode> {
    let row: Option<(Uuid, Uuid)> =
        sqlx::query_as("SELECT school_id, teacher_id FROM grades WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    let (school_id, teacher_id) = row.ok_or(AppError::NotFound)?;
    let ok = match auth.role {
        Role::Superadmin => true,
        Role::Admin => auth.school_id == Some(school_id),
        Role::Teacher => teacher_id == auth.id,
        _ => false,
    };
    if !ok {
        return Err(AppError::Forbidden);
    }
    sqlx::query("DELETE FROM grades WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/lessons/{id}/grades", post(bulk_grades))
        .route("/lessons/{id}/attendance", post(bulk_attendance))
        .route("/students/{id}/grades", get(student_grades))
        .route("/students/{id}/attendance", get(student_attendance))
        .route("/classes/{id}/journal", get(class_journal))
        .route(
            "/grades/{id}",
            axum::routing::patch(update_grade).delete(delete_grade),
        )
}
