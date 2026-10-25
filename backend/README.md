# ClassNode Backend

Rust API для шкільної платформи: multi-school (tenant-ізоляція по `school_id`),
JWT auth з ролями, класи, розклад, журнал, ДЗ, матеріали, новини, повідомлення,
сповіщення.

## Стек

Axum 0.8 · SQLx + PostgreSQL · Argon2 · jsonwebtoken · tower-http

## Запуск

```bash
docker compose up -d db          # Postgres 17 на :5432
cp .env.example .env
cargo run --bin classnode-backend # API на :3000
```

Сід демо-даних (школа, адмін, вчитель, клас 9-А, учні; усі паролі `password123`):

```bash
cargo run --bin seed
```

## Тести

```bash
TEST_DATABASE_URL=postgres://classnode:classnode@localhost:5432/classnode cargo test
```

Без `TEST_DATABASE_URL`/`DATABASE_URL` інтеграційні тести пропускаються.

## API (`/api/v1`)

| Розділ | Ендпоїнти |
|---|---|
| `auth` | `login`, `refresh`, `logout`, `me` |
| `schools` | CRUD (superadmin), `PATCH /:id` — налаштування + шкала оцінок |
| `users` | CRUD, `PATCH /me`, `/{id}/children` (parent links) |
| `classes` | CRUD (+`type` class/club, `location_id`), `/{id}/students`, `/{id}/subjects`, `/{id}/subgroups` |
| `subjects` | CRUD |
| `schedule` | `timetable` (slots, `me`), `lessons` (+`generate`) |
| `homework` | assignments, `/{id}/submit`, `/{id}/submissions`, grade |
| `journal` | `lessons/{id}/grades|attendance`, `students/{id}/grades|attendance`, `classes/{id}/journal` |
| `materials` | multipart upload/list/`{id}/download` |
| `announcements` | CRUD, audience: school/class/role |
| `messages` | send, conversations, `with/{id}`, mark read |
| `notifications` | list, unread-count, read, read-all |
| `finance` | charges, payments, debts, summary |
| `meals` | menus, orders, cancel |
| `quizzes` | quiz+questions CRUD, publish, submit (auto-score), attempts |
| `rewards` | give points, student balance/history, class leaderboard |
| `events` | school calendar, audience school/class/staff |
| `polls` | опитування: create, vote, results |
| `tasks` | staff tasks: create, list, status |
| `pickups` | "покликати дитину": parent call, done |
| `documents` | doc upload/list/download per user |
| `locations` | school locations CRUD |
| `transport` | routes + student assignment (довезення) |
| `leads` | CRM: prospective clients, statuses |
| `analytics` | attendance %, teacher-load, grade stats, report-card, journal.csv |

## Ролі

`superadmin` (платформа, без school_id) · `admin` (школа) · `teacher` · `student` · `parent`

Шкала оцінок — у `schools.settings.grading` (`{"min":1,"max":12}` за замовчуванням),
валідація на бекенді.
