use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use validator::Validate;

use crate::AppState;
use crate::auth::{AuthUser, hash_password, verify_password};
use crate::error::{AppError, AppResult};
use crate::types::Role;

#[derive(Serialize, sqlx::FromRow)]
pub struct UserView {
    pub id: Uuid,
    pub school_id: Option<Uuid>,
    pub email: String,
    pub role: Role,
    pub first_name: String,
    pub last_name: String,
    pub phone: Option<String>,
    pub avatar_url: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

const USER_COLS: &str =
    "id, school_id, email, role, first_name, last_name, phone, avatar_url, is_active, created_at";

#[derive(Deserialize, Validate)]
pub struct CreateUser {
    #[validate(email)]
    email: String,
    #[validate(length(min = 8))]
    password: String,
    role: Role,
    #[validate(length(min = 1))]
    first_name: String,
    #[validate(length(min = 1))]
    last_name: String,
    phone: Option<String>,
    school_id: Option<Uuid>,
}

#[derive(Deserialize, Validate)]
pub struct UpdateUser {
    #[validate(email)]
    email: Option<String>,
    role: Option<Role>,
    first_name: Option<String>,
    last_name: Option<String>,
    phone: Option<String>,
    avatar_url: Option<String>,
    is_active: Option<bool>,
}

#[derive(Deserialize, Validate)]
pub struct UpdateMe {
    first_name: Option<String>,
    last_name: Option<String>,
    phone: Option<String>,
    avatar_url: Option<String>,
    current_password: Option<String>,
    #[validate(length(min = 8))]
    new_password: Option<String>,
}

#[derive(Deserialize)]
pub struct ListQuery {
    role: Option<Role>,
    school_id: Option<Uuid>,
}

fn can_manage(auth: &AuthUser, target_school: Option<Uuid>) -> AppResult<()> {
    match auth.role {
        Role::Superadmin => Ok(()),
        Role::Admin if target_school == auth.school_id => Ok(()),
        _ => Err(AppError::Forbidden),
    }
}

async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateUser>,
) -> AppResult<Json<UserView>> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    body.validate()?;

    let (school_id, role) = match (auth.role, body.role) {
        (_, Role::Superadmin) if auth.role != Role::Superadmin => {
            return Err(AppError::Forbidden);
        }
        (Role::Superadmin, Role::Superadmin) => (None, Role::Superadmin),
        (Role::Superadmin, r) => (
            Some(body.school_id.ok_or(AppError::BadRequest(
                "school_id is required".into(),
            ))?),
            r,
        ),
        (Role::Admin, Role::Admin) => (auth.school_id, Role::Admin),
        (Role::Admin, r) => (auth.school_id, r),
        _ => return Err(AppError::Forbidden),
    };

    let user = sqlx::query_as::<_, UserView>(sqlx::AssertSqlSafe(format!(
        "INSERT INTO users (id, school_id, email, password_hash, role, first_name, last_name, phone)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING {USER_COLS}"
    )))
    .bind(Uuid::new_v4())
    .bind(school_id)
    .bind(&body.email)
    .bind(hash_password(&body.password)?)
    .bind(role)
    .bind(&body.first_name)
    .bind(&body.last_name)
    .bind(&body.phone)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(user))
}

async fn list(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<Vec<UserView>>> {
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    let scope = auth.scope(q.school_id)?;

    let users = match (scope, q.role) {
        (Some(sid), Some(role)) => {
            sqlx::query_as::<_, UserView>(sqlx::AssertSqlSafe(format!(
                "SELECT {USER_COLS} FROM users WHERE school_id = $1 AND role = $2 ORDER BY last_name"
            )))
            .bind(sid)
            .bind(role)
            .fetch_all(&state.db)
            .await?
        }
        (Some(sid), None) => {
            sqlx::query_as::<_, UserView>(sqlx::AssertSqlSafe(format!(
                "SELECT {USER_COLS} FROM users WHERE school_id = $1 ORDER BY last_name"
            )))
            .bind(sid)
            .fetch_all(&state.db)
            .await?
        }
        (None, Some(role)) => {
            sqlx::query_as::<_, UserView>(sqlx::AssertSqlSafe(format!(
                "SELECT {USER_COLS} FROM users WHERE role = $1 ORDER BY last_name"
            )))
            .bind(role)
            .fetch_all(&state.db)
            .await?
        }
        (None, None) => {
            sqlx::query_as::<_, UserView>(sqlx::AssertSqlSafe(format!(
                "SELECT {USER_COLS} FROM users ORDER BY school_id, last_name"
            )))
            .fetch_all(&state.db)
            .await?
        }
    };
    Ok(Json(users))
}

async fn get_one(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<UserView>> {
    let user = sqlx::query_as::<_, UserView>(sqlx::AssertSqlSafe(format!(
        "SELECT {USER_COLS} FROM users WHERE id = $1"
    )))
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;

    if auth.id != id {
        can_manage(&auth, user.school_id)?;
    }
    Ok(Json(user))
}

async fn update(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateUser>,
) -> AppResult<Json<UserView>> {
    body.validate()?;
    let target = sqlx::query_as::<_, UserView>(sqlx::AssertSqlSafe(format!(
        "SELECT {USER_COLS} FROM users WHERE id = $1"
    )))
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;

    can_manage(&auth, target.school_id)?;
    if target.role == Role::Superadmin && auth.role != Role::Superadmin {
        return Err(AppError::Forbidden);
    }
    if body.role == Some(Role::Superadmin) && auth.role != Role::Superadmin {
        return Err(AppError::Forbidden);
    }

    let user = sqlx::query_as::<_, UserView>(sqlx::AssertSqlSafe(format!(
        "UPDATE users SET
            email = COALESCE($2, email),
            role = COALESCE($3, role),
            first_name = COALESCE($4, first_name),
            last_name = COALESCE($5, last_name),
            phone = COALESCE($6, phone),
            avatar_url = COALESCE($7, avatar_url),
            is_active = COALESCE($8, is_active)
         WHERE id = $1 RETURNING {USER_COLS}"
    )))
    .bind(id)
    .bind(&body.email)
    .bind(body.role)
    .bind(&body.first_name)
    .bind(&body.last_name)
    .bind(&body.phone)
    .bind(&body.avatar_url)
    .bind(body.is_active)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(user))
}

async fn update_me(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<UpdateMe>,
) -> AppResult<Json<UserView>> {
    body.validate()?;

    if body.new_password.is_some() {
        let current = body
            .current_password
            .as_deref()
            .ok_or(AppError::BadRequest("current_password is required".into()))?;
        let hash: String =
            sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
                .bind(auth.id)
                .fetch_one(&state.db)
                .await?;
        if !verify_password(current, &hash) {
            return Err(AppError::Unauthorized);
        }
        sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
            .bind(auth.id)
            .bind(hash_password(body.new_password.as_deref().unwrap())?)
            .execute(&state.db)
            .await?;
        sqlx::query("UPDATE refresh_tokens SET revoked = TRUE WHERE user_id = $1")
            .bind(auth.id)
            .execute(&state.db)
            .await?;
    }

    let user = sqlx::query_as::<_, UserView>(sqlx::AssertSqlSafe(format!(
        "UPDATE users SET
            first_name = COALESCE($2, first_name),
            last_name = COALESCE($3, last_name),
            phone = COALESCE($4, phone),
            avatar_url = COALESCE($5, avatar_url)
         WHERE id = $1 RETURNING {USER_COLS}"
    )))
    .bind(auth.id)
    .bind(&body.first_name)
    .bind(&body.last_name)
    .bind(&body.phone)
    .bind(&body.avatar_url)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(user))
}

async fn deactivate(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<axum::http::StatusCode> {
    let school: Option<Uuid> = sqlx::query_scalar("SELECT school_id FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound)?;
    can_manage(&auth, school)?;
    if auth.id == id {
        return Err(AppError::BadRequest("cannot deactivate yourself".into()));
    }
    sqlx::query("UPDATE users SET is_active = FALSE WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    sqlx::query("UPDATE refresh_tokens SET revoked = TRUE WHERE user_id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ---- parent <-> child links ----

#[derive(Deserialize)]
pub struct LinkChild {
    student_id: Uuid,
}

async fn link_child(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(parent_id): Path<Uuid>,
    Json(body): Json<LinkChild>,
) -> AppResult<axum::http::StatusCode> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    let sid = auth.scope(None)?;
    sqlx::query(
        "INSERT INTO parent_links (parent_id, student_id)
         SELECT $1, u.id FROM users u
         WHERE u.id = $2 AND u.role = 'student' AND ($3::uuid IS NULL OR u.school_id = $3)
         ON CONFLICT DO NOTHING",
    )
    .bind(parent_id)
    .bind(body.student_id)
    .bind(sid)
    .execute(&state.db)
    .await?;
    Ok(axum::http::StatusCode::CREATED)
}

async fn list_children(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(parent_id): Path<Uuid>,
) -> AppResult<Json<Vec<UserView>>> {
    if auth.id != parent_id {
        auth.require(&[Role::Superadmin, Role::Admin])?;
    }
    let children = sqlx::query_as::<_, UserView>(sqlx::AssertSqlSafe(format!(
        "SELECT {USER_COLS} FROM users u
         JOIN parent_links pl ON pl.student_id = u.id
         WHERE pl.parent_id = $1"
    )))
    .bind(parent_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(children))
}

async fn my_children(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<UserView>>> {
    auth.require(&[Role::Parent])?;
    let children = sqlx::query_as::<_, UserView>(sqlx::AssertSqlSafe(format!(
        "SELECT {USER_COLS} FROM users u
         JOIN parent_links pl ON pl.student_id = u.id
         WHERE pl.parent_id = $1"
    )))
    .bind(auth.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(children))
}

async fn unlink_child(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((parent_id, student_id)): Path<(Uuid, Uuid)>,
) -> AppResult<axum::http::StatusCode> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    sqlx::query("DELETE FROM parent_links WHERE parent_id = $1 AND student_id = $2")
        .bind(parent_id)
        .bind(student_id)
        .execute(&state.db)
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/me", axum::routing::patch(update_me))
        .route("/me/children", get(my_children))
        .route("/{id}", get(get_one).patch(update).delete(deactivate))
        .route(
            "/{id}/children",
            get(list_children).post(link_child),
        )
        .route("/{id}/children/{student_id}", axum::routing::delete(unlink_child))
}
