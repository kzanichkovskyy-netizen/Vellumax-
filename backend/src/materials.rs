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

use crate::AppState;
use crate::auth::AuthUser;
use crate::classes::can_view_class;
use crate::error::{AppError, AppResult};
use crate::types::Role;

#[derive(Serialize, sqlx::FromRow)]
pub struct Material {
    id: Uuid,
    school_id: Uuid,
    class_id: Option<Uuid>,
    subject_id: Option<Uuid>,
    uploader_id: Uuid,
    filename: String,
    mime: Option<String>,
    size: i64,
    created_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct ListQuery {
    class_id: Option<Uuid>,
    subject_id: Option<Uuid>,
}

fn sanitize_filename(name: &str) -> String {
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

async fn upload(
    auth: AuthUser,
    State(state): State<AppState>,
    mut mp: Multipart,
) -> AppResult<Response> {
    auth.require(&[Role::Superadmin, Role::Admin, Role::Teacher])?;
    let sid = auth.school()?;

    let mut class_id: Option<Uuid> = None;
    let mut subject_id: Option<Uuid> = None;
    let mut saved: Option<(String, String, Option<String>, i64)> = None;

    while let Some(field) = mp
        .next_field()
        .await
        .map_err(|_| AppError::BadRequest("invalid multipart".into()))?
    {
        match field.name() {
            Some("class_id") => {
                class_id = field
                    .text()
                    .await
                    .ok()
                    .and_then(|t| Uuid::parse_str(&t).ok());
            }
            Some("subject_id") => {
                subject_id = field
                    .text()
                    .await
                    .ok()
                    .and_then(|t| Uuid::parse_str(&t).ok());
            }
            Some("file") => {
                let filename = sanitize_filename(field.file_name().unwrap_or("file"));
                let mime = field.content_type().map(|m| m.to_string());
                let stored = format!("{}_{}", Uuid::new_v4(), filename);
                let path = format!("{}/{}", state.config.upload_dir, stored);
                let data = field
                    .bytes()
                    .await
                    .map_err(|_| AppError::BadRequest("failed to read file".into()))?;
                tokio::fs::write(&path, &data)
                    .await
                    .map_err(|_| AppError::BadRequest("failed to store file".into()))?;
                saved = Some((filename, stored, mime, data.len() as i64));
            }
            _ => {}
        }
    }

    let (filename, stored, mime, size) =
        saved.ok_or(AppError::BadRequest("missing 'file' field".into()))?;
    if let Some(cid) = class_id {
        can_view_class(&state, &auth, cid).await?;
    }

    let m = sqlx::query_as::<_, Material>(
        "INSERT INTO materials (id, school_id, class_id, subject_id, uploader_id, filename, stored_name, mime, size)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
         RETURNING id, school_id, class_id, subject_id, uploader_id, filename, mime, size, created_at",
    )
    .bind(Uuid::new_v4())
    .bind(sid)
    .bind(class_id)
    .bind(subject_id)
    .bind(auth.id)
    .bind(&filename)
    .bind(&stored)
    .bind(&mime)
    .bind(size)
    .fetch_one(&state.db)
    .await?;
    Ok((StatusCode::CREATED, Json(m)).into_response())
}

async fn list(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<Vec<Material>>> {
    let rows = match auth.role {
        Role::Superadmin | Role::Admin | Role::Teacher => {
            sqlx::query_as::<_, Material>(
                "SELECT id, school_id, class_id, subject_id, uploader_id, filename, mime, size, created_at
                 FROM materials WHERE school_id = $1
                   AND ($2::uuid IS NULL OR class_id = $2)
                   AND ($3::uuid IS NULL OR subject_id = $3)
                 ORDER BY created_at DESC LIMIT 200",
            )
            .bind(auth.school_id)
            .bind(q.class_id)
            .bind(q.subject_id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Student => {
            sqlx::query_as::<_, Material>(
                "SELECT m.id, m.school_id, m.class_id, m.subject_id, m.uploader_id,
                        m.filename, m.mime, m.size, m.created_at
                 FROM materials m
                 WHERE m.school_id = $1 AND (
                    m.class_id IS NULL OR EXISTS(
                        SELECT 1 FROM enrollments e
                        WHERE e.class_id = m.class_id AND e.student_id = $2))
                 ORDER BY m.created_at DESC LIMIT 200",
            )
            .bind(auth.school_id)
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
        Role::Parent => {
            sqlx::query_as::<_, Material>(
                "SELECT m.id, m.school_id, m.class_id, m.subject_id, m.uploader_id,
                        m.filename, m.mime, m.size, m.created_at
                 FROM materials m
                 WHERE m.school_id = $1 AND (
                    m.class_id IS NULL OR EXISTS(
                        SELECT 1 FROM enrollments e
                        JOIN parent_links pl ON pl.student_id = e.student_id
                        WHERE e.class_id = m.class_id AND pl.parent_id = $2))
                 ORDER BY m.created_at DESC LIMIT 200",
            )
            .bind(auth.school_id)
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?
        }
    };
    Ok(Json(rows))
}

async fn can_access_material(
    state: &AppState,
    auth: &AuthUser,
    school_id: Uuid,
    class_id: Option<Uuid>,
) -> bool {
    match auth.role {
        Role::Superadmin => true,
        _ if auth.school_id != Some(school_id) => false,
        Role::Admin | Role::Teacher => true,
        _ => match class_id {
            None => true,
            Some(cid) => can_view_class(state, auth, cid).await.is_ok(),
        },
    }
}

#[derive(sqlx::FromRow)]
struct MaterialFile {
    school_id: Uuid,
    class_id: Option<Uuid>,
    filename: String,
    stored_name: String,
    mime: Option<String>,
}

async fn download(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Response> {
    let f = sqlx::query_as::<_, MaterialFile>(
        "SELECT school_id, class_id, filename, stored_name, mime FROM materials WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    let MaterialFile {
        school_id,
        class_id,
        filename,
        stored_name: stored,
        mime,
    } = f;
    if !can_access_material(&state, &auth, school_id, class_id).await {
        return Err(AppError::Forbidden);
    }
    let path = format!("{}/{}", state.config.upload_dir, stored);
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| AppError::NotFound)?;
    let stream = ReaderStream::new(file);
    let disposition = format!("attachment; filename=\"{}\"", filename);
    Ok((
        [
            (
                header::CONTENT_TYPE,
                mime.unwrap_or_else(|| "application/octet-stream".into()),
            ),
            (header::CONTENT_DISPOSITION, disposition),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}

async fn delete(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    let row: Option<(Uuid, Option<String>, Uuid)> =
        sqlx::query_as("SELECT school_id, stored_name, uploader_id FROM materials WHERE id = $1")
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
    sqlx::query("DELETE FROM materials WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    if let Some(stored) = stored {
        let _ = tokio::fs::remove_file(format!("{}/{}", state.config.upload_dir, stored)).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(upload))
        .route("/{id}/download", get(download))
        .route("/{id}", axum::routing::delete(delete))
}
