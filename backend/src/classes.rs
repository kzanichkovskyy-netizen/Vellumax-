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
use crate::users::UserView;

#[derive(Serialize, sqlx::FromRow)]
pub struct ClassView {
    id: Uuid,
    school_id: Uuid,
    name: String,
    grade_level: i32,
    homeroom_teacher_id: Option<Uuid>,
    year: i32,
    #[serde(rename = "type")]
    #[sqlx(rename = "type")]
    class_type: String,
    location_id: Option<Uuid>,
    created_at: DateTime<Utc>,
}

const CLASS_COLS: &str =
    "id, school_id, name, grade_level, homeroom_teacher_id, year, type, location_id, created_at";
const CLASS_COLS_C: &str = "c.id, c.school_id, c.name, c.grade_level, c.homeroom_teacher_id, c.year, c.type, c.location_id, c.created_at";

#[derive(Deserialize, Validate)]
pub struct CreateClass {
    #[validate(length(min = 1, max = 50))]
    name: String,
    grade_level: i32,
    year: i32,
    homeroom_teacher_id: Option<Uuid>,
    /// 'class' (default) or 'club' (гурток)
    #[serde(rename = "type")]
    class_type: Option<String>,
    location_id: Option<Uuid>,
}

#[derive(Deserialize, Validate)]
pub struct UpdateClass {
    #[validate(length(min = 1, max = 50))]
    name: Option<String>,
    grade_level: Option<i32>,
    year: Option<i32>,
    homeroom_teacher_id: Option<Uuid>,
    location_id: Option<Uuid>,
}

#[derive(Deserialize)]
pub struct Enroll {
    student_id: Uuid,
}

/// Whether the caller may view this class (membership-aware).
pub(crate) async fn can_view_class(
    state: &AppState,
    auth: &AuthUser,
    class_id: Uuid,
) -> AppResult<()> {
    let ok: bool = match auth.role {
        Role::Superadmin => true,
        Role::Admin => {
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM classes WHERE id=$1 AND school_id=$2)")
                .bind(class_id)
                .bind(auth.school_id)
                .fetch_one(&state.db)
                .await?
        }
        Role::Teacher => {
            sqlx::query_scalar(
                "SELECT EXISTS(
                    SELECT 1 FROM classes c
                    WHERE c.id = $1 AND c.school_id = $2 AND (
                        c.homeroom_teacher_id = $3 OR EXISTS(
                            SELECT 1 FROM class_subject_teachers cst
                            WHERE cst.class_id = c.id AND cst.teacher_id = $3)))",
            )
            .bind(class_id)
            .bind(auth.school_id)
            .bind(auth.id)
            .fetch_one(&state.db)
            .await?
        }
        Role::Student => {
            sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM enrollments WHERE class_id=$1 AND student_id=$2)",
            )
            .bind(class_id)
            .bind(auth.id)
            .fetch_one(&state.db)
            .await?
        }
        Role::Parent => {
            sqlx::query_scalar(
                "SELECT EXISTS(
                    SELECT 1 FROM enrollments e
                    JOIN parent_links pl ON pl.student_id = e.student_id
                    WHERE e.class_id = $1 AND pl.parent_id = $2)",
            )
            .bind(class_id)
            .bind(auth.id)
            .fetch_one(&state.db)
            .await?
        }
    };
    if ok { Ok(()) } else { Err(AppError::Forbidden) }
}

async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateClass>,
) -> AppResult<Json<ClassView>> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    body.validate()?;
    let ctype = body.class_type.as_deref().unwrap_or("class");
    if !["class", "club"].contains(&ctype) {
        return Err(AppError::BadRequest(
            "type must be 'class' or 'club'".into(),
        ));
    }
    let class = sqlx::query_as::<_, ClassView>(sqlx::AssertSqlSafe(format!(
        "INSERT INTO classes (id, school_id, name, grade_level, year, homeroom_teacher_id, type, location_id)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING {CLASS_COLS}"
    )))
    .bind(Uuid::new_v4())
    .bind(auth.school()?)
    .bind(&body.name)
    .bind(body.grade_level)
    .bind(body.year)
    .bind(body.homeroom_teacher_id)
    .bind(ctype)
    .bind(body.location_id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(class))
}

async fn list(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Vec<ClassView>>> {
    let classes = match auth.role {
        Role::Superadmin => {
            sqlx::query_as::<_, ClassView>(sqlx::AssertSqlSafe(format!(
                "SELECT {CLASS_COLS} FROM classes ORDER BY school_id, name"
            )))
            .fetch_all(&state.db)
            .await?
        }
        Role::Admin => {
            sqlx::query_as::<_, ClassView>(sqlx::AssertSqlSafe(format!(
                "SELECT {CLASS_COLS} FROM classes WHERE school_id = $1 ORDER BY name"
            )))
            .bind(auth.school_id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Teacher => {
            sqlx::query_as::<_, ClassView>(sqlx::AssertSqlSafe(format!(
                "SELECT DISTINCT {CLASS_COLS_C} FROM classes c
                 LEFT JOIN class_subject_teachers cst ON cst.class_id = c.id
                 WHERE c.school_id = $1 AND (c.homeroom_teacher_id = $2 OR cst.teacher_id = $2)
                 ORDER BY c.name"
            )))
            .bind(auth.school_id)
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Student => {
            sqlx::query_as::<_, ClassView>(sqlx::AssertSqlSafe(format!(
                "SELECT {CLASS_COLS_C} FROM classes c
                 JOIN enrollments e ON e.class_id = c.id
                 WHERE e.student_id = $1 ORDER BY c.name"
            )))
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Parent => {
            sqlx::query_as::<_, ClassView>(sqlx::AssertSqlSafe(format!(
                "SELECT DISTINCT {CLASS_COLS_C} FROM classes c
                 JOIN enrollments e ON e.class_id = c.id
                 JOIN parent_links pl ON pl.student_id = e.student_id
                 WHERE pl.parent_id = $1 ORDER BY c.name"
            )))
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
    };
    Ok(Json(classes))
}

async fn get_one(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<ClassView>> {
    can_view_class(&state, &auth, id).await?;
    let class = sqlx::query_as::<_, ClassView>(sqlx::AssertSqlSafe(format!(
        "SELECT {CLASS_COLS} FROM classes WHERE id = $1"
    )))
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(class))
}

async fn update(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateClass>,
) -> AppResult<Json<ClassView>> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    body.validate()?;
    let class = sqlx::query_as::<_, ClassView>(sqlx::AssertSqlSafe(format!(
        "UPDATE classes SET
            name = COALESCE($2, name),
            grade_level = COALESCE($3, grade_level),
            year = COALESCE($4, year),
            homeroom_teacher_id = COALESCE($5, homeroom_teacher_id),
            location_id = COALESCE($6, location_id)
         WHERE id = $1 AND school_id = $7
         RETURNING {CLASS_COLS}"
    )))
    .bind(id)
    .bind(&body.name)
    .bind(body.grade_level)
    .bind(body.year)
    .bind(body.homeroom_teacher_id)
    .bind(body.location_id)
    .bind(auth.school()?)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(class))
}

async fn delete(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<axum::http::StatusCode> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    let res = sqlx::query("DELETE FROM classes WHERE id = $1 AND school_id = $2")
        .bind(id)
        .bind(auth.school()?)
        .execute(&state.db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn enroll(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<Enroll>,
) -> AppResult<axum::http::StatusCode> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    let sid = auth.school()?;
    let valid: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM classes c, users u
         WHERE c.id = $1 AND u.id = $2 AND u.role = 'student'
           AND c.school_id = $3 AND u.school_id = $3)",
    )
    .bind(id)
    .bind(body.student_id)
    .bind(sid)
    .fetch_one(&state.db)
    .await?;
    if !valid {
        return Err(AppError::BadRequest(
            "class or student not found in your school".into(),
        ));
    }
    sqlx::query(
        "INSERT INTO enrollments (class_id, student_id) VALUES ($1,$2) ON CONFLICT DO NOTHING",
    )
    .bind(id)
    .bind(body.student_id)
    .execute(&state.db)
    .await?;
    Ok(axum::http::StatusCode::CREATED)
}

async fn unenroll(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((id, student_id)): Path<(Uuid, Uuid)>,
) -> AppResult<axum::http::StatusCode> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    sqlx::query(
        "DELETE FROM enrollments WHERE class_id = $1 AND student_id = $2
         AND EXISTS(SELECT 1 FROM classes c WHERE c.id = $1 AND c.school_id = $3)",
    )
    .bind(id)
    .bind(student_id)
    .bind(auth.school()?)
    .execute(&state.db)
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn students(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Vec<UserView>>> {
    can_view_class(&state, &auth, id).await?;
    let students = sqlx::query_as::<_, UserView>(
        "SELECT u.id, u.school_id, u.email, u.role, u.first_name, u.last_name, u.phone,
                u.avatar_url, u.is_active, u.created_at, u.birth_date, u.address, u.custom_fields
         FROM users u JOIN enrollments e ON e.student_id = u.id
         WHERE e.class_id = $1 ORDER BY u.last_name",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(students))
}

// ---- subject assignment ----

#[derive(Deserialize)]
pub struct AssignSubject {
    subject_id: Uuid,
    teacher_id: Uuid,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct ClassSubject {
    subject_id: Uuid,
    subject_name: String,
    teacher_id: Uuid,
    teacher_first_name: String,
    teacher_last_name: String,
}

async fn assign_subject(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<AssignSubject>,
) -> AppResult<axum::http::StatusCode> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    let sid = auth.school()?;
    let valid: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM subjects s, users t, classes c
         WHERE s.id = $2 AND t.id = $3 AND t.role = 'teacher' AND c.id = $1
           AND s.school_id = $4 AND t.school_id = $4 AND c.school_id = $4)",
    )
    .bind(id)
    .bind(body.subject_id)
    .bind(body.teacher_id)
    .bind(sid)
    .fetch_one(&state.db)
    .await?;
    if !valid {
        return Err(AppError::BadRequest(
            "class, subject or teacher not found in your school".into(),
        ));
    }
    sqlx::query(
        "INSERT INTO class_subject_teachers (class_id, subject_id, teacher_id)
         VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
    )
    .bind(id)
    .bind(body.subject_id)
    .bind(body.teacher_id)
    .execute(&state.db)
    .await?;
    Ok(axum::http::StatusCode::CREATED)
}

async fn unassign_subject(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((id, subject_id)): Path<(Uuid, Uuid)>,
) -> AppResult<axum::http::StatusCode> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    sqlx::query(
        "DELETE FROM class_subject_teachers WHERE class_id = $1 AND subject_id = $2
         AND EXISTS(SELECT 1 FROM classes c WHERE c.id = $1 AND c.school_id = $3)",
    )
    .bind(id)
    .bind(subject_id)
    .bind(auth.school()?)
    .execute(&state.db)
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn class_subjects(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Vec<ClassSubject>>> {
    can_view_class(&state, &auth, id).await?;
    let rows = sqlx::query_as::<_, ClassSubject>(
        "SELECT cst.subject_id, s.name AS subject_name, cst.teacher_id,
                t.first_name AS teacher_first_name, t.last_name AS teacher_last_name
         FROM class_subject_teachers cst
         JOIN subjects s ON s.id = cst.subject_id
         JOIN users t ON t.id = cst.teacher_id
         WHERE cst.class_id = $1
         ORDER BY s.name, t.last_name",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

// ---- subgroups ----

#[derive(Serialize, sqlx::FromRow)]
pub struct Subgroup {
    id: Uuid,
    class_id: Uuid,
    subject_id: Option<Uuid>,
    name: String,
}

#[derive(Deserialize, Validate)]
pub struct CreateSubgroup {
    #[validate(length(min = 1, max = 100))]
    name: String,
    subject_id: Option<Uuid>,
}

async fn create_subgroup(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<CreateSubgroup>,
) -> AppResult<Json<Subgroup>> {
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    body.validate()?;
    let g = sqlx::query_as::<_, Subgroup>(
        "INSERT INTO subgroups (id, class_id, subject_id, name)
         SELECT $1, c.id, $3, $4 FROM classes c
         WHERE c.id = $2 AND c.school_id = $5
         RETURNING id, class_id, subject_id, name",
    )
    .bind(Uuid::new_v4())
    .bind(id)
    .bind(body.subject_id)
    .bind(&body.name)
    .bind(auth.school()?)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(g))
}

async fn list_subgroups(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Vec<Subgroup>>> {
    can_view_class(&state, &auth, id).await?;
    let rows = sqlx::query_as::<_, Subgroup>(
        "SELECT id, class_id, subject_id, name FROM subgroups WHERE class_id = $1 ORDER BY name",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

async fn delete_subgroup(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((id, subgroup_id)): Path<(Uuid, Uuid)>,
) -> AppResult<axum::http::StatusCode> {
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    let res = sqlx::query(
        "DELETE FROM subgroups WHERE id = $1 AND class_id = $2
         AND EXISTS(SELECT 1 FROM classes c WHERE c.id = $2 AND c.school_id = $3)",
    )
    .bind(subgroup_id)
    .bind(id)
    .bind(auth.school()?)
    .execute(&state.db)
    .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct SubgroupEnroll {
    student_id: Uuid,
}

async fn subgroup_add_student(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((id, subgroup_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<SubgroupEnroll>,
) -> AppResult<axum::http::StatusCode> {
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    // student must be enrolled in the parent class
    let ok: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM subgroups sg
            JOIN enrollments e ON e.class_id = sg.class_id
            JOIN classes c ON c.id = sg.class_id
            WHERE sg.id = $1 AND sg.class_id = $2 AND e.student_id = $3 AND c.school_id = $4)",
    )
    .bind(subgroup_id)
    .bind(id)
    .bind(body.student_id)
    .bind(auth.school()?)
    .fetch_one(&state.db)
    .await?;
    if !ok {
        return Err(AppError::BadRequest(
            "student not enrolled in this class".into(),
        ));
    }
    sqlx::query(
        "INSERT INTO subgroup_students (subgroup_id, student_id) VALUES ($1,$2) ON CONFLICT DO NOTHING",
    )
    .bind(subgroup_id)
    .bind(body.student_id)
    .execute(&state.db)
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn subgroup_remove_student(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((id, subgroup_id, student_id)): Path<(Uuid, Uuid, Uuid)>,
) -> AppResult<axum::http::StatusCode> {
    let _ = id;
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    sqlx::query("DELETE FROM subgroup_students WHERE subgroup_id = $1 AND student_id = $2")
        .bind(subgroup_id)
        .bind(student_id)
        .execute(&state.db)
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn subgroup_students(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((id, subgroup_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<Vec<UserView>>> {
    can_view_class(&state, &auth, id).await?;
    let rows = sqlx::query_as::<_, UserView>(
        "SELECT u.id, u.school_id, u.email, u.role, u.first_name, u.last_name, u.phone,
                u.avatar_url, u.is_active, u.created_at, u.birth_date, u.address, u.custom_fields
         FROM users u
         JOIN subgroup_students ss ON ss.student_id = u.id
         WHERE ss.subgroup_id = $1 ORDER BY u.last_name",
    )
    .bind(subgroup_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(get_one).patch(update).delete(delete))
        .route("/{id}/students", get(students).post(enroll))
        .route(
            "/{id}/students/{student_id}",
            axum::routing::delete(unenroll),
        )
        .route("/{id}/subjects", get(class_subjects).post(assign_subject))
        .route(
            "/{id}/subjects/{subject_id}",
            axum::routing::delete(unassign_subject),
        )
        .route("/{id}/subgroups", get(list_subgroups).post(create_subgroup))
        .route(
            "/{id}/subgroups/{subgroup_id}",
            axum::routing::delete(delete_subgroup),
        )
        .route(
            "/{id}/subgroups/{subgroup_id}/students",
            get(subgroup_students).post(subgroup_add_student),
        )
        .route(
            "/{id}/subgroups/{subgroup_id}/students/{student_id}",
            axum::routing::delete(subgroup_remove_student),
        )
}
