//! Service owning the member-proposal-submissions lifecycle: create
//! (with field bounds + the per-member open cap), owner-scoped
//! edit/withdraw, admin status transitions over a validated graph, and
//! `promote` (create a standard `Event` via the existing event path).
//!
//! Security-load-bearing invariants live here, NOT in the handlers:
//!   - `submitter_member_id` and `decided_by` are ALWAYS taken from the
//!     authenticated principal passed in, never from request data.
//!   - Ownership is enforced on every owner-scoped read/mutate and denied
//!     WITHOUT disclosure (404), never relying on the id being unguessable.
//!   - Status transitions are admin-only and restricted to a fixed graph;
//!     a member can only reach `withdrawn`.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    domain::{EventType, EventVisibility, Submission, SubmissionStatus, SubmissionVisibility},
    error::{AppError, Result},
    repository::SubmissionRepository,
    service::{
        audit_service::AuditService,
        event_admin_service::{CreateEventInput, EventAdminService},
    },
};

/// Field bounds. Rejected (not truncated) when exceeded, so an oversized
/// value never silently persists partially.
const TITLE_MAX_CHARS: usize = 200;
const ABSTRACT_MAX_CHARS: usize = 5_000;
const REVIEWER_NOTE_MAX_CHARS: usize = 2_000;

/// Per-member cap on open (non-terminal) submissions — bounds spam and
/// storage exhaustion. A create that would exceed it is refused.
// ponytail: fixed cap; make it a `submissions.*` setting if an org ever
// needs to tune it.
const MAX_OPEN_SUBMISSIONS: i64 = 5;

/// Typed create input. `submitter_member_id` is NOT part of this — it is
/// supplied separately from the session so a forged body can't set it.
pub struct CreateSubmissionInput {
    pub title: String,
    pub abstract_text: String,
    pub visibility_requested: SubmissionVisibility,
    /// Server-generated relative path from the upload layer, or `None`.
    pub attachment_path: Option<String>,
    pub preferred_start: Option<DateTime<Utc>>,
    pub timezone: String,
    pub duration_minutes: Option<i32>,
}

/// Editable subset a member may change while their submission is still
/// `submitted`. Status, submitter, decision fields are not here.
pub struct UpdateSubmissionInput {
    pub title: String,
    pub abstract_text: String,
    pub visibility_requested: SubmissionVisibility,
    pub preferred_start: Option<DateTime<Utc>>,
    pub timezone: String,
    pub duration_minutes: Option<i32>,
    /// `None` = leave the current attachment; `Some(path)` = replace with
    /// a freshly-saved one.
    pub new_attachment_path: Option<String>,
}

/// Optional schedule an admin supplies when accepting, used to promote
/// the submission to a standard `Event`. The wall-clock convention
/// follows events: `start` is a naive local time paired with `timezone`.
pub struct PromoteSchedule {
    pub start: DateTime<Utc>,
    pub timezone: String,
    pub duration_minutes: Option<i32>,
}

pub struct SubmissionService {
    submission_repo: Arc<dyn SubmissionRepository>,
    event_admin_service: Arc<EventAdminService>,
    audit_service: Arc<AuditService>,
}

impl SubmissionService {
    pub fn new(
        submission_repo: Arc<dyn SubmissionRepository>,
        event_admin_service: Arc<EventAdminService>,
        audit_service: Arc<AuditService>,
    ) -> Self {
        Self {
            submission_repo,
            event_admin_service,
            audit_service,
        }
    }

    // --- Member-facing --------------------------------------------------

    /// Create a submission owned by `submitter_id` (from the session).
    /// Enforces field bounds and the per-member open cap; persists with
    /// status `submitted`.
    pub async fn create(
        &self,
        submitter_id: Uuid,
        input: CreateSubmissionInput,
    ) -> Result<Submission> {
        validate_title(&input.title)?;
        validate_abstract(&input.abstract_text)?;

        let open = self
            .submission_repo
            .count_open_for_member(submitter_id)
            .await?;
        if open >= MAX_OPEN_SUBMISSIONS {
            return Err(AppError::Validation(format!(
                "You already have {} open submissions (the maximum). Withdraw one before creating another.",
                MAX_OPEN_SUBMISSIONS
            )));
        }

        let now = Utc::now();
        let submission = Submission {
            id: Uuid::new_v4(),
            submitter_member_id: submitter_id,
            title: input.title,
            abstract_text: input.abstract_text,
            visibility_requested: input.visibility_requested,
            attachment_path: input.attachment_path,
            preferred_start: input.preferred_start,
            timezone: input.timezone,
            duration_minutes: input.duration_minutes,
            status: SubmissionStatus::Submitted,
            reviewer_note: None,
            decided_by: None,
            event_id: None,
            created_at: now,
            updated_at: now,
        };
        let created = self.submission_repo.create(submission).await?;
        self.audit_service
            .log(
                Some(submitter_id),
                "create_submission",
                "submission",
                &created.id.to_string(),
                None,
                Some(&created.title),
                None,
            )
            .await;
        Ok(created)
    }

    /// Load a submission the caller is allowed to see. The submitter and
    /// any admin may read it; anyone else is denied WITHOUT disclosing the
    /// row exists (404), and the check never relies on the id being
    /// unguessable.
    pub async fn get_authorized(
        &self,
        actor_id: Uuid,
        is_admin: bool,
        id: Uuid,
    ) -> Result<Submission> {
        let submission = self
            .submission_repo
            .find_by_id(id)
            .await?
            .ok_or_else(not_found)?;
        if !is_admin && submission.submitter_member_id != actor_id {
            // Deny without disclosure — indistinguishable from "no such row".
            return Err(not_found());
        }
        Ok(submission)
    }

    /// Owner edit, permitted ONLY while the submission is still
    /// `submitted`. Ownership is enforced (admins do not edit member
    /// content through this path).
    pub async fn update_owned(
        &self,
        actor_id: Uuid,
        id: Uuid,
        input: UpdateSubmissionInput,
    ) -> Result<Submission> {
        let mut submission = self.load_owned(actor_id, id).await?;
        if submission.status != SubmissionStatus::Submitted {
            return Err(AppError::Validation(
                "This submission can no longer be edited (a reviewer has picked it up)."
                    .to_string(),
            ));
        }
        validate_title(&input.title)?;
        validate_abstract(&input.abstract_text)?;

        submission.title = input.title;
        submission.abstract_text = input.abstract_text;
        submission.visibility_requested = input.visibility_requested;
        submission.preferred_start = input.preferred_start;
        submission.timezone = input.timezone;
        submission.duration_minutes = input.duration_minutes;
        if let Some(path) = input.new_attachment_path {
            submission.attachment_path = Some(path);
        }
        self.submission_repo.update(submission).await
    }

    /// Owner withdraws their own open submission → terminal `withdrawn`.
    pub async fn withdraw_owned(&self, actor_id: Uuid, id: Uuid) -> Result<Submission> {
        let mut submission = self.load_owned(actor_id, id).await?;
        if !submission.is_open() {
            return Err(AppError::Validation(
                "This submission is already in a final state.".to_string(),
            ));
        }
        submission.status = SubmissionStatus::Withdrawn;
        let saved = self.submission_repo.update(submission).await?;
        // Route through audit_transition (like accept/decline/start_review)
        // so the audit entry records the resulting `withdrawn` status in
        // new_value, not None.
        self.audit_transition(actor_id, &saved, "withdraw_submission")
            .await;
        Ok(saved)
    }

    /// Owner deletes their own submission — permitted ONLY from a terminal
    /// `withdrawn` or `declined` state. The attachment (if any) is deleted
    /// best-effort, then the row is removed. A non-owner gets an undisclosed
    /// 404; a delete of any other state is refused and leaves the row
    /// untouched. `uploads_dir` is the configured filesystem upload root.
    pub async fn delete(&self, actor_id: Uuid, id: Uuid, uploads_dir: &str) -> Result<()> {
        let submission = self.load_owned(actor_id, id).await?;
        if !submission.status.is_deletable() {
            return Err(AppError::Validation(
                "Only a withdrawn or declined submission can be deleted.".to_string(),
            ));
        }
        // Delete the row FIRST, guarded on status in SQL: if a concurrent
        // re-open landed between load and here, the guard matches nothing
        // and we refuse — without having touched the attachment of a now
        // -active submission.
        if !self.submission_repo.delete(id).await? {
            return Err(AppError::Validation(
                "Only a withdrawn or declined submission can be deleted.".to_string(),
            ));
        }
        // Best-effort: an unremovable file must not block the (already done)
        // row delete.
        if let Some(path) = submission.attachment_path.as_deref() {
            let _ = crate::web::uploads::delete_uploaded_file(uploads_dir, path).await;
        }
        self.audit_service
            .log(
                Some(actor_id),
                "delete_submission",
                "submission",
                &id.to_string(),
                Some(submission.status.as_wire()),
                None,
                None,
            )
            .await;
        Ok(())
    }

    /// Owner re-opens their own `withdrawn` submission back to `submitted`
    /// for revision/resubmission. Allowed ONLY from `withdrawn` — a
    /// `declined` decision is preserved (make a fresh submission instead).
    pub async fn reopen(&self, actor_id: Uuid, id: Uuid) -> Result<Submission> {
        let mut submission = self.load_owned(actor_id, id).await?;
        if !submission.status.is_reopenable() {
            return Err(AppError::Validation(
                "Only a withdrawn submission can be re-opened.".to_string(),
            ));
        }
        // ponytail: the per-member open cap is NOT re-checked here — the spec
        // permits re-opening a withdrawn submission unconditionally. Add a cap
        // guard if resurrection ever becomes a spam vector.
        submission.status = SubmissionStatus::Submitted;
        let saved = self.submission_repo.update(submission).await?;
        self.audit_transition(actor_id, &saved, "reopen_submission")
            .await;
        Ok(saved)
    }

    // --- Admin-facing ---------------------------------------------------

    /// `submitted → under_review`. Admin only.
    pub async fn start_review(&self, admin_id: Uuid, id: Uuid) -> Result<Submission> {
        let mut submission = self.load_any(id).await?;
        if submission.status != SubmissionStatus::Submitted {
            return Err(AppError::BadRequest(
                "Only a submitted proposal can be moved to under-review.".to_string(),
            ));
        }
        submission.status = SubmissionStatus::UnderReview;
        let saved = self.submission_repo.update(submission).await?;
        self.audit_transition(admin_id, &saved, "review_submission")
            .await;
        Ok(saved)
    }

    /// Decline a submission. Admin only; from `submitted`/`under_review`.
    pub async fn decline(
        &self,
        admin_id: Uuid,
        id: Uuid,
        reviewer_note: Option<String>,
    ) -> Result<Submission> {
        validate_reviewer_note(reviewer_note.as_deref())?;
        let mut submission = self.load_any(id).await?;
        require_decidable(&submission)?;
        submission.status = SubmissionStatus::Declined;
        submission.decided_by = Some(admin_id);
        submission.reviewer_note = reviewer_note;
        let saved = self.submission_repo.update(submission).await?;
        self.audit_transition(admin_id, &saved, "decline_submission")
            .await;
        Ok(saved)
    }

    /// Accept a submission. Admin only; from `submitted`/`under_review`.
    /// When a `schedule` is supplied, the submission is promoted to a
    /// standard `Event` (via the existing event path) whose visibility
    /// mirrors the accepted visibility, and the status becomes
    /// `scheduled`; otherwise it becomes `accepted`.
    pub async fn accept(
        &self,
        admin_id: Uuid,
        id: Uuid,
        reviewer_note: Option<String>,
        schedule: Option<PromoteSchedule>,
    ) -> Result<Submission> {
        validate_reviewer_note(reviewer_note.as_deref())?;
        let mut submission = self.load_any(id).await?;
        require_decidable(&submission)?;

        submission.status = SubmissionStatus::Accepted;
        submission.decided_by = Some(admin_id);
        submission.reviewer_note = reviewer_note;

        let promoted_event_id = if let Some(schedule) = schedule {
            let event = self.promote(admin_id, &submission, schedule).await?;
            submission.event_id = Some(event.id);
            submission.status = SubmissionStatus::Scheduled;
            Some(event.id)
        } else {
            None
        };

        // ponytail: promote() (above) and this update are separate writes.
        // A shared transaction would mean threading one through the whole
        // event-admin + integration-dispatch path — not worth it for a rare
        // window. If the update fails after a promotion the Event is already
        // persisted, so log its id: an admin reconciles by deleting the
        // stray event (it has no linking submission). Reconcile-by-cleanup,
        // not rollback — upgrade to a shared txn if orphans ever recur.
        let saved = match self.submission_repo.update(submission).await {
            Ok(saved) => saved,
            Err(e) => {
                if let Some(event_id) = promoted_event_id {
                    tracing::error!(
                        "Submission {} promoted to event {} but the status update failed: {}. \
                         The orphan event may need manual cleanup.",
                        id,
                        event_id,
                        e
                    );
                }
                return Err(e);
            }
        };
        self.audit_transition(admin_id, &saved, "accept_submission")
            .await;
        Ok(saved)
    }

    /// Create a standard `Event` from an accepted submission via the
    /// existing admin event path — reusing its audit + integration
    /// dispatch rather than a second event-write surface.
    async fn promote(
        &self,
        admin_id: Uuid,
        submission: &Submission,
        schedule: PromoteSchedule,
    ) -> Result<crate::domain::Event> {
        let visibility = match submission.visibility_requested {
            SubmissionVisibility::Public => EventVisibility::Public,
            SubmissionVisibility::Members => EventVisibility::MembersOnly,
        };
        // Duration → end wall-clock in the same naive container. The event
        // path derives the true instants from (wall-clock, zone).
        let end_time = schedule
            .duration_minutes
            .filter(|m| *m > 0)
            .map(|m| schedule.start + chrono::Duration::minutes(m as i64));

        let input = CreateEventInput {
            title: submission.title.clone(),
            description: submission.abstract_text.clone(),
            // Proposals promote to a Meeting-type event by default; a
            // type picker on accept is a v2 nicety.
            event_type: EventType::Meeting,
            event_type_id: None,
            visibility,
            start_time: schedule.start,
            end_time,
            timezone: schedule.timezone,
            location: None,
            max_attendees: None,
            rsvp_required: false,
            // A promoted proposal is free; an admin can price it after.
            member_price_cents: 0,
            guest_price_cents: 0,
            guest_registration_enabled: false,
            image_url: None,
            recurrence: None,
            recurrence_until: None,
        };
        self.event_admin_service.create(admin_id, input).await
    }

    // --- helpers --------------------------------------------------------

    async fn load_any(&self, id: Uuid) -> Result<Submission> {
        self.submission_repo
            .find_by_id(id)
            .await?
            .ok_or_else(not_found)
    }

    /// Load enforcing ownership — non-owners get an undisclosed 404.
    async fn load_owned(&self, actor_id: Uuid, id: Uuid) -> Result<Submission> {
        let submission = self.load_any(id).await?;
        if submission.submitter_member_id != actor_id {
            return Err(not_found());
        }
        Ok(submission)
    }

    async fn audit_transition(&self, admin_id: Uuid, submission: &Submission, action: &str) {
        self.audit_service
            .log(
                Some(admin_id),
                action,
                "submission",
                &submission.id.to_string(),
                None,
                Some(submission.status.as_wire()),
                None,
            )
            .await;
    }
}

fn not_found() -> AppError {
    AppError::NotFound("Submission not found".to_string())
}

fn require_decidable(submission: &Submission) -> Result<()> {
    match submission.status {
        SubmissionStatus::Submitted | SubmissionStatus::UnderReview => Ok(()),
        _ => Err(AppError::BadRequest(
            "This submission has already been decided or withdrawn.".to_string(),
        )),
    }
}

fn validate_title(title: &str) -> Result<()> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(AppError::Validation("Title is required.".to_string()));
    }
    if title.chars().count() > TITLE_MAX_CHARS {
        return Err(AppError::Validation(format!(
            "Title must be at most {} characters.",
            TITLE_MAX_CHARS
        )));
    }
    Ok(())
}

fn validate_abstract(abstract_text: &str) -> Result<()> {
    if abstract_text.chars().count() > ABSTRACT_MAX_CHARS {
        return Err(AppError::Validation(format!(
            "Abstract must be at most {} characters.",
            ABSTRACT_MAX_CHARS
        )));
    }
    Ok(())
}

fn validate_reviewer_note(note: Option<&str>) -> Result<()> {
    if let Some(note) = note {
        if note.chars().count() > REVIEWER_NOTE_MAX_CHARS {
            return Err(AppError::Validation(format!(
                "Reviewer note must be at most {} characters.",
                REVIEWER_NOTE_MAX_CHARS
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
