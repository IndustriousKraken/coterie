use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A member-authored proposal (talk/session/etc.) reviewed by an admin.
///
/// Every member-authored field (`title`, `abstract_text`, and the
/// uploaded attachment) is untrusted input consumed later in the admin's
/// authenticated origin — see the member-proposal-submissions design for
/// the threat model. Ownership is `submitter_member_id`, always taken
/// from the session, never the request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Submission {
    pub id: Uuid,
    pub submitter_member_id: Uuid,
    pub title: String,
    pub abstract_text: String,
    pub visibility_requested: SubmissionVisibility,
    /// Server-generated `uploads/<uuid>.pdf` relative path, or `None`.
    pub attachment_path: Option<String>,
    /// Preferred **local wall-clock** (naive part of a `DateTime<Utc>`
    /// container), paired with [`Submission::timezone`] — NOT a real
    /// instant. Mirrors `Event::start_time`. `None` when unspecified.
    pub preferred_start: Option<DateTime<Utc>>,
    /// IANA zone the `preferred_start` wall-clock is understood in.
    pub timezone: String,
    pub duration_minutes: Option<i32>,
    pub status: SubmissionStatus,
    pub reviewer_note: Option<String>,
    pub decided_by: Option<Uuid>,
    /// Set on promotion to a standard `Event`.
    pub event_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Submission {
    /// Whether the submission is still open (non-terminal) — i.e. counts
    /// against the per-member open cap and is still editable/withdrawable
    /// by its owner.
    pub fn is_open(&self) -> bool {
        matches!(
            self.status,
            SubmissionStatus::Submitted | SubmissionStatus::UnderReview
        )
    }
}

/// Requested audience for a submission. This is a *request* only —
/// nothing reaches the public surface until an admin accepts and (with a
/// schedule) promotes it to an `Event`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubmissionVisibility {
    Public,
    Members,
}

impl SubmissionVisibility {
    pub fn as_wire(&self) -> &'static str {
        match self {
            SubmissionVisibility::Public => "public",
            SubmissionVisibility::Members => "members",
        }
    }

    /// Parse the canonical wire string; anything unrecognized falls back
    /// to the most restrictive (`Members`) so a bad value never widens
    /// exposure.
    pub fn from_wire(s: &str) -> Self {
        match s {
            "public" => SubmissionVisibility::Public,
            _ => SubmissionVisibility::Members,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubmissionStatus {
    Submitted,
    UnderReview,
    Accepted,
    Declined,
    Withdrawn,
    Scheduled,
}

impl SubmissionStatus {
    pub fn as_wire(&self) -> &'static str {
        match self {
            SubmissionStatus::Submitted => "submitted",
            SubmissionStatus::UnderReview => "under_review",
            SubmissionStatus::Accepted => "accepted",
            SubmissionStatus::Declined => "declined",
            SubmissionStatus::Withdrawn => "withdrawn",
            SubmissionStatus::Scheduled => "scheduled",
        }
    }

    /// Parse the canonical wire string. Returns `None` on an unknown
    /// value so a corrupt row surfaces as an error rather than being
    /// silently coerced.
    pub fn from_wire(s: &str) -> Option<Self> {
        Some(match s {
            "submitted" => SubmissionStatus::Submitted,
            "under_review" => SubmissionStatus::UnderReview,
            "accepted" => SubmissionStatus::Accepted,
            "declined" => SubmissionStatus::Declined,
            "withdrawn" => SubmissionStatus::Withdrawn,
            "scheduled" => SubmissionStatus::Scheduled,
            _ => return None,
        })
    }

    /// Human label for the review UI.
    pub fn label(&self) -> &'static str {
        match self {
            SubmissionStatus::Submitted => "Submitted",
            SubmissionStatus::UnderReview => "Under review",
            SubmissionStatus::Accepted => "Accepted",
            SubmissionStatus::Declined => "Declined",
            SubmissionStatus::Withdrawn => "Withdrawn",
            SubmissionStatus::Scheduled => "Scheduled",
        }
    }
}
