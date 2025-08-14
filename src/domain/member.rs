use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Member {
    pub id: Uuid,
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub status: MemberStatus,
    pub membership_type: MembershipType,
    pub joined_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub dues_paid_until: Option<DateTime<Utc>>,
    pub bypass_dues: bool,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq)]
#[sqlx(type_name = "TEXT")]
pub enum MemberStatus {
    Pending,
    Active,
    Expired,
    Suspended,
    Honorary,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT")]
pub enum MembershipType {
    Regular,
    Student,
    Corporate,
    Lifetime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberProfile {
    pub member_id: Uuid,
    pub bio: Option<String>,
    pub skills: Vec<String>,
    pub show_in_directory: bool,
    pub blog_url: Option<String>,
    pub github_username: Option<String>,
    pub discord_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMemberRequest {
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub membership_type: MembershipType,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateMemberRequest {
    pub full_name: Option<String>,
    pub status: Option<MemberStatus>,
    pub membership_type: Option<MembershipType>,
    pub expires_at: Option<DateTime<Utc>>,
    pub bypass_dues: Option<bool>,
    pub notes: Option<String>,
}