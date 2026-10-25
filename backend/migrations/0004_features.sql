-- extended user profile
ALTER TABLE users
    ADD COLUMN birth_date DATE,
    ADD COLUMN address TEXT,
    ADD COLUMN custom_fields JSONB NOT NULL DEFAULT '{}';

-- classes can be regular classes or clubs (гуртки)
ALTER TABLE classes
    ADD COLUMN type TEXT NOT NULL DEFAULT 'class' CHECK (type IN ('class','club'));

-- subgroups within a class
CREATE TABLE subgroups (
    id UUID PRIMARY KEY,
    class_id UUID NOT NULL REFERENCES classes(id) ON DELETE CASCADE,
    subject_id UUID REFERENCES subjects(id) ON DELETE SET NULL,
    name TEXT NOT NULL,
    UNIQUE (class_id, name)
);

CREATE TABLE subgroup_students (
    subgroup_id UUID NOT NULL REFERENCES subgroups(id) ON DELETE CASCADE,
    student_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (subgroup_id, student_id)
);

-- teacher substitutions (заміни)
CREATE TABLE substitutions (
    lesson_id UUID PRIMARY KEY REFERENCES lessons(id) ON DELETE CASCADE,
    substitute_teacher_id UUID NOT NULL REFERENCES users(id),
    reason TEXT,
    created_by UUID NOT NULL REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- meals (харчування)
CREATE TABLE meal_menus (
    id UUID PRIMARY KEY,
    school_id UUID NOT NULL REFERENCES schools(id) ON DELETE CASCADE,
    date DATE NOT NULL,
    meal_type TEXT NOT NULL CHECK (meal_type IN ('breakfast','lunch','snack','dinner')),
    title TEXT NOT NULL,
    description TEXT,
    price NUMERIC(10,2),
    UNIQUE (school_id, date, meal_type)
);

CREATE TABLE meal_orders (
    id UUID PRIMARY KEY,
    menu_id UUID NOT NULL REFERENCES meal_menus(id) ON DELETE CASCADE,
    student_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    ordered_by UUID NOT NULL REFERENCES users(id),
    status TEXT NOT NULL DEFAULT 'ordered' CHECK (status IN ('ordered','served','cancelled')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (menu_id, student_id)
);

-- finance (оплати, заборгованості)
CREATE TABLE charges (
    id UUID PRIMARY KEY,
    school_id UUID NOT NULL REFERENCES schools(id) ON DELETE CASCADE,
    student_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    amount NUMERIC(12,2) NOT NULL,
    currency TEXT NOT NULL DEFAULT 'UAH',
    title TEXT NOT NULL,
    due_date DATE,
    status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open','paid','cancelled')),
    created_by UUID REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE payments (
    id UUID PRIMARY KEY,
    school_id UUID NOT NULL REFERENCES schools(id) ON DELETE CASCADE,
    charge_id UUID REFERENCES charges(id) ON DELETE SET NULL,
    student_id UUID NOT NULL REFERENCES users(id),
    amount NUMERIC(12,2) NOT NULL,
    method TEXT NOT NULL DEFAULT 'cash',
    note TEXT,
    recorded_by UUID REFERENCES users(id),
    paid_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- quizzes (тестування)
CREATE TABLE quizzes (
    id UUID PRIMARY KEY,
    school_id UUID NOT NULL REFERENCES schools(id) ON DELETE CASCADE,
    class_id UUID REFERENCES classes(id) ON DELETE CASCADE,
    subject_id UUID REFERENCES subjects(id) ON DELETE SET NULL,
    title TEXT NOT NULL,
    description TEXT,
    published BOOLEAN NOT NULL DEFAULT FALSE,
    created_by UUID NOT NULL REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE quiz_questions (
    id UUID PRIMARY KEY,
    quiz_id UUID NOT NULL REFERENCES quizzes(id) ON DELETE CASCADE,
    text TEXT NOT NULL,
    type TEXT NOT NULL CHECK (type IN ('single','multiple','text')),
    options JSONB NOT NULL DEFAULT '[]',
    correct JSONB NOT NULL DEFAULT '[]',
    points INT NOT NULL DEFAULT 1,
    position INT NOT NULL DEFAULT 0
);

CREATE TABLE quiz_attempts (
    id UUID PRIMARY KEY,
    quiz_id UUID NOT NULL REFERENCES quizzes(id) ON DELETE CASCADE,
    student_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    answers JSONB NOT NULL DEFAULT '{}',
    score NUMERIC,
    max_score NUMERIC,
    submitted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (quiz_id, student_id)
);

-- rewards (система заохочень)
CREATE TABLE reward_points (
    id UUID PRIMARY KEY,
    school_id UUID NOT NULL REFERENCES schools(id) ON DELETE CASCADE,
    student_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    points INT NOT NULL,
    reason TEXT NOT NULL,
    given_by UUID REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- calendar events (календар школи)
CREATE TABLE events (
    id UUID PRIMARY KEY,
    school_id UUID NOT NULL REFERENCES schools(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    description TEXT,
    start_at TIMESTAMPTZ NOT NULL,
    end_at TIMESTAMPTZ,
    audience TEXT NOT NULL DEFAULT 'school' CHECK (audience IN ('school','class','staff')),
    class_id UUID REFERENCES classes(id) ON DELETE CASCADE,
    type TEXT NOT NULL DEFAULT 'event',
    created_by UUID REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- polls (опитування)
CREATE TABLE polls (
    id UUID PRIMARY KEY,
    school_id UUID NOT NULL REFERENCES schools(id) ON DELETE CASCADE,
    question TEXT NOT NULL,
    options JSONB NOT NULL,
    audience TEXT NOT NULL DEFAULT 'school' CHECK (audience IN ('school','parents','teachers','class')),
    class_id UUID REFERENCES classes(id) ON DELETE CASCADE,
    ends_at TIMESTAMPTZ,
    created_by UUID REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE poll_votes (
    poll_id UUID NOT NULL REFERENCES polls(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    option_index INT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (poll_id, user_id)
);

-- staff tasks (завдання користувачам)
CREATE TABLE tasks (
    id UUID PRIMARY KEY,
    school_id UUID NOT NULL REFERENCES schools(id) ON DELETE CASCADE,
    creator_id UUID NOT NULL REFERENCES users(id),
    assignee_id UUID REFERENCES users(id),
    title TEXT NOT NULL,
    description TEXT,
    due_date DATE,
    status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open','done','cancelled')),
    done_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- pickup calls (покликати дитину)
CREATE TABLE pickup_calls (
    id UUID PRIMARY KEY,
    student_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    requested_by UUID NOT NULL REFERENCES users(id),
    note TEXT,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','done')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    done_at TIMESTAMPTZ
);

-- documents (електронна документація)
CREATE TABLE documents (
    id UUID PRIMARY KEY,
    school_id UUID NOT NULL REFERENCES schools(id) ON DELETE CASCADE,
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    uploader_id UUID NOT NULL REFERENCES users(id),
    title TEXT NOT NULL,
    doc_type TEXT NOT NULL DEFAULT 'other',
    filename TEXT NOT NULL,
    stored_name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- locations (додаткові локації)
CREATE TABLE locations (
    id UUID PRIMARY KEY,
    school_id UUID NOT NULL REFERENCES schools(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    address TEXT,
    UNIQUE (school_id, name)
);

ALTER TABLE classes ADD COLUMN location_id UUID REFERENCES locations(id) ON DELETE SET NULL;

-- transport (довезення)
CREATE TABLE transport_routes (
    id UUID PRIMARY KEY,
    school_id UUID NOT NULL REFERENCES schools(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    driver_name TEXT,
    driver_phone TEXT,
    stops JSONB NOT NULL DEFAULT '[]',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (school_id, name)
);

CREATE TABLE route_students (
    route_id UUID NOT NULL REFERENCES transport_routes(id) ON DELETE CASCADE,
    student_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    stop TEXT,
    PRIMARY KEY (route_id, student_id)
);

-- CRM leads (робота з клієнтами)
CREATE TABLE leads (
    id UUID PRIMARY KEY,
    school_id UUID NOT NULL REFERENCES schools(id) ON DELETE CASCADE,
    parent_name TEXT NOT NULL,
    parent_phone TEXT,
    parent_email TEXT,
    child_name TEXT,
    notes TEXT,
    status TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new','contacted','trial','enrolled','rejected')),
    created_by UUID REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_subgroups_class ON subgroups(class_id);
CREATE INDEX idx_meal_menus_date ON meal_menus(school_id, date);
CREATE INDEX idx_meal_orders_student ON meal_orders(student_id);
CREATE INDEX idx_charges_student ON charges(student_id, status);
CREATE INDEX idx_payments_student ON payments(student_id);
CREATE INDEX idx_quizzes_class ON quizzes(class_id);
CREATE INDEX idx_quiz_attempts_student ON quiz_attempts(student_id);
CREATE INDEX idx_reward_student ON reward_points(student_id);
CREATE INDEX idx_events_school ON events(school_id, start_at);
CREATE INDEX idx_tasks_assignee ON tasks(assignee_id, status);
CREATE INDEX idx_pickup_student ON pickup_calls(student_id, status);
CREATE INDEX idx_documents_user ON documents(user_id);
CREATE INDEX idx_leads_school ON leads(school_id, status);
