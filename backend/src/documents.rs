use axum::body::Body;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio_util::io::ReaderStream;
use uuid::Uuid;
use validator::Validate;

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::types::Role;

#[derive(Serialize, sqlx::FromRow)]
pub struct Document {
    id: Uuid,
    user_id: Option<Uuid>,
    uploader_id: Uuid,
    title: String,
    doc_type: String,
    filename: String,
    created_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct DocQuery {
    user_id: Option<Uuid>,
    doc_type: Option<String>,
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(120)
        .collect()
}

async fn can_view_user_docs(
    state: &AppState,
    auth: &AuthUser,
    user_id: Option<Uuid>,
) -> AppResult<()> {
    let ok = match auth.role {
        Role::Superadmin | Role::Admin => true,
        _ => match user_id {
            None => true, // school-level documents
            Some(uid) if uid == auth.id => true,
            Some(uid) if auth.role == Role::Parent => sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM parent_links WHERE parent_id=$1 AND student_id=$2)",
            )
            .bind(auth.id)
            .bind(uid)
            .fetch_one(&state.db)
            .await?,
            _ => false,
        },
    };
    if ok { Ok(()) } else { Err(AppError::Forbidden) }
}

async fn upload(
    auth: AuthUser,
    State(state): State<AppState>,
    mut mp: Multipart,
) -> AppResult<Response> {
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    let sid = auth.school()?;

    let mut user_id: Option<Uuid> = None;
    let mut title: Option<String> = None;
    let mut doc_type = "other".to_string();
    let mut saved: Option<(String, String)> = None;

    while let Some(field) = mp
        .next_field()
        .await
        .map_err(|_| AppError::BadRequest("invalid multipart".into()))?
    {
        match field.name() {
            Some("user_id") => {
                user_id = field
                    .text()
                    .await
                    .ok()
                    .and_then(|t| Uuid::parse_str(&t).ok())
            }
            Some("title") => title = field.text().await.ok(),
            Some("doc_type") => doc_type = field.text().await.unwrap_or_else(|_| "other".into()),
            Some("file") => {
                let filename = sanitize(field.file_name().unwrap_or("file"));
                let stored = format!("doc_{}_{}", Uuid::new_v4(), filename);
                let path = format!("{}/{}", state.config.upload_dir, stored);
                let data = field
                    .bytes()
                    .await
                    .map_err(|_| AppError::BadRequest("failed to read file".into()))?;
                tokio::fs::write(&path, &data)
                    .await
                    .map_err(|_| AppError::BadRequest("failed to store file".into()))?;
                saved = Some((filename, stored));
            }
            _ => {}
        }
    }

    let (filename, stored) = saved.ok_or(AppError::BadRequest("missing 'file' field".into()))?;
    let title = title.ok_or(AppError::BadRequest("missing 'title' field".into()))?;
    Validate::validate(&CreateDocTitle {
        title: title.clone(),
    })?;

    let d = sqlx::query_as::<_, Document>(
        "INSERT INTO documents (id, school_id, user_id, uploader_id, title, doc_type, filename, stored_name)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
         RETURNING id, user_id, uploader_id, title, doc_type, filename, created_at",
    )
    .bind(Uuid::new_v4())
    .bind(sid)
    .bind(user_id)
    .bind(auth.id)
    .bind(&title)
    .bind(&doc_type)
    .bind(&filename)
    .bind(&stored)
    .fetch_one(&state.db)
    .await?;
    Ok((StatusCode::CREATED, Json(d)).into_response())
}

#[derive(Validate)]
struct CreateDocTitle {
    #[validate(length(min = 1, max = 300))]
    title: String,
}

async fn list(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<DocQuery>,
) -> AppResult<Json<Vec<Document>>> {
    can_view_user_docs(&state, &auth, q.user_id).await?;
    let rows = match auth.role {
        Role::Superadmin | Role::Admin | Role::Teacher => {
            sqlx::query_as::<_, Document>(
                "SELECT id, user_id, uploader_id, title, doc_type, filename, created_at
                 FROM documents WHERE school_id = $1
                   AND ($2::uuid IS NULL OR user_id = $2)
                   AND ($3::text IS NULL OR doc_type = $3)
                 ORDER BY created_at DESC LIMIT 300",
            )
            .bind(auth.school_id)
            .bind(q.user_id)
            .bind(&q.doc_type)
            .fetch_all(&state.db)
            .await?
        }
        _ => {
            sqlx::query_as::<_, Document>(
                "SELECT id, user_id, uploader_id, title, doc_type, filename, created_at
                 FROM documents WHERE school_id = $1
                   AND ($2::uuid IS NULL OR user_id = $2)
                   AND ($3::text IS NULL OR doc_type = $3)
                   AND (user_id IS NULL OR user_id = $4 OR EXISTS(
                        SELECT 1 FROM parent_links pl
                        WHERE pl.student_id = documents.user_id AND pl.parent_id = $4))
                 ORDER BY created_at DESC LIMIT 300",
            )
            .bind(auth.school_id)
            .bind(q.user_id)
            .bind(&q.doc_type)
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
    };
    Ok(Json(rows))
}

async fn download(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Response> {
    #[derive(sqlx::FromRow)]
    struct DocFile {
        school_id: Uuid,
        user_id: Option<Uuid>,
        filename: String,
        stored_name: String,
    }
    let d = sqlx::query_as::<_, DocFile>(
        "SELECT school_id, user_id, filename, stored_name FROM documents WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    if auth.school_id != Some(d.school_id) && auth.role != Role::Superadmin {
        return Err(AppError::Forbidden);
    }
    can_view_user_docs(&state, &auth, d.user_id).await?;

    let path = format!("{}/{}", state.config.upload_dir, d.stored_name);
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| AppError::NotFound)?;
    Ok((
        [(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", d.filename),
        )],
        Body::from_stream(ReaderStream::new(file)),
    )
        .into_response())
}

async fn delete(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    let row: Option<(Uuid, String, Uuid)> =
        sqlx::query_as("SELECT school_id, stored_name, uploader_id FROM documents WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    let (school_id, stored, uploader) = row.ok_or(AppError::NotFound)?;
    let ok = match auth.role {
        Role::Superadmin => true,
        Role::Admin => auth.school_id == Some(school_id),
        Role::Teacher => uploader == auth.id,
        _ => false,
    };
    if !ok {
        return Err(AppError::Forbidden);
    }
    sqlx::query("DELETE FROM documents WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    let _ = tokio::fs::remove_file(format!("{}/{}", state.config.upload_dir, stored)).await;
    Ok(StatusCode::NO_CONTENT)
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(upload))
        .route("/{id}/download", get(download))
        .route("/{id}", axum::routing::delete(delete))
}
