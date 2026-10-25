use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(type_name = "text", rename_all = "lowercase")]
pub enum Role {
    Superadmin,
    Admin,
    Teacher,
    Student,
    Parent,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Superadmin => "superadmin",
            Role::Admin => "admin",
            Role::Teacher => "teacher",
            Role::Student => "student",
            Role::Parent => "parent",
        }
    }
}
