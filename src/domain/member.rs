use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::payment_method::BillingMode;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Member {
    pub id: Uuid,
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub status: MemberStatus,
    pub membership_type: MembershipType,
    pub membership_type_id: Option<Uuid>,
    pub joined_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub dues_paid_until: Option<DateTime<Utc>>,
    pub bypass_dues: bool,
    pub is_admin: bool,
    pub notes: Option<String>,
    pub stripe_customer_id: Option<String>,
    pub stripe_subscription_id: Option<String>,
    pub billing_mode: BillingMode,
    /// When the member verified ownership of their email address.
    /// NULL = never verified. New signups start NULL; existing members
    /// were backfilled to their joined_at by migration 007.
    pub email_verified_at: Option<DateTime<Utc>>,
    /// When we last sent a "dues expiring soon" reminder for the
    /// current dues cycle. Cleared on payment (when dues_paid_until
    /// advances), set when the reminder goes out. One reminder per
    /// cycle per member.
    pub dues_reminder_sent_at: Option<DateTime<Utc>>,
    /// Discord user ID (snowflake). NULL means we don't know who they
    /// are on Discord — role sync skips them.
    pub discord_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Member {
    pub fn email_verified(&self) -> bool {
        self.email_verified_at.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq, ToSchema)]
#[sqlx(type_name = "TEXT")]
pub enum MemberStatus {
    Pending,
    Active,
    Expired,
    Suspended,
    Honorary,
}

impl MemberStatus {
    /// Canonical wire/DB string for this status. Same casing as the
    /// `status` column. Use this in URLs, sort keys, template DTOs —
    /// anywhere a stable string is needed — instead of
    /// `format!("{:?}", status)`, which couples the wire format to the
    /// `Debug` derive and rotates silently on rename.
    pub fn as_str(&self) -> &'static str {
        match self {
            MemberStatus::Pending => "Pending",
            MemberStatus::Active => "Active",
            MemberStatus::Expired => "Expired",
            MemberStatus::Suspended => "Suspended",
            MemberStatus::Honorary => "Honorary",
        }
    }

    /// Parse from the wire/DB string. Returns `None` for unknown
    /// values — callers should treat that as an explicit failure
    /// (BadRequest at the boundary, AppError::Database deeper down)
    /// rather than mapping to a default.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "Pending" => Some(MemberStatus::Pending),
            "Active" => Some(MemberStatus::Active),
            "Expired" => Some(MemberStatus::Expired),
            "Suspended" => Some(MemberStatus::Suspended),
            "Honorary" => Some(MemberStatus::Honorary),
            _ => None,
        }
    }
}

/// Legacy membership type enum - DEPRECATED
///
/// This enum is being phased out in favor of database-driven membership types.
/// Use `membership_type_id` field to reference `MembershipTypeConfig` from the
/// `membership_types` table instead.
///
/// To get the membership type name, look up the type by ID:
/// ```ignore
/// let type_config = membership_type_service.get(member.membership_type_id).await?;
/// let type_name = type_config.name;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq, ToSchema)]
#[sqlx(type_name = "TEXT")]
pub enum MembershipType {
    Regular,
    Student,
    Corporate,
    Lifetime,
}

impl MembershipType {
    /// Canonical wire/DB string. Same casing as the `membership_type`
    /// column. See `MemberStatus::as_str` for why this exists.
    pub fn as_str(&self) -> &'static str {
        match self {
            MembershipType::Regular => "Regular",
            MembershipType::Student => "Student",
            MembershipType::Corporate => "Corporate",
            MembershipType::Lifetime => "Lifetime",
        }
    }

    /// Parse from the wire/DB string. `None` on unknown — never map to
    /// a default; bad input should fail loudly at the boundary.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "Regular" => Some(MembershipType::Regular),
            "Student" => Some(MembershipType::Student),
            "Corporate" => Some(MembershipType::Corporate),
            "Lifetime" => Some(MembershipType::Lifetime),
            _ => None,
        }
    }
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
    pub password: String,
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