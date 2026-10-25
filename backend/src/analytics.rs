use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::NaiveDate;
use serde::Deserialize;
use uuid::Uuid;

use crate::AppState;
use crate::auth::AuthUser;
use crate::classes::can_view_class;
use crate::error::{AppError, AppResult};
use crate::types::Role;

#[derive(Deserialize)]
pub struct RangeQuery {
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    class_id: Option<Uuid>,
}

/// Attendance percentage per student (optionally per class filter).
async fn attendance_stats(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<RangeQuery>,
) -> AppResult<Json<Vec<serde_json::Value>>> {
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    let from = q
        .from
        .unwrap_or(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap());
    let to = q.to.unwrap_or(NaiveDate::from_ymd_opt(2100, 1, 1).unwrap());
    let rows: Vec<(Uuid, String, String, i64, i64)> = sqlx::query_as(
        "SELECT u.id, u.first_name, u.last_name,
                COUNT(*) FILTER (WHERE a.status IN ('present','late')) AS attended,
                COUNT(*) AS total
         FROM attendance a
         JOIN lessons l ON l.id = a.lesson_id
         JOIN users u ON u.id = a.student_id
         WHERE l.school_id = $1 AND l.date >= $2 AND l.date <= $3
           AND ($4::uuid IS NULL OR l.class_id = $4)
         GROUP BY u.id, u.first_name, u.last_name
         ORDER BY u.last_name",
    )
    .bind(auth.school()?)
    .bind(from)
    .bind(to)
    .bind(q.class_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, first, last, attended, total)| {
                serde_json::json!({
                    "student_id": id,
                    "first_name": first,
                    "last_name": last,
                    "attended": attended,
                    "total": total,
                    "rate": if total > 0 { attended as f64 / total as f64 } else { 0.0 },
                })
            })
            .collect(),
    ))
}

/// Lessons per teacher per week derived from the timetable — workload control.
async fn teacher_load(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<serde_json::Value>>> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    let rows: Vec<(Uuid, String, String, i64)> = sqlx::query_as(
        "SELECT u.id, u.first_name, u.last_name, COUNT(*) AS weekly_lessons
         FROM timetable_slots ts JOIN users u ON u.id = ts.teacher_id
         WHERE ts.school_id = $1
         GROUP BY u.id, u.first_name, u.last_name
         ORDER BY weekly_lessons DESC",
    )
    .bind(auth.school()?)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, first, last, n)| {
                serde_json::json!({
                    "teacher_id": id, "first_name": first, "last_name": last,
                    "weekly_lessons": n,
                })
            })
            .collect(),
    ))
}

/// Average grade per subject per student — the report card (табель успішності).
async fn report_card(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(student_id): Path<Uuid>,
    Query(q): Query<RangeQuery>,
) -> AppResult<Json<Vec<serde_json::Value>>> {
    let ok =
        match auth.role {
            Role::Superadmin | Role::Admin => true,
            Role::Student => auth.id == student_id,
            Role::Parent => sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM parent_links WHERE parent_id=$1 AND student_id=$2)",
            )
            .bind(auth.id)
            .bind(student_id)
            .fetch_one(&state.db)
            .await?,
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
    if !ok {
        return Err(AppError::Forbidden);
    }
    let from = q
        .from
        .unwrap_or(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap());
    let to = q.to.unwrap_or(NaiveDate::from_ymd_opt(2100, 1, 1).unwrap());
    let rows: Vec<(Uuid, String, i64, f64)> = sqlx::query_as(
        "SELECT s.id, s.name, COUNT(*) AS cnt, AVG(g.value)::float8 AS avg
         FROM grades g JOIN subjects s ON s.id = g.subject_id
         WHERE g.student_id = $1 AND g.date >= $2 AND g.date <= $3
         GROUP BY s.id, s.name ORDER BY s.name",
    )
    .bind(student_id)
    .bind(from)
    .bind(to)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, cnt, avg)| {
                serde_json::json!({
                    "subject_id": id, "subject": name,
                    "grades_count": cnt, "average": (avg * 100.0).round() / 100.0,
                })
            })
            .collect(),
    ))
}

/// Class grade stats per subject — teacher effectiveness view.
async fn grade_stats(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<RangeQuery>,
) -> AppResult<Json<Vec<serde_json::Value>>> {
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    let from = q
        .from
        .unwrap_or(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap());
    let to = q.to.unwrap_or(NaiveDate::from_ymd_opt(2100, 1, 1).unwrap());
    let rows: Vec<(String, i64, f64, i64)> = sqlx::query_as(
        "SELECT s.name, COUNT(*) AS cnt, AVG(g.value)::float8 AS avg,
                COUNT(DISTINCT g.student_id) AS students
         FROM grades g JOIN subjects s ON s.id = g.subject_id
         WHERE g.school_id = $1 AND g.date >= $2 AND g.date <= $3
           AND ($4::uuid IS NULL OR EXISTS(
                SELECT 1 FROM lessons l WHERE l.id = g.lesson_id AND l.class_id = $4))
         GROUP BY s.id, s.name ORDER BY s.name",
    )
    .bind(auth.school()?)
    .bind(from)
    .bind(to)
    .bind(q.class_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(name, cnt, avg, students)| {
                serde_json::json!({
                    "subject": name, "grades_count": cnt,
                    "average": (avg * 100.0).round() / 100.0,
                    "students": students,
                })
            })
            .collect(),
    ))
}

/// Journal CSV export ("друк паперового журналу").
async fn journal_csv(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(class_id): Path<Uuid>,
) -> AppResult<Response> {
    can_view_class(&state, &auth, class_id).await?;
    // CSV rows: student,date,lesson,grade,attendance
    #[derive(sqlx::FromRow)]
    struct Cell {
        student: String,
        date: NaiveDate,
        lesson_number: i32,
        grade: Option<f64>,
        status: Option<String>,
    }
    let cells = sqlx::query_as::<_, Cell>(
        "SELECT u.last_name || ' ' || u.first_name AS student,
                l.date, l.lesson_number,
                g.value::float8 AS grade, a.status
         FROM lessons l
         JOIN enrollments e ON e.class_id = l.class_id
         JOIN users u ON u.id = e.student_id
         LEFT JOIN grades g ON g.lesson_id = l.id AND g.student_id = u.id
         LEFT JOIN attendance a ON a.lesson_id = l.id AND a.student_id = u.id
         WHERE l.class_id = $1
         ORDER BY u.last_name, l.date, l.lesson_number",
    )
    .bind(class_id)
    .fetch_all(&state.db)
    .await?;

    let mut csv = String::from("student,date,lesson,grade,attendance\n");
    for c in cells {
        csv.push_str(&format!(
            "{},{},{},{},{}\n",
            c.student.replace(',', " "),
            c.date,
            c.lesson_number,
            c.grade.map(|v| v.to_string()).unwrap_or_default(),
            c.status.unwrap_or_default(),
        ));
    }
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"journal_{class_id}.csv\""),
            ),
        ],
        csv,
    )
        .into_response())
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/attendance", get(attendance_stats))
        .route("/teacher-load", get(teacher_load))
        .route("/grades", get(grade_stats))
        .route("/students/{id}/report-card", get(report_card))
        .route("/classes/{id}/journal.csv", get(journal_csv))
}
