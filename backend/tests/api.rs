//! Integration tests. Require a Postgres database at TEST_DATABASE_URL
//! (falling back to DATABASE_URL). Tests skip silently without one.

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use chrono::Datelike;
use classnode_backend::auth::hash_password;
use classnode_backend::{AppState, build_router, config::Config};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

struct Ctx {
    state: AppState,
    school_a: Uuid,
    school_b: Uuid,
}

async fn ctx() -> Option<Ctx> {
    let url = std::env::var("TEST_DATABASE_URL")
        .ok()
        .or_else(|| std::env::var("DATABASE_URL").ok())?;
    let pool = sqlx::PgPool::connect(&url).await.ok()?;
    sqlx::migrate!("./migrations").run(&pool).await.ok()?;

    let config = Arc::new(Config {
        database_url: url,
        jwt_secret: "test-secret".into(),
        port: 0,
        upload_dir: "uploads".into(),
        access_token_ttl_secs: 3600,
        refresh_token_ttl_days: 30,
    });
    let state = AppState {
        db: pool.clone(),
        config,
    };

    let pw = hash_password("password123").unwrap();
    let school_a = Uuid::new_v4();
    let school_b = Uuid::new_v4();
    for (id, name) in [(school_a, "A"), (school_b, "B")] {
        sqlx::query("INSERT INTO schools (id, name) VALUES ($1, $2)")
            .bind(id)
            .bind(name)
            .execute(&pool)
            .await
            .unwrap();
    }
    let admin_a = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, school_id, email, password_hash, role, first_name, last_name)
         VALUES ($1,$2,$3,$4,'admin','A','Admin')",
    )
    .bind(admin_a)
    .bind(school_a)
    .bind(format!("admin-{school_a}@t.dev"))
    .bind(&pw)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO users (id, school_id, email, password_hash, role, first_name, last_name)
         VALUES ($1,$2,$3,$4,'admin','B','Admin')",
    )
    .bind(Uuid::new_v4())
    .bind(school_b)
    .bind(format!("admin-{school_b}@t.dev"))
    .bind(&pw)
    .execute(&pool)
    .await
    .unwrap();

    Some(Ctx {
        state,
        school_a,
        school_b,
    })
}

async fn call(
    ctx: &Ctx,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(format!("/api/v1{path}"))
        .header("content-type", "application/json");
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let req = req
        .body(Body::from(
            body.map(|b| b.to_string()).unwrap_or_else(|| "{}".into()),
        ))
        .unwrap();
    let res = build_router(ctx.state.clone()).oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(json!({}));
    if status.is_server_error() || status.is_client_error() {
        eprintln!("{method} {path} -> {status} {json}");
    }
    (status, json)
}

async fn login(ctx: &Ctx, email: &str, password: &str) -> (StatusCode, Value) {
    call(
        ctx,
        "POST",
        "/auth/login",
        None,
        Some(json!({ "email": email, "password": password })),
    )
    .await
}

#[tokio::test]
async fn login_flow() {
    let Some(ctx) = ctx().await else { return };
    let email = format!("admin-{}@t.dev", ctx.school_a);

    let (s, body) = login(&ctx, &email, "password123").await;
    assert_eq!(s, StatusCode::OK);
    assert!(body["access_token"].is_string());
    assert!(body["refresh_token"].is_string());

    // me
    let token = body["access_token"].as_str().unwrap();
    let (s, me) = call(&ctx, "GET", "/auth/me", Some(token), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(me["email"], email);
    assert_eq!(me["role"], "admin");

    // wrong password
    let (s, _) = login(&ctx, &email, "wrong-password").await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // no token
    let (s, _) = call(&ctx, "GET", "/auth/me", None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // refresh rotation
    let refresh = body["refresh_token"].as_str().unwrap();
    let (s, body2) = call(
        &ctx,
        "POST",
        "/auth/refresh",
        None,
        Some(json!({ "refresh_token": refresh })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(body2["access_token"].is_string());
    // old refresh token is revoked
    let (s, _) = call(
        &ctx,
        "POST",
        "/auth/refresh",
        None,
        Some(json!({ "refresh_token": refresh })),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn tenant_isolation() {
    let Some(ctx) = ctx().await else { return };
    let email_a = format!("admin-{}@t.dev", ctx.school_a);
    let email_b = format!("admin-{}@t.dev", ctx.school_b);

    let (_, a) = login(&ctx, &email_a, "password123").await;
    let (_, b) = login(&ctx, &email_b, "password123").await;
    let ta = a["access_token"].as_str().unwrap();
    let tb = b["access_token"].as_str().unwrap();

    // admin A sees only school A users
    let (s, users) = call(&ctx, "GET", "/users", Some(ta), None).await;
    assert_eq!(s, StatusCode::OK);
    let users = users.as_array().unwrap();
    assert!(
        users
            .iter()
            .all(|u| u["school_id"] == ctx.school_a.to_string())
    );

    // admin B cannot touch school A by id
    let (s, _) = call(
        &ctx,
        "GET",
        &format!("/schools/{}", ctx.school_a),
        Some(tb),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // admin A cannot create a user in school B — school_id is forced to own
    let (s, u) = call(
        &ctx,
        "POST",
        "/users",
        Some(ta),
        Some(json!({
            "email": format!("x-{}@t.dev", ctx.school_a.simple()),
            "password": "password123",
            "role": "student",
            "first_name": "X",
            "last_name": "Y",
            "school_id": ctx.school_b,
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(u["school_id"], ctx.school_a.to_string());
}

#[tokio::test]
async fn journal_flow() {
    let Some(ctx) = ctx().await else { return };
    let email_a = format!("admin-{}@t.dev", ctx.school_a);
    let (_, a) = login(&ctx, &email_a, "password123").await;
    let ta = a["access_token"].as_str().unwrap();
    let sfx = ctx.school_a.simple().to_string();
    let student_email = format!("s1-{sfx}@t.dev");
    let teacher_email = format!("t1-{sfx}@t.dev");

    // subject + class + student
    let (s, subj) = call(
        &ctx,
        "POST",
        "/subjects",
        Some(ta),
        Some(json!({"name": "Math"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let subject = subj["id"].as_str().unwrap().to_string();
    let (s, cls) = call(
        &ctx,
        "POST",
        "/classes",
        Some(ta),
        Some(json!({"name": "9-A", "grade_level": 9, "year": 2026})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let class = cls["id"].as_str().unwrap().to_string();
    let (s, stu) = call(
        &ctx,
        "POST",
        "/users",
        Some(ta),
        Some(json!({
            "email": student_email, "password": "password123", "role": "student",
            "first_name": "S", "last_name": "T",
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let student = stu["id"].as_str().unwrap().to_string();

    let (s, _) = call(
        &ctx,
        "POST",
        &format!("/classes/{class}/students"),
        Some(ta),
        Some(json!({"student_id": student})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    // teacher + timetable slot + lesson generation
    let (s, t) = call(
        &ctx,
        "POST",
        "/users",
        Some(ta),
        Some(json!({
            "email": teacher_email, "password": "password123", "role": "teacher",
            "first_name": "T", "last_name": "T",
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let teacher = t["id"].as_str().unwrap().to_string();

    let (s, _) = call(
        &ctx,
        "POST",
        "/schedule/timetable",
        Some(ta),
        Some(json!({
            "class_id": class, "subject_id": subject, "teacher_id": teacher,
            "weekday": 1, "lesson_number": 1, "room": "101",
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // find next Monday (or today if Monday)
    let today = chrono::Utc::now().date_naive();
    let days_ahead = (7 - today.weekday().num_days_from_monday()) % 7;
    let monday = today + chrono::Duration::days(days_ahead as i64);
    let (s, lessons) = call(
        &ctx,
        "POST",
        "/schedule/lessons/generate",
        Some(ta),
        Some(json!({ "class_id": class, "week_start": monday })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let lesson = lessons[0]["id"].as_str().unwrap().to_string();

    // grade within scale (1..12 default)
    let (s, _) = call(
        &ctx,
        "POST",
        &format!("/journal/lessons/{lesson}/grades"),
        Some(ta),
        Some(json!({ "entries": [{ "student_id": student, "value": 11, "type": "test" }] })),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // grade out of scale rejected
    let (s, _) = call(
        &ctx,
        "POST",
        &format!("/journal/lessons/{lesson}/grades"),
        Some(ta),
        Some(json!({ "entries": [{ "student_id": student, "value": 42, "type": "test" }] })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // attendance
    let (s, _) = call(
        &ctx,
        "POST",
        &format!("/journal/lessons/{lesson}/attendance"),
        Some(ta),
        Some(json!({ "entries": [{ "student_id": student, "status": "present" }] })),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // student sees own grades
    let (_, s_login) = login(&ctx, &student_email, "password123").await;
    let ts = s_login["access_token"].as_str().unwrap();
    let (s, grades) = call(
        &ctx,
        "GET",
        &format!("/journal/students/{student}/grades"),
        Some(ts),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(grades[0]["value"], 11.0);
}
