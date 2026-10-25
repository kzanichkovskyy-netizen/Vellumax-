use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use validator::Validate;

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::types::Role;

#[derive(Serialize, sqlx::FromRow)]
pub struct MealMenu {
    id: Uuid,
    date: NaiveDate,
    meal_type: String,
    title: String,
    description: Option<String>,
    price: Option<f64>,
}

#[derive(Deserialize, Validate)]
pub struct UpsertMenu {
    date: NaiveDate,
    meal_type: String,
    #[validate(length(min = 1, max = 300))]
    title: String,
    description: Option<String>,
    price: Option<f64>,
}

#[derive(Deserialize)]
pub struct MenuQuery {
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
}

async fn upsert_menu(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<UpsertMenu>,
) -> AppResult<Json<MealMenu>> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    body.validate()?;
    if !["breakfast", "lunch", "snack", "dinner"].contains(&body.meal_type.as_str()) {
        return Err(AppError::BadRequest("invalid meal_type".into()));
    }
    let m = sqlx::query_as::<_, MealMenu>(
        "INSERT INTO meal_menus (id, school_id, date, meal_type, title, description, price)
         VALUES ($1,$2,$3,$4,$5,$6,$7)
         ON CONFLICT (school_id, date, meal_type)
         DO UPDATE SET title = EXCLUDED.title, description = EXCLUDED.description, price = EXCLUDED.price
         RETURNING id, date, meal_type, title, description, price::float8",
    )
    .bind(Uuid::new_v4())
    .bind(auth.school()?)
    .bind(body.date)
    .bind(&body.meal_type)
    .bind(&body.title)
    .bind(&body.description)
    .bind(body.price)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(m))
}

async fn list_menus(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<MenuQuery>,
) -> AppResult<Json<Vec<MealMenu>>> {
    let from = q.from.unwrap_or_else(|| Utc::now().date_naive());
    let to = q.to.unwrap_or(from + chrono::Duration::days(7));
    let rows = sqlx::query_as::<_, MealMenu>(
        "SELECT id, date, meal_type, title, description, price::float8
         FROM meal_menus WHERE school_id = $1 AND date >= $2 AND date < $3
         ORDER BY date, meal_type",
    )
    .bind(auth.school()?)
    .bind(from)
    .bind(to)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct MealOrder {
    id: Uuid,
    menu_id: Uuid,
    student_id: Uuid,
    ordered_by: Uuid,
    status: String,
    created_at: chrono::DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct CreateOrder {
    menu_id: Uuid,
    student_id: Uuid,
}

/// Parent orders for own child; student orders for self; admin for anyone.
async fn order(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateOrder>,
) -> AppResult<Json<MealOrder>> {
    let ok =
        match auth.role {
            Role::Superadmin | Role::Admin => true,
            Role::Student => auth.id == body.student_id,
            Role::Parent => sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM parent_links WHERE parent_id=$1 AND student_id=$2)",
            )
            .bind(auth.id)
            .bind(body.student_id)
            .fetch_one(&state.db)
            .await?,
            Role::Teacher => false,
        };
    if !ok {
        return Err(AppError::Forbidden);
    }
    let o = sqlx::query_as::<_, MealOrder>(
        "INSERT INTO meal_orders (id, menu_id, student_id, ordered_by)
         VALUES ($1,$2,$3,$4)
         ON CONFLICT (menu_id, student_id) DO UPDATE SET status = 'ordered'
         RETURNING id, menu_id, student_id, ordered_by, status, created_at",
    )
    .bind(Uuid::new_v4())
    .bind(body.menu_id)
    .bind(body.student_id)
    .bind(auth.id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(o))
}

async fn cancel_order(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<MealOrder>> {
    let o = sqlx::query_as::<_, MealOrder>(
        "UPDATE meal_orders SET status = 'cancelled'
         WHERE id = $1 AND (ordered_by = $2 OR $3) AND status = 'ordered'
         RETURNING id, menu_id, student_id, ordered_by, status, created_at",
    )
    .bind(id)
    .bind(auth.id)
    .bind(matches!(auth.role, Role::Admin | Role::Superadmin))
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(o))
}

#[derive(Deserialize)]
pub struct OrdersQuery {
    menu_id: Option<Uuid>,
    student_id: Option<Uuid>,
    date: Option<NaiveDate>,
}

async fn list_orders(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<OrdersQuery>,
) -> AppResult<Json<Vec<MealOrder>>> {
    let rows = match auth.role {
        Role::Superadmin | Role::Admin => {
            sqlx::query_as::<_, MealOrder>(
                "SELECT o.id, o.menu_id, o.student_id, o.ordered_by, o.status, o.created_at
                 FROM meal_orders o JOIN meal_menus m ON m.id = o.menu_id
                 WHERE m.school_id = $1
                   AND ($2::uuid IS NULL OR o.menu_id = $2)
                   AND ($3::uuid IS NULL OR o.student_id = $3)
                   AND ($4::date IS NULL OR m.date = $4)
                 ORDER BY o.created_at DESC LIMIT 500",
            )
            .bind(auth.school_id)
            .bind(q.menu_id)
            .bind(q.student_id)
            .bind(q.date)
            .fetch_all(&state.db)
            .await?
        }
        Role::Student => {
            sqlx::query_as::<_, MealOrder>(
                "SELECT id, menu_id, student_id, ordered_by, status, created_at
                 FROM meal_orders WHERE student_id = $1 ORDER BY created_at DESC LIMIT 100",
            )
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Parent => {
            sqlx::query_as::<_, MealOrder>(
                "SELECT o.id, o.menu_id, o.student_id, o.ordered_by, o.status, o.created_at
                 FROM meal_orders o JOIN parent_links pl ON pl.student_id = o.student_id
                 WHERE pl.parent_id = $1 ORDER BY o.created_at DESC LIMIT 100",
            )
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Teacher => return Err(AppError::Forbidden),
    };
    Ok(Json(rows))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/menus", get(list_menus).post(upsert_menu))
        .route("/orders", get(list_orders).post(order))
        .route("/orders/{id}/cancel", post(cancel_order))
}
