use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// =============================================================================
// Basic Type (shared shape for event types and announcement types)
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BasicType {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub sort_order: i32,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// Closed enum — `table()`, `usage_table()`, `usage_fk()`, and `display_name()`
// return compile-time `&'static str` constants. SQL strings interpolate these
// safely because they're never user-controlled; do not extend this enum to
// admit user input.
#[derive(Debug, Clone, Copy)]
pub enum BasicTypeKind {
    Event,
    Announcement,
}

impl BasicTypeKind {
    pub fn table(self) -> &'static str {
        match self {
            BasicTypeKind::Event => "event_types",
            BasicTypeKind::Announcement => "announcement_types",
        }
    }

    pub fn usage_table(self) -> &'static str {
        match self {
            BasicTypeKind::Event => "events",
            BasicTypeKind::Announcement => "announcements",
        }
    }

    pub fn usage_fk(self) -> &'static str {
        match self {
            BasicTypeKind::Event => "event_type_id",
            BasicTypeKind::Announcement => "announcement_type_id",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            BasicTypeKind::Event => "event type",
            BasicTypeKind::Announcement => "announcement type",
        }
    }

    pub fn usage_noun_plural(self) -> &'static str {
        match self {
            BasicTypeKind::Event => "events",
            BasicTypeKind::Announcement => "announcements",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateBasicTypeRequest {
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateBasicTypeRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub sort_order: Option<i32>,
    pub is_active: Option<bool>,
}

// Old domain names live on as aliases so the API boundary keeps reading as
// "event-type-flavored" / "announcement-type-flavored" data. The aliases are
// in use, not a backwards-compat shim.
pub type EventTypeConfig = BasicType;
pub type AnnouncementTypeConfig = BasicType;
pub type CreateEventTypeRequest = CreateBasicTypeRequest;
pub type CreateAnnouncementTypeRequest = CreateBasicTypeRequest;
pub type UpdateEventTypeRequest = UpdateBasicTypeRequest;
pub type UpdateAnnouncementTypeRequest = UpdateBasicTypeRequest;

// =============================================================================
// Membership Type
// =============================================================================

#[derive(Debug, Clone, Copy, Serialize, Deserialize, sqlx::Type, PartialEq, Eq)]
#[sqlx(type_name = "TEXT")]
pub enum BillingPeriod {
    Monthly,
    Yearly,
    Lifetime,
}

impl BillingPeriod {
    pub fn as_str(&self) -> &'static str {
        match self {
            BillingPeriod::Monthly => "monthly",
            BillingPeriod::Yearly => "yearly",
            BillingPeriod::Lifetime => "lifetime",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "monthly" => Some(BillingPeriod::Monthly),
            "yearly" => Some(BillingPeriod::Yearly),
            "lifetime" => Some(BillingPeriod::Lifetime),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MembershipTypeConfig {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub sort_order: i32,
    pub is_active: bool,
    pub fee_cents: i32,
    pub billing_period: String, // Stored as text, parsed via BillingPeriod
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl MembershipTypeConfig {
    pub fn billing_period_enum(&self) -> Option<BillingPeriod> {
        BillingPeriod::from_str(&self.billing_period)
    }

    pub fn fee_dollars(&self) -> f64 {
        self.fee_cents as f64 / 100.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMembershipTypeRequest {
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub fee_cents: i32,
    pub billing_period: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateMembershipTypeRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub sort_order: Option<i32>,
    pub is_active: Option<bool>,
    pub fee_cents: Option<i32>,
    pub billing_period: Option<String>,
}

// =============================================================================
// Utility Functions
// =============================================================================

/// Generate a URL-safe slug from a name
pub fn slugify(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Validate hex color format
pub fn validate_hex_color(color: &str) -> bool {
    if !color.starts_with('#') {
        return false;
    }
    let hex = &color[1..];
    (hex.len() == 3 || hex.len() == 6) && hex.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("CTF Result"), "ctf-result");
        assert_eq!(slugify("  Multiple   Spaces  "), "multiple-spaces");
        assert_eq!(slugify("Special!@#$Characters"), "special-characters");
    }

    #[test]
    fn test_validate_hex_color() {
        assert!(validate_hex_color("#FFF"));
        assert!(validate_hex_color("#fff"));
        assert!(validate_hex_color("#FFFFFF"));
        assert!(validate_hex_color("#2196F3"));
        assert!(!validate_hex_color("FFF"));
        assert!(!validate_hex_color("#GGGGGG"));
        assert!(!validate_hex_color("#12345"));
    }

    #[test]
    fn test_billing_period() {
        assert_eq!(BillingPeriod::Yearly.as_str(), "yearly");
        assert_eq!(BillingPeriod::from_str("yearly"), Some(BillingPeriod::Yearly));
        assert_eq!(BillingPeriod::from_str("YEARLY"), Some(BillingPeriod::Yearly));
        assert_eq!(BillingPeriod::from_str("invalid"), None);
    }

    #[test]
    fn test_basic_type_kind_accessors() {
        assert_eq!(BasicTypeKind::Event.table(), "event_types");
        assert_eq!(BasicTypeKind::Event.usage_table(), "events");
        assert_eq!(BasicTypeKind::Event.usage_fk(), "event_type_id");
        assert_eq!(BasicTypeKind::Event.display_name(), "event type");

        assert_eq!(BasicTypeKind::Announcement.table(), "announcement_types");
        assert_eq!(BasicTypeKind::Announcement.usage_table(), "announcements");
        assert_eq!(BasicTypeKind::Announcement.usage_fk(), "announcement_type_id");
        assert_eq!(BasicTypeKind::Announcement.display_name(), "announcement type");
    }
}
