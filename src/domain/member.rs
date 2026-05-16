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
    pub membership_type_id: Uuid,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, sqlx::Type, PartialEq, ToSchema)]
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

    pub fn is_active(self) -> bool { matches!(self, MemberStatus::Active) }
    pub fn is_pending(self) -> bool { matches!(self, MemberStatus::Pending) }
    pub fn is_expired(self) -> bool { matches!(self, MemberStatus::Expired) }
    pub fn is_suspended(self) -> bool { matches!(self, MemberStatus::Suspended) }
    pub fn is_honorary(self) -> bool { matches!(self, MemberStatus::Honorary) }
}

#[cfg(test)]
mod member_status_predicate_tests {
    use super::MemberStatus;

    #[test]
    fn is_active_returns_true_for_active_only() {
        assert!(MemberStatus::Active.is_active());
        assert!(!MemberStatus::Pending.is_active());
        assert!(!MemberStatus::Expired.is_active());
        assert!(!MemberStatus::Suspended.is_active());
        assert!(!MemberStatus::Honorary.is_active());
    }

    #[test]
    fn is_pending_returns_true_for_pending_only() {
        assert!(MemberStatus::Pending.is_pending());
        assert!(!MemberStatus::Active.is_pending());
        assert!(!MemberStatus::Expired.is_pending());
        assert!(!MemberStatus::Suspended.is_pending());
        assert!(!MemberStatus::Honorary.is_pending());
    }

    #[test]
    fn is_expired_returns_true_for_expired_only() {
        assert!(MemberStatus::Expired.is_expired());
        assert!(!MemberStatus::Active.is_expired());
        assert!(!MemberStatus::Pending.is_expired());
        assert!(!MemberStatus::Suspended.is_expired());
        assert!(!MemberStatus::Honorary.is_expired());
    }

    #[test]
    fn is_suspended_returns_true_for_suspended_only() {
        assert!(MemberStatus::Suspended.is_suspended());
        assert!(!MemberStatus::Active.is_suspended());
        assert!(!MemberStatus::Pending.is_suspended());
        assert!(!MemberStatus::Expired.is_suspended());
        assert!(!MemberStatus::Honorary.is_suspended());
    }

    #[test]
    fn is_honorary_returns_true_for_honorary_only() {
        assert!(MemberStatus::Honorary.is_honorary());
        assert!(!MemberStatus::Active.is_honorary());
        assert!(!MemberStatus::Pending.is_honorary());
        assert!(!MemberStatus::Expired.is_honorary());
        assert!(!MemberStatus::Suspended.is_honorary());
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
    /// Caller-supplied FK into `membership_types`. `None` lets the
    /// repo pick the first `is_active` row by `sort_order` — used by
    /// public signup (no slug provided), tests, and the seed binary.
    /// Admin "create member" passes the chosen ID explicitly.
    pub membership_type_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateMemberRequest {
    pub full_name: Option<String>,
    pub status: Option<MemberStatus>,
    pub membership_type_id: Option<Uuid>,
    pub expires_at: Option<DateTime<Utc>>,
    pub bypass_dues: Option<bool>,
    pub notes: Option<String>,
}