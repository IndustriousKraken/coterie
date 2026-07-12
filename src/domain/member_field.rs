//! Org-defined member fields (see the member-custom-fields capability).
//! A cybersecurity guild adds "HackTheBox ID"; a baduk club adds
//! "Rank" — Coterie stays org-agnostic and the admin defines what a
//! member record carries.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Rendering/validation hint for a field. `Url` values must be
/// http(s):// and render as links.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemberFieldType {
    Text,
    Url,
}

impl MemberFieldType {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemberFieldType::Text => "text",
            MemberFieldType::Url => "url",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "text" => Some(MemberFieldType::Text),
            "url" => Some(MemberFieldType::Url),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberFieldDefinition {
    pub id: Uuid,
    /// Display label, renameable ("HackTheBox ID").
    pub name: String,
    /// Stable identifier forms post under ("hackthebox-id"). Unique,
    /// immutable after creation.
    pub field_key: String,
    pub field_type: MemberFieldType,
    /// Whether members may edit this field on their own profile.
    /// Admins can always edit.
    pub member_editable: bool,
    pub sort_order: i32,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A definition paired with one member's value (None = unset). The
/// display shape for both the admin member page and the profile page.
#[derive(Debug, Clone)]
pub struct FieldWithValue {
    pub definition: MemberFieldDefinition,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateMemberFieldRequest {
    pub name: String,
    /// Optional explicit key; derived from the name when omitted.
    pub field_key: Option<String>,
    pub field_type: String,
    pub member_editable: bool,
    pub sort_order: i32,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpdateMemberFieldRequest {
    pub name: Option<String>,
    pub field_type: Option<String>,
    pub member_editable: Option<bool>,
    pub sort_order: Option<i32>,
    pub is_active: Option<bool>,
}
