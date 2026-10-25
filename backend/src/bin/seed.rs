use chrono::Datelike;
use uuid::Uuid;

use classnode_backend::{auth::hash_password, config::Config, db};

#[tokio::main]
async fn main() {
    let config = Config::from_env();
    let pool = db::connect(&config.database_url).await;
    let pw = hash_password("password123").unwrap();

    let school = Uuid::new_v4();
    sqlx::query("INSERT INTO schools (id, name) VALUES ($1, 'Demo School') ON CONFLICT DO NOTHING")
        .bind(school)
        .execute(&pool)
        .await
        .unwrap();

    let mk = |email: &str, role: &str, first: &str, last: &str, sid: Option<Uuid>| {
        let pool = pool.clone();
        let pw = pw.clone();
        let email = email.to_string();
        let role = role.to_string();
        let first = first.to_string();
        let last = last.to_string();
        async move {
            let id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO users (id, school_id, email, password_hash, role, first_name, last_name)
                 VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (email) DO NOTHING",
            )
            .bind(id)
            .bind(sid)
            .bind(&email)
            .bind(&pw)
            .bind(&role)
            .bind(&first)
            .bind(&last)
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE email = $1")
                .bind(&email)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };

    let superadmin = mk("root@classnode.dev", "superadmin", "Root", "Admin", None).await;
    let admin = mk(
        "admin@classnode.dev",
        "admin",
        "School",
        "Admin",
        Some(school),
    )
    .await;
    let teacher = mk(
        "teacher@classnode.dev",
        "teacher",
        "Tetiana",
        "Vchytel",
        Some(school),
    )
    .await;
    let parent = mk(
        "parent@classnode.dev",
        "parent",
        "Olena",
        "Batko",
        Some(school),
    )
    .await;
    let students = [
        mk(
            "ivan@classnode.dev",
            "student",
            "Ivan",
            "Uchen",
            Some(school),
        )
        .await,
        mk(
            "maria@classnode.dev",
            "student",
            "Maria",
            "Uchen",
            Some(school),
        )
        .await,
        mk(
            "petro@classnode.dev",
            "student",
            "Petro",
            "Uchen",
            Some(school),
        )
        .await,
    ];

    let class = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO classes (id, school_id, name, grade_level, year, homeroom_teacher_id)
         VALUES ($1,$2,'9-А',9,$3,$4) ON CONFLICT DO NOTHING",
    )
    .bind(class)
    .bind(school)
    .bind(chrono::Utc::now().year())
    .bind(teacher)
    .execute(&pool)
    .await
    .unwrap();
    let class_id: Uuid =
        sqlx::query_scalar("SELECT id FROM classes WHERE school_id = $1 AND name = '9-А' LIMIT 1")
            .bind(school)
            .fetch_one(&pool)
            .await
            .unwrap();

    for s in &students {
        sqlx::query(
            "INSERT INTO enrollments (class_id, student_id) VALUES ($1,$2) ON CONFLICT DO NOTHING",
        )
        .bind(class_id)
        .bind(s)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO parent_links (parent_id, student_id) VALUES ($1,$2) ON CONFLICT DO NOTHING",
    )
    .bind(parent)
    .bind(students[0])
    .execute(&pool)
    .await
    .unwrap();

    let subjects = ["Математика", "Українська мова", "Англійська"];
    let mut subject_ids = Vec::new();
    for name in subjects {
        sqlx::query(
            "INSERT INTO subjects (id, school_id, name) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
        )
        .bind(Uuid::new_v4())
        .bind(school)
        .bind(name)
        .execute(&pool)
        .await
        .unwrap();
        subject_ids.push(
            sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM subjects WHERE school_id = $1 AND name = $2",
            )
            .bind(school)
            .bind(name)
            .fetch_one(&pool)
            .await
            .unwrap(),
        );
    }
    for (i, sub) in subject_ids.iter().enumerate() {
        sqlx::query(
            "INSERT INTO class_subject_teachers (class_id, subject_id, teacher_id)
             VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
        )
        .bind(class_id)
        .bind(sub)
        .bind(teacher)
        .execute(&pool)
        .await
        .unwrap();
        // one timetable slot per subject: Mon/Tue/Wed lesson 1
        sqlx::query(
            "INSERT INTO timetable_slots (id, school_id, class_id, subject_id, teacher_id, weekday, lesson_number, room)
             VALUES ($1,$2,$3,$4,$5,$6,1,$7) ON CONFLICT DO NOTHING",
        )
        .bind(Uuid::new_v4())
        .bind(school)
        .bind(class_id)
        .bind(sub)
        .bind(teacher)
        .bind(i as i32 + 1)
        .bind(format!("{}", 101 + i))
        .execute(&pool)
        .await
        .unwrap();
    }

    println!("Seed complete. All passwords: password123");
    println!("  superadmin: root@classnode.dev ({superadmin})");
    println!("  admin:      admin@classnode.dev ({admin})");
    println!("  teacher:    teacher@classnode.dev ({teacher})");
    println!("  parent:     parent@classnode.dev ({parent}, child = ivan@classnode.dev)");
    println!("  students:   ivan@ / maria@ / petro@classnode.dev");
    println!("  school {school}, class 9-А {class_id}");
}
