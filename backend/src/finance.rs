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
pub struct Charge {
    id: Uuid,
    student_id: Uuid,
    amount: f64,
    currency: String,
    title: String,
    due_date: Option<NaiveDate>,
    status: String,
    created_at: DateTime<Utc>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Payment {
    id: Uuid,
    charge_id: Option<Uuid>,
    student_id: Uuid,
    amount: f64,
    method: String,
    note: Option<String>,
    recorded_by: Option<Uuid>,
    paid_at: DateTime<Utc>,
}

#[derive(Deserialize, Validate)]
pub struct CreateCharge {
    student_id: Uuid,
    amount: f64,
    currency: Option<String>,
    #[validate(length(min = 1, max = 300))]
    title: String,
    due_date: Option<NaiveDate>,
}

#[derive(Deserialize)]
pub struct ChargesQuery {
    student_id: Option<Uuid>,
    status: Option<String>,
}

async fn create_charge(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateCharge>,
) -> AppResult<Json<Charge>> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    body.validate()?;
    if body.amount <= 0.0 {
        return Err(AppError::BadRequest("amount must be positive".into()));
    }
    let c = sqlx::query_as::<_, Charge>(
        "INSERT INTO charges (id, school_id, student_id, amount, currency, title, due_date, created_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
         RETURNING id, student_id, amount::float8 AS amount, currency, title, due_date, status, created_at",
    )
    .bind(Uuid::new_v4())
    .bind(auth.school()?)
    .bind(body.student_id)
    .bind(body.amount)
    .bind(body.currency.as_deref().unwrap_or("UAH"))
    .bind(&body.title)
    .bind(body.due_date)
    .bind(auth.id)
    .fetch_one(&state.db)
    .await?;
    crate::notifications::notify_student_and_parents(
        &state,
        body.student_id,
        "charge",
        serde_json::json!({ "charge_id": c.id, "title": c.title, "amount": c.amount }),
    )
    .await;
    Ok(Json(c))
}

async fn list_charges(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<ChargesQuery>,
) -> AppResult<Json<Vec<Charge>>> {
    let rows = match auth.role {
        Role::Superadmin | Role::Admin => {
            sqlx::query_as::<_, Charge>(
                "SELECT id, student_id, amount::float8 AS amount, currency, title, due_date, status, created_at
                 FROM charges WHERE school_id = $1
                   AND ($2::uuid IS NULL OR student_id = $2)
                   AND ($3::text IS NULL OR status = $3)
                 ORDER BY created_at DESC LIMIT 500",
            )
            .bind(auth.school_id)
            .bind(q.student_id)
            .bind(&q.status)
            .fetch_all(&state.db)
            .await?
        }
        Role::Student => {
            sqlx::query_as::<_, Charge>(
                "SELECT id, student_id, amount::float8 AS amount, currency, title, due_date, status, created_at
                 FROM charges WHERE student_id = $1 ORDER BY created_at DESC LIMIT 500",
            )
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Parent => {
            sqlx::query_as::<_, Charge>(
                "SELECT c.id, c.student_id, c.amount::float8 AS amount, c.currency, c.title, c.due_date, c.status, c.created_at
                 FROM charges c JOIN parent_links pl ON pl.student_id = c.student_id
                 WHERE pl.parent_id = $1 ORDER BY c.created_at DESC LIMIT 500",
            )
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Teacher => return Err(AppError::Forbidden),
    };
    Ok(Json(rows))
}

async fn cancel_charge(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Charge>> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    let c = sqlx::query_as::<_, Charge>(
        "UPDATE charges SET status = 'cancelled'
         WHERE id = $1 AND school_id = $2 AND status = 'open'
         RETURNING id, student_id, amount::float8 AS amount, currency, title, due_date, status, created_at",
    )
    .bind(id)
    .bind(auth.school()?)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(c))
}

#[derive(Deserialize, Validate)]
pub struct CreatePayment {
    charge_id: Option<Uuid>,
    student_id: Uuid,
    amount: f64,
    method: Option<String>,
    note: Option<String>,
}

async fn create_payment(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreatePayment>,
) -> AppResult<Json<Payment>> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    body.validate()?;
    if body.amount <= 0.0 {
        return Err(AppError::BadRequest("amount must be positive".into()));
    }

    let mut tx = state.db.begin().await?;
    let p = sqlx::query_as::<_, Payment>(
        "INSERT INTO payments (id, school_id, charge_id, student_id, amount, method, note, recorded_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
         RETURNING id, charge_id, student_id, amount::float8 AS amount, method, note, recorded_by, paid_at",
    )
    .bind(Uuid::new_v4())
    .bind(auth.school()?)
    .bind(body.charge_id)
    .bind(body.student_id)
    .bind(body.amount)
    .bind(body.method.as_deref().unwrap_or("cash"))
    .bind(&body.note)
    .bind(auth.id)
    .fetch_one(&mut *tx)
    .await?;

    // auto-close the charge when its payments cover the amount
    if let Some(cid) = body.charge_id {
        sqlx::query(
            "UPDATE charges SET status = 'paid'
             WHERE id = $1 AND status = 'open' AND (
                SELECT COALESCE(SUM(amount),0) FROM payments WHERE charge_id = $1
             ) >= amount",
        )
        .bind(cid)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Json(p))
}

async fn list_payments(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<ChargesQuery>,
) -> AppResult<Json<Vec<Payment>>> {
    let rows = match auth.role {
        Role::Superadmin | Role::Admin => {
            sqlx::query_as::<_, Payment>(
                "SELECT id, charge_id, student_id, amount::float8 AS amount, method, note, recorded_by, paid_at
                 FROM payments WHERE school_id = $1
                   AND ($2::uuid IS NULL OR student_id = $2)
                 ORDER BY paid_at DESC LIMIT 500",
            )
            .bind(auth.school_id)
            .bind(q.student_id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Student => {
            sqlx::query_as::<_, Payment>(
                "SELECT id, charge_id, student_id, amount::float8 AS amount, method, note, recorded_by, paid_at
                 FROM payments WHERE student_id = $1 ORDER BY paid_at DESC LIMIT 500",
            )
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Parent => {
            sqlx::query_as::<_, Payment>(
                "SELECT p.id, p.charge_id, p.student_id, p.amount::float8 AS amount, p.method, p.note, p.recorded_by, p.paid_at
                 FROM payments p JOIN parent_links pl ON pl.student_id = p.student_id
                 WHERE pl.parent_id = $1 ORDER BY p.paid_at DESC LIMIT 500",
            )
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Teacher => return Err(AppError::Forbidden),
    };
    Ok(Json(rows))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct DebtRow {
    student_id: Uuid,
    first_name: String,
    last_name: String,
    open_charges: i64,
    debt_total: f64,
}

/// Students with open charges — the debt list for the school office.
async fn debts(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Vec<DebtRow>>> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    let rows = sqlx::query_as::<_, DebtRow>(
        "SELECT u.id AS student_id, u.first_name, u.last_name,
                COUNT(c.id) AS open_charges, COALESCE(SUM(c.amount),0)::float8 AS debt_total
         FROM users u
         JOIN charges c ON c.student_id = u.id AND c.status = 'open'
         WHERE u.school_id = $1 AND u.role = 'student'
         GROUP BY u.id, u.first_name, u.last_name
         ORDER BY debt_total DESC",
    )
    .bind(auth.school()?)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

#[derive(Serialize)]
pub struct FinanceSummary {
    total_charged: f64,
    total_paid: f64,
    total_open: f64,
    payments_count: i64,
}

async fn summary(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<FinanceSummary>> {
    auth.require(&[Role::Superadmin, Role::Admin])?;
    let sid = auth.school()?;
    let (charged, open): (f64, f64) = sqlx::query_as(
        "SELECT COALESCE(SUM(amount),0)::float8,
                COALESCE(SUM(amount) FILTER (WHERE status='open'),0)::float8
         FROM charges WHERE school_id = $1",
    )
    .bind(sid)
    .fetch_one(&state.db)
    .await?;
    let (paid, cnt): (f64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(amount),0)::float8, COUNT(*) FROM payments WHERE school_id = $1",
    )
    .bind(sid)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(FinanceSummary {
        total_charged: charged,
        total_paid: paid,
        total_open: open,
        payments_count: cnt,
    }))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/charges", get(list_charges).post(create_charge))
        .route("/charges/{id}/cancel", axum::routing::patch(cancel_charge))
        .route("/payments", get(list_payments).post(create_payment))
        .route("/debts", get(debts))
        .route("/summary", get(summary))
}
