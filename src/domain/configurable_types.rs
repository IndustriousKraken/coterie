use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// =============================================================================
// Event Type
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EventTypeConfig {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateEventTypeRequest {
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateEventTypeRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub sort_order: Option<i32>,
    pub is_active: Option<bool>,
}

// =============================================================================
// Announcement Type
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AnnouncementTypeConfig {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAnnouncementTypeRequest {
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateAnnouncementTypeRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub sort_order: Option<i32>,
    pub is_active: Option<bool>,
}

// =============================================================================
// Membership Type
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq)]
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

// =============================================================================
// Default Types
// =============================================================================

/// Default event types to seed
pub fn default_event_types() -> Vec<(&'static str, &'static str, &'static str, &'static str)> {
    vec![
        ("Member Meeting", "member-meeting", "#2196F3", "users"),
        ("Social", "social", "#4CAF50", "glass-cheers"),
    ]
}

/// Default announcement types to seed
pub fn default_announcement_types() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("News", "news", "#2196F3"),
        ("Awards", "awards", "#FFC107"),
    ]
}

/// Default membership types to seed (name, slug, color, fee_cents, billing_period)
pub fn default_membership_types() -> Vec<(&'static str, &'static str, &'static str, i32, &'static str)> {
    vec![
        ("Member", "member", "#2196F3", 500, "monthly"),
        ("Associate", "associate", "#9C27B0", 10000, "monthly"),
        ("Life Member", "life-member", "#FF9800", 1000000, "lifetime"),
    ]
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
}
