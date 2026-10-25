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
pub struct Route {
    id: Uuid,
    name: String,
    driver_name: Option<String>,
    driver_phone: Option<String>,
    stops: serde_json::Value,
    created_at: DateTime<Utc>,
}

#[derive(Deserialize, Validate)]
pub struct UpsertRoute {
    #[validate(length(min = 1, max = 200))]
    name: String,
    driver_name: Option<String>,
    driver_phone: Option<String>,
    /// ordered list of stop names
    stops: Option<Vec<String>>,
}

async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<UpsertRoute>,
) -> AppResult<Json<Route>> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    body.validate()?;
    let r = sqlx::query_as::<_, Route>(
        "INSERT INTO transport_routes (id, school_id, name, driver_name, driver_phone, stops)
         VALUES ($1,$2,$3,$4,$5,$6)
         RETURNING id, name, driver_name, driver_phone, stops, created_at",
    )
    .bind(Uuid::new_v4())
    .bind(auth.school()?)
    .bind(&body.name)
    .bind(&body.driver_name)
    .bind(&body.driver_phone)
    .bind(serde_json::json!(body.stops.unwrap_or_default()))
    .fetch_one(&state.db)
    .await?;
    Ok(Json(r))
}

async fn list(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Vec<Route>>> {
    let rows = sqlx::query_as::<_, Route>(
        "SELECT id, name, driver_name, driver_phone, stops, created_at
         FROM transport_routes WHERE school_id = $1 ORDER BY name",
    )
    .bind(auth.school()?)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

async fn update(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpsertRoute>,
) -> AppResult<Json<Route>> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    body.validate()?;
    let r = sqlx::query_as::<_, Route>(
        "UPDATE transport_routes SET name = $3, driver_name = $4, driver_phone = $5,
            stops = COALESCE($6, stops)
         WHERE id = $1 AND school_id = $2
         RETURNING id, name, driver_name, driver_phone, stops, created_at",
    )
    .bind(id)
    .bind(auth.school()?)
    .bind(&body.name)
    .bind(&body.driver_name)
    .bind(&body.driver_phone)
    .bind(body.stops.as_ref().map(|s| serde_json::json!(s)))
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(r))
}

async fn delete(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<axum::http::StatusCode> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    let res = sqlx::query("DELETE FROM transport_routes WHERE id = $1 AND school_id = $2")
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
pub struct AssignStudent {
    student_id: Uuid,
    stop: Option<String>,
}

async fn assign_student(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<AssignStudent>,
) -> AppResult<axum::http::StatusCode> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    sqlx::query(
        "INSERT INTO route_students (route_id, student_id, stop)
         SELECT $1, u.id, $3 FROM users u
         WHERE u.id = $2 AND u.role = 'student' AND u.school_id = $4
         ON CONFLICT (route_id, student_id) DO UPDATE SET stop = EXCLUDED.stop",
    )
    .bind(id)
    .bind(body.student_id)
    .bind(&body.stop)
    .bind(auth.school()?)
    .execute(&state.db)
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn unassign_student(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((id, student_id)): Path<(Uuid, Uuid)>,
) -> AppResult<axum::http::StatusCode> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    sqlx::query("DELETE FROM route_students WHERE route_id = $1 AND student_id = $2")
        .bind(id)
        .bind(student_id)
        .execute(&state.db)
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Serialize, sqlx::FromRow)]
pub struct RouteStudent {
    student_id: Uuid,
    first_name: String,
    last_name: String,
    stop: Option<String>,
}

async fn route_students(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Vec<RouteStudent>>> {
    // admin/staff see all; parent sees only own children's entries
    let rows = match auth.role {
        Role::Superadmin | Role::Admin | Role::Teacher => {
            sqlx::query_as::<_, RouteStudent>(
                "SELECT rs.student_id, u.first_name, u.last_name, rs.stop
                 FROM route_students rs JOIN users u ON u.id = rs.student_id
                 WHERE rs.route_id = $1 ORDER BY u.last_name",
            )
            .bind(id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Parent => {
            sqlx::query_as::<_, RouteStudent>(
                "SELECT rs.student_id, u.first_name, u.last_name, rs.stop
                 FROM route_students rs
                 JOIN users u ON u.id = rs.student_id
                 JOIN parent_links pl ON pl.student_id = rs.student_id
                 WHERE rs.route_id = $1 AND pl.parent_id = $2",
            )
            .bind(id)
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Student => {
            sqlx::query_as::<_, RouteStudent>(
                "SELECT rs.student_id, u.first_name, u.last_name, rs.stop
                 FROM route_students rs JOIN users u ON u.id = rs.student_id
                 WHERE rs.route_id = $1 AND rs.student_id = $2",
            )
            .bind(id)
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
    };
    Ok(Json(rows))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", axum::routing::patch(update).delete(delete))
        .route("/{id}/students", get(route_students).post(assign_student))
        .route(
            "/{id}/students/{student_id}",
            axum::routing::delete(unassign_student),
        )
}
