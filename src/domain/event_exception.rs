//! Per-occurrence exceptions for recurring event series.
//!
//! The materializer normally treats every `(series, occurrence_index)`
//! pair identically, applying the series template to each. An exception
//! row names one such pair as special — either cancelled (skipped) or
//! overridden (template + per-occurrence field deltas). See migration
//! `025_event_series_exceptions.sql` and `design.md` D1/D2/D3.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::Event;

/// Discriminator on the exception row. Cancelled exceptions have a NULL
/// `override_payload`; overridden exceptions carry a JSON-serialized
/// `OccurrenceOverride`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OccurrenceExceptionKind {
    Cancelled,
    Overridden,
}

impl OccurrenceExceptionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Overridden => "overridden",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "cancelled" => Some(Self::Cancelled),
            "overridden" => Some(Self::Overridden),
            _ => None,
        }
    }
}

/// One row of `event_series_exceptions`. Domain shape — repo layer
/// converts to/from the sqlx row representation.
#[derive(Debug, Clone)]
pub struct OccurrenceException {
    pub series_id: Uuid,
    pub occurrence_index: i32,
    pub kind: OccurrenceExceptionKind,
    /// `Some` only when `kind == Overridden`. JSON-serialized
    /// [`OccurrenceOverride`].
    pub override_payload: Option<String>,
    pub created_at: DateTime<Utc>,
    pub created_by: Uuid,
    pub audit_reason: Option<String>,
}

/// The subset of `Event` fields a per-occurrence override may set.
/// `None` for a field means "use the series template's value." Stored
/// as JSON in `event_series_exceptions.override_payload` for overridden
/// exceptions.
///
/// `event_type` and `visibility` are deliberately omitted — those are
/// series-level concerns per `design.md` D2.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OccurrenceOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_attendees: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rsvp_required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
}

impl OccurrenceOverride {
    /// Overwrite each of `target`'s fields that has a corresponding
    /// `Some` value on `self`. Fields that are `None` are left as-is.
    pub fn apply(self, target: &mut Event) {
        if let Some(v) = self.title {
            target.title = v;
        }
        if let Some(v) = self.description {
            target.description = v;
        }
        if let Some(v) = self.start_time {
            target.start_time = v;
        }
        if let Some(v) = self.end_time {
            target.end_time = Some(v);
        }
        if let Some(v) = self.location {
            target.location = Some(v);
        }
        if let Some(v) = self.max_attendees {
            target.max_attendees = Some(v);
        }
        if let Some(v) = self.rsvp_required {
            target.rsvp_required = v;
        }
        if let Some(v) = self.image_url {
            target.image_url = Some(v);
        }
    }

    /// True when no field has been set — round-tripping an empty
    /// override is allowed but the caller probably meant nothing.
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.description.is_none()
            && self.start_time.is_none()
            && self.end_time.is_none()
            && self.location.is_none()
            && self.max_attendees.is_none()
            && self.rsvp_required.is_none()
            && self.image_url.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{EventType, EventVisibility};

    fn sample_event() -> Event {
        Event {
            id: Uuid::new_v4(),
            title: "Original".to_string(),
            description: "Original description".to_string(),
            event_type: EventType::Meeting,
            event_type_id: None,
            visibility: EventVisibility::MembersOnly,
            start_time: Utc::now(),
            end_time: None,
            location: Some("Room A".to_string()),
            max_attendees: Some(10),
            rsvp_required: false,
            image_url: None,
            created_by: Uuid::new_v4(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            series_id: Some(Uuid::new_v4()),
            occurrence_index: Some(5),
        }
    }

    #[test]
    fn override_apply_only_sets_some_fields() {
        let mut event = sample_event();
        let original_title = event.title.clone();
        let original_max = event.max_attendees;

        let ov = OccurrenceOverride {
            location: Some("Room B".to_string()),
            ..Default::default()
        };
        ov.apply(&mut event);

        assert_eq!(event.location, Some("Room B".to_string()));
        assert_eq!(event.title, original_title);
        assert_eq!(event.max_attendees, original_max);
    }

    #[test]
    fn override_apply_all_fields() {
        let mut event = sample_event();
        let new_time = Utc::now() + chrono::Duration::days(7);

        let ov = OccurrenceOverride {
            title: Some("Renamed".to_string()),
            description: Some("New desc".to_string()),
            start_time: Some(new_time),
            end_time: Some(new_time + chrono::Duration::hours(1)),
            location: Some("Room X".to_string()),
            max_attendees: Some(99),
            rsvp_required: Some(true),
            image_url: Some("img.png".to_string()),
        };
        ov.apply(&mut event);

        assert_eq!(event.title, "Renamed");
        assert_eq!(event.description, "New desc");
        assert_eq!(event.start_time, new_time);
        assert_eq!(event.end_time, Some(new_time + chrono::Duration::hours(1)));
        assert_eq!(event.location, Some("Room X".to_string()));
        assert_eq!(event.max_attendees, Some(99));
        assert!(event.rsvp_required);
        assert_eq!(event.image_url, Some("img.png".to_string()));
    }

    #[test]
    fn override_empty_serde_default() {
        let json = "{}";
        let ov: OccurrenceOverride = serde_json::from_str(json).unwrap();
        assert!(ov.is_empty());
    }

    #[test]
    fn override_serde_roundtrip_partial() {
        let ov = OccurrenceOverride {
            location: Some("Conference Room B".to_string()),
            ..Default::default()
        };
        let s = serde_json::to_string(&ov).unwrap();
        // Only the set field should be in the JSON.
        assert!(s.contains("location"));
        assert!(!s.contains("title"));
        let parsed: OccurrenceOverride = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed, ov);
    }

    #[test]
    fn kind_string_roundtrip() {
        assert_eq!(OccurrenceExceptionKind::Cancelled.as_str(), "cancelled");
        assert_eq!(OccurrenceExceptionKind::Overridden.as_str(), "overridden");
        assert_eq!(
            OccurrenceExceptionKind::parse("cancelled"),
            Some(OccurrenceExceptionKind::Cancelled),
        );
        assert_eq!(
            OccurrenceExceptionKind::parse("overridden"),
            Some(OccurrenceExceptionKind::Overridden),
        );
        assert_eq!(OccurrenceExceptionKind::parse("nope"), None);
    }
}
