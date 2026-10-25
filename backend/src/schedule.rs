use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use validator::Validate;

use crate::AppState;
use crate::auth::AuthUser;
use crate::classes::can_view_class;
use crate::error::{AppError, AppResult};
use crate::types::Role;

// ---------- timetable slots ----------

#[derive(Serialize, sqlx::FromRow)]
pub struct Slot {
    id: Uuid,
    class_id: Uuid,
    subject_id: Uuid,
    teacher_id: Uuid,
    weekday: i32,
    lesson_number: i32,
    room: Option<String>,
}

#[derive(Deserialize, Validate)]
pub struct UpsertSlot {
    class_id: Uuid,
    subject_id: Uuid,
    teacher_id: Uuid,
    #[validate(range(min = 1, max = 7))]
    weekday: i32,
    #[validate(range(min = 1, max = 12))]
    lesson_number: i32,
    room: Option<String>,
}

#[derive(Deserialize)]
pub struct TimetableQuery {
    class_id: Option<Uuid>,
    teacher_id: Option<Uuid>,
}

async fn create_slot(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<UpsertSlot>,
) -> AppResult<Json<Slot>> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    body.validate()?;
    let sid = auth.school()?;
    let valid: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM subjects s, users t, classes c
         WHERE s.id = $1 AND t.id = $2 AND t.role = 'teacher' AND c.id = $3
           AND s.school_id = $4 AND t.school_id = $4 AND c.school_id = $4)",
    )
    .bind(body.subject_id)
    .bind(body.teacher_id)
    .bind(body.class_id)
    .bind(sid)
    .fetch_one(&state.db)
    .await?;
    if !valid {
        return Err(AppError::BadRequest(
            "class, subject or teacher not found in your school".into(),
        ));
    }
    let slot = sqlx::query_as::<_, Slot>(
        "INSERT INTO timetable_slots (id, school_id, class_id, subject_id, teacher_id, weekday, lesson_number, room)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
         ON CONFLICT (class_id, weekday, lesson_number)
         DO UPDATE SET subject_id = EXCLUDED.subject_id, teacher_id = EXCLUDED.teacher_id, room = EXCLUDED.room
         RETURNING id, class_id, subject_id, teacher_id, weekday, lesson_number, room",
    )
    .bind(Uuid::new_v4())
    .bind(sid)
    .bind(body.class_id)
    .bind(body.subject_id)
    .bind(body.teacher_id)
    .bind(body.weekday)
    .bind(body.lesson_number)
    .bind(&body.room)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(slot))
}

async fn timetable(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<TimetableQuery>,
) -> AppResult<Json<Vec<Slot>>> {
    let class_id = q
        .class_id
        .ok_or(AppError::BadRequest("class_id is required".into()))?;
    can_view_class(&state, &auth, class_id).await?;

    let slots = if let Some(tid) = q.teacher_id {
        sqlx::query_as::<_, Slot>(
            "SELECT id, class_id, subject_id, teacher_id, weekday, lesson_number, room
             FROM timetable_slots WHERE class_id = $1 AND teacher_id = $2
             ORDER BY weekday, lesson_number",
        )
        .bind(class_id)
        .bind(tid)
        .fetch_all(&state.db)
        .await?
    } else {
        sqlx::query_as::<_, Slot>(
            "SELECT id, class_id, subject_id, teacher_id, weekday, lesson_number, room
             FROM timetable_slots WHERE class_id = $1
             ORDER BY weekday, lesson_number",
        )
        .bind(class_id)
        .fetch_all(&state.db)
        .await?
    };
    Ok(Json(slots))
}

async fn my_timetable(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<Slot>>> {
    auth.require(&[Role::Teacher])?;
    let slots = sqlx::query_as::<_, Slot>(
        "SELECT id, class_id, subject_id, teacher_id, weekday, lesson_number, room
         FROM timetable_slots WHERE teacher_id = $1 AND school_id = $2
         ORDER BY weekday, lesson_number",
    )
    .bind(auth.id)
    .bind(auth.school_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(slots))
}

async fn delete_slot(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<axum::http::StatusCode> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    let res = sqlx::query("DELETE FROM timetable_slots WHERE id = $1 AND school_id = $2")
        .bind(id)
        .bind(auth.school()?)
        .execute(&state.db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ---------- lessons ----------

#[derive(Serialize, sqlx::FromRow)]
pub struct Lesson {
    pub id: Uuid,
    pub school_id: Uuid,
    pub class_id: Uuid,
    pub subject_id: Uuid,
    pub teacher_id: Uuid,
    pub date: NaiveDate,
    pub lesson_number: i32,
    pub topic: Option<String>,
    pub room: Option<String>,
    pub created_at: DateTime<Utc>,
}

const LESSON_COLS: &str =
    "id, school_id, class_id, subject_id, teacher_id, date, lesson_number, topic, room, created_at";

#[derive(Deserialize)]
pub struct GenerateLessons {
    class_id: Uuid,
    /// Monday of the week to generate.
    week_start: NaiveDate,
}

async fn generate_lessons(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<GenerateLessons>,
) -> AppResult<Json<Vec<Lesson>>> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    if body.week_start.weekday() != chrono::Weekday::Mon {
        return Err(AppError::BadRequest("week_start must be a Monday".into()));
    }
    let sid = auth.school()?;
    // weekday N maps to week_start + (N-1) days
    sqlx::query(
        "INSERT INTO lessons (id, school_id, class_id, subject_id, teacher_id, date, lesson_number, room)
         SELECT gen_random_uuid(), ts.school_id, ts.class_id, ts.subject_id, ts.teacher_id,
                $1::date + (ts.weekday - 1), ts.lesson_number, ts.room
         FROM timetable_slots ts
         WHERE ts.school_id = $2 AND ts.class_id = $3
         ON CONFLICT (class_id, date, lesson_number) DO NOTHING",
    )
    .bind(body.week_start)
    .bind(sid)
    .bind(body.class_id)
    .execute(&state.db)
    .await?;
    let lessons = sqlx::query_as::<_, Lesson>(sqlx::AssertSqlSafe(format!(
        "SELECT {LESSON_COLS} FROM lessons WHERE class_id = $1 AND date >= $2 AND date < $2 + 7
         ORDER BY date, lesson_number"
    )))
    .bind(body.class_id)
    .bind(body.week_start)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(lessons))
}

#[derive(Deserialize)]
pub struct LessonsQuery {
    class_id: Option<Uuid>,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
}

async fn list_lessons(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<LessonsQuery>,
) -> AppResult<Json<Vec<Lesson>>> {
    if let Some(class_id) = q.class_id {
        can_view_class(&state, &auth, class_id).await?;
    } else {
        // teacher's own lessons or admin whole school
        match auth.role {
            Role::Superadmin | Role::Admin => {}
            Role::Teacher => {}
            _ => return Err(AppError::BadRequest("class_id is required".into())),
        }
    }
    let from = q.from.unwrap_or_else(|| Utc::now().date_naive());
    let to = q.to.unwrap_or(from + chrono::Duration::days(7));

    let lessons = match auth.role {
        Role::Teacher if q.class_id.is_none() => {
            sqlx::query_as::<_, Lesson>(sqlx::AssertSqlSafe(format!(
                "SELECT {LESSON_COLS} FROM lessons WHERE teacher_id = $1 AND date >= $2 AND date < $3
                 ORDER BY date, lesson_number"
            )))
            .bind(auth.id)
            .bind(from)
            .bind(to)
            .fetch_all(&state.db)
            .await?
        }
        Role::Superadmin => {
            if let Some(cid) = q.class_id {
                sqlx::query_as::<_, Lesson>(sqlx::AssertSqlSafe(format!(
                    "SELECT {LESSON_COLS} FROM lessons WHERE class_id = $1 AND date >= $2 AND date < $3
                     ORDER BY date, lesson_number"
                )))
                .bind(cid)
                .bind(from)
                .bind(to)
                .fetch_all(&state.db)
                .await?
            } else {
                sqlx::query_as::<_, Lesson>(sqlx::AssertSqlSafe(format!(
                    "SELECT {LESSON_COLS} FROM lessons WHERE date >= $1 AND date < $2
                     ORDER BY date, lesson_number"
                )))
                .bind(from)
                .bind(to)
                .fetch_all(&state.db)
                .await?
            }
        }
        _ => {
            if let Some(cid) = q.class_id {
                sqlx::query_as::<_, Lesson>(sqlx::AssertSqlSafe(format!(
                    "SELECT {LESSON_COLS} FROM lessons WHERE class_id = $1 AND school_id = $2 AND date >= $3 AND date < $4
                     ORDER BY date, lesson_number"
                )))
                .bind(cid)
                .bind(auth.school_id)
                .bind(from)
                .bind(to)
                .fetch_all(&state.db)
                .await?
            } else {
                sqlx::query_as::<_, Lesson>(sqlx::AssertSqlSafe(format!(
                    "SELECT {LESSON_COLS} FROM lessons WHERE school_id = $1 AND date >= $2 AND date < $3
                     ORDER BY date, lesson_number"
                )))
                .bind(auth.school_id)
                .bind(from)
                .bind(to)
                .fetch_all(&state.db)
                .await?
            }
        }
    };
    Ok(Json(lessons))
}

#[derive(Deserialize, Validate)]
pub struct CreateLesson {
    class_id: Uuid,
    subject_id: Uuid,
    teacher_id: Option<Uuid>,
    date: NaiveDate,
    #[validate(range(min = 1, max = 12))]
    lesson_number: i32,
    topic: Option<String>,
    room: Option<String>,
}

async fn create_lesson(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateLesson>,
) -> AppResult<Json<Lesson>> {
    body.validate()?;
    let teacher_id = match auth.role {
        Role::Teacher => auth.id,
        Role::Superadmin | Role::Admin => body.teacher_id.ok_or(AppError::BadRequest(
            "teacher_id is required".into(),
        ))?,
        _ => return Err(AppError::Forbidden),
    };
    let lesson = sqlx::query_as::<_, Lesson>(sqlx::AssertSqlSafe(format!(
        "INSERT INTO lessons (id, school_id, class_id, subject_id, teacher_id, date, lesson_number, topic, room)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
         ON CONFLICT (class_id, date, lesson_number) DO UPDATE SET
            subject_id = EXCLUDED.subject_id, teacher_id = EXCLUDED.teacher_id,
            topic = EXCLUDED.topic, room = EXCLUDED.room
         RETURNING {LESSON_COLS}"
    )))
    .bind(Uuid::new_v4())
    .bind(auth.school()?)
    .bind(body.class_id)
    .bind(body.subject_id)
    .bind(teacher_id)
    .bind(body.date)
    .bind(body.lesson_number)
    .bind(&body.topic)
    .bind(&body.room)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(lesson))
}

#[derive(Deserialize)]
pub struct UpdateLesson {
    topic: Option<String>,
    room: Option<String>,
}

async fn update_lesson(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateLesson>,
) -> AppResult<Json<Lesson>> {
    let lesson = sqlx::query_as::<_, Lesson>(sqlx::AssertSqlSafe(format!(
        "UPDATE lessons SET topic = COALESCE($2, topic), room = COALESCE($3, room)
         WHERE id = $1 AND school_id = $4 AND (teacher_id = $5 OR $6)
         RETURNING {LESSON_COLS}"
    )))
    .bind(id)
    .bind(&body.topic)
    .bind(&body.room)
    .bind(auth.school()?)
    .bind(auth.id)
    .bind(matches!(auth.role, Role::Admin | Role::Superadmin))
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(lesson))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/timetable", get(timetable).post(create_slot))
        .route("/timetable/me", get(my_timetable))
        .route("/timetable/{id}", axum::routing::delete(delete_slot))
        .route("/lessons", get(list_lessons).post(create_lesson))
        .route("/lessons/generate", axum::routing::post(generate_lessons))
        .route("/lessons/{id}", axum::routing::patch(update_lesson))
}
