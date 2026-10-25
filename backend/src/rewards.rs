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
pub struct Reward {
    id: Uuid,
    student_id: Uuid,
    points: i32,
    reason: String,
    given_by: Option<Uuid>,
    created_at: DateTime<Utc>,
}

#[derive(Deserialize, Validate)]
pub struct GiveReward {
    student_id: Uuid,
    points: i32,
    #[validate(length(min = 1, max = 300))]
    reason: String,
}

async fn give(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<GiveReward>,
) -> AppResult<Json<Reward>> {
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    body.validate()?;
    if body.points == 0 || body.points.abs() > 100 {
        return Err(AppError::BadRequest(
            "points must be 1..100 or -100..-1".into(),
        ));
    }
    // teacher may only reward students from their classes
    if auth.role == Role::Teacher {
        let ok: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1 FROM enrollments e
                JOIN class_subject_teachers cst ON cst.class_id = e.class_id
                WHERE e.student_id = $1 AND cst.teacher_id = $2)
             OR EXISTS(
                SELECT 1 FROM enrollments e
                JOIN classes c ON c.id = e.class_id
                WHERE e.student_id = $1 AND c.homeroom_teacher_id = $2)",
        )
        .bind(body.student_id)
        .bind(auth.id)
        .fetch_one(&state.db)
        .await?;
        if !ok {
            return Err(AppError::Forbidden);
        }
    }
    let r = sqlx::query_as::<_, Reward>(
        "INSERT INTO reward_points (id, school_id, student_id, points, reason, given_by)
         VALUES ($1,$2,$3,$4,$5,$6)
         RETURNING id, student_id, points, reason, given_by, created_at",
    )
    .bind(Uuid::new_v4())
    .bind(auth.school()?)
    .bind(body.student_id)
    .bind(body.points)
    .bind(&body.reason)
    .bind(auth.id)
    .fetch_one(&state.db)
    .await?;
    crate::notifications::notify_student_and_parents(
        &state,
        body.student_id,
        "reward",
        serde_json::json!({ "points": body.points, "reason": body.reason }),
    )
    .await;
    Ok(Json(r))
}

async fn student_rewards(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(student_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
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

    let balance: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(points),0) FROM reward_points WHERE student_id = $1",
    )
    .bind(student_id)
    .fetch_one(&state.db)
    .await?;
    let history = sqlx::query_as::<_, Reward>(
        "SELECT id, student_id, points, reason, given_by, created_at
         FROM reward_points WHERE student_id = $1 ORDER BY created_at DESC LIMIT 200",
    )
    .bind(student_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(serde_json::json!({
        "balance": balance,
        "history": history,
    })))
}

/// Class leaderboard.
async fn leaderboard(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(class_id): Path<Uuid>,
) -> AppResult<Json<Vec<serde_json::Value>>> {
    crate::classes::can_view_class(&state, &auth, class_id).await?;
    let rows: Vec<(Uuid, String, String, i64)> = sqlx::query_as(
        "SELECT u.id, u.first_name, u.last_name, COALESCE(SUM(r.points),0) AS total
         FROM users u
         JOIN enrollments e ON e.student_id = u.id
         LEFT JOIN reward_points r ON r.student_id = u.id
         WHERE e.class_id = $1
         GROUP BY u.id, u.first_name, u.last_name
         ORDER BY total DESC",
    )
    .bind(class_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, first, last, total)| {
                serde_json::json!({
                    "student_id": id, "first_name": first, "last_name": last, "total": total,
                })
            })
            .collect(),
    ))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", axum::routing::post(give))
        .route("/students/{id}", get(student_rewards))
        .route("/classes/{id}/leaderboard", get(leaderboard))
}
