use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{Duration, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use validator::Validate;

use crate::AppState;
use crate::error::{AppError, AppResult};
use crate::types::Role;

#[derive(sqlx::FromRow)]
struct UserRow {
    id: Uuid,
    school_id: Option<Uuid>,
    password_hash: String,
    role: Role,
    is_active: bool,
}

#[derive(Serialize, Deserialize)]
struct Claims {
    sub: String,
    sid: Option<Uuid>,
    role: Role,
    iat: usize,
    exp: usize,
}

#[derive(Clone)]
pub struct AuthUser {
    pub id: Uuid,
    pub school_id: Option<Uuid>,
    pub role: Role,
}

impl AuthUser {
    pub fn require(&self, roles: &[Role]) -> AppResult<()> {
        if roles.contains(&self.role) {
            Ok(())
        } else {
            Err(AppError::Forbidden)
        }
    }

    /// School id the caller is bound to. Superadmin gets an error — use
    /// `effective_school` for endpoints where superadmin may pick a school.
    pub fn school(&self) -> AppResult<Uuid> {
        self.school_id.ok_or(AppError::Forbidden)
    }

    /// Tenant scope for queries: own school for bound roles,
    /// explicit `requested` or `None` (all schools) for superadmin.
    pub fn scope(&self, requested: Option<Uuid>) -> AppResult<Option<Uuid>> {
        match self.role {
            Role::Superadmin => Ok(requested),
            _ => Ok(Some(self.school()?)),
        }
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or(AppError::Unauthorized)?;
        let claims = decode_access(&state.config.jwt_secret, token)?;
        Ok(AuthUser {
            id: Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?,
            school_id: claims.sid,
            role: claims.role,
        })
    }
}

pub fn hash_password(password: &str) -> AppResult<String> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|_| AppError::BadRequest("could not hash password".into()))
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|h| {
            Argon2::default()
                .verify_password(password.as_bytes(), &h)
                .is_ok()
        })
        .unwrap_or(false)
}

fn encode_access(state: &AppState, user: &UserRow) -> AppResult<String> {
    let now = Utc::now();
    let claims = Claims {
        sub: user.id.to_string(),
        sid: user.school_id,
        role: user.role,
        iat: now.timestamp() as usize,
        exp: (now.timestamp() + state.config.access_token_ttl_secs) as usize,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(state.config.jwt_secret.as_bytes()),
    )
    .map_err(|_| AppError::Unauthorized)
}

fn decode_access(secret: &str, token: &str) -> AppResult<Claims> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map(|d| d.claims)
    .map_err(|_| AppError::Unauthorized)
}

fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

#[derive(Serialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    token_type: &'static str,
    expires_in: i64,
}

async fn issue_tokens(state: &AppState, user: &UserRow) -> AppResult<TokenResponse> {
    let access = encode_access(state, user)?;

    let refresh = hex::encode(rand::random::<[u8; 32]>());

    sqlx::query(
        "INSERT INTO refresh_tokens (id, user_id, token_hash, expires_at) VALUES ($1,$2,$3,$4)",
    )
    .bind(Uuid::new_v4())
    .bind(user.id)
    .bind(token_hash(&refresh))
    .bind(Utc::now() + Duration::days(state.config.refresh_token_ttl_days))
    .execute(&state.db)
    .await?;

    Ok(TokenResponse {
        access_token: access,
        refresh_token: refresh,
        token_type: "Bearer",
        expires_in: state.config.access_token_ttl_secs,
    })
}

#[derive(Deserialize, Validate)]
struct LoginRequest {
    #[validate(email)]
    email: String,
    password: String,
}

async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> AppResult<Json<TokenResponse>> {
    body.validate()?;
    let user = sqlx::query_as::<_, UserRow>(
        "SELECT id, school_id, password_hash, role, is_active FROM users WHERE email = $1",
    )
    .bind(&body.email)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::Unauthorized)?;

    if !user.is_active || !verify_password(&body.password, &user.password_hash) {
        return Err(AppError::Unauthorized);
    }
    Ok(Json(issue_tokens(&state, &user).await?))
}

#[derive(Deserialize)]
struct RefreshRequest {
    refresh_token: String,
}

async fn refresh(
    State(state): State<AppState>,
    Json(body): Json<RefreshRequest>,
) -> AppResult<Json<TokenResponse>> {
    let hash = token_hash(&body.refresh_token);
    let user_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM refresh_tokens WHERE token_hash = $1 AND NOT revoked AND expires_at > now()",
    )
    .bind(&hash)
    .fetch_optional(&state.db)
    .await?;
    let user_id = user_id.ok_or(AppError::Unauthorized)?;

    sqlx::query("UPDATE refresh_tokens SET revoked = TRUE WHERE token_hash = $1")
        .bind(&hash)
        .execute(&state.db)
        .await?;

    let user = sqlx::query_as::<_, UserRow>(
        "SELECT id, school_id, password_hash, role, is_active FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    .filter(|u| u.is_active)
    .ok_or(AppError::Unauthorized)?;

    Ok(Json(issue_tokens(&state, &user).await?))
}

async fn logout(
    State(state): State<AppState>,
    Json(body): Json<RefreshRequest>,
) -> AppResult<StatusCode> {
    sqlx::query("UPDATE refresh_tokens SET revoked = TRUE WHERE token_hash = $1")
        .bind(token_hash(&body.refresh_token))
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

use axum::http::StatusCode;

#[derive(Serialize, sqlx::FromRow)]
struct MeResponse {
    id: Uuid,
    school_id: Option<Uuid>,
    email: String,
    role: Role,
    first_name: String,
    last_name: String,
}

async fn me(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<MeResponse>> {
    let me = sqlx::query_as::<_, MeResponse>(
        "SELECT id, school_id, email, role, first_name, last_name FROM users WHERE id = $1",
    )
    .bind(auth.id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(me))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/login", post(login))
        .route("/refresh", post(refresh))
        .route("/logout", post(logout))
        .route("/me", get(me))
}
