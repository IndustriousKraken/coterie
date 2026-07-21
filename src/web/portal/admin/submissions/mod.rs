//! Admin review surface for member proposal submissions: the review
//! queue, per-submission detail, and the audited status transitions
//! (`review` / `accept` / `decline`). Accept may carry a schedule, which
//! promotes the proposal to a standard `Event` via the existing event
//! path.
//!
//! Member-supplied `title` / `abstract` / `reviewer_note` are rendered
//! through Askama auto-escaping in the templates (no `|safe`), so a
//! script-bearing title is inert in the reviewer's authenticated origin.

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
    Extension, Form,
};
use serde::Deserialize;

use crate::{
    api::middleware::auth::{CurrentUser, SessionInfo},
    auth::CsrfService,
    domain::{Submission, SubmissionStatus},
    repository::{MemberRepository, SubmissionRepository},
    service::settings_service::SettingsService,
    service::submission_service::{PromoteSchedule, SubmissionService},
    web::templates::{BaseContext, HtmlTemplate},
};

pub struct AdminSubmissionRow {
    pub id: String,
    pub title: String,
    pub submitter_name: String,
    pub status_label: String,
    pub visibility: String,
    pub created_at: String,
    pub has_attachment: bool,
}

#[derive(Template)]
#[template(path = "admin/submissions.html")]
pub struct AdminSubmissionsTemplate {
    pub base: BaseContext,
    pub submissions: Vec<AdminSubmissionRow>,
}

pub struct AdminSubmissionDetail {
    pub id: String,
    pub submitter_name: String,
    pub submitter_email: String,
    pub title: String,
    pub abstract_text: String,
    pub visibility: String,
    pub status_label: String,
    pub is_submitted: bool,
    pub is_decidable: bool,
    pub reviewer_note: Option<String>,
    pub attachment_path: Option<String>,
    pub schedule_default: String,
    pub duration_minutes: Option<i32>,
    pub created_at: String,
    pub updated_at: String,
    pub event_id: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/submission_detail.html")]
pub struct AdminSubmissionDetailTemplate {
    pub base: BaseContext,
    pub submission: AdminSubmissionDetail,
}

#[derive(Debug, Deserialize)]
pub struct DecisionForm {
    // The form's `csrf_token` field is validated by the top-level CSRF
    // middleware before the handler runs; `serde_urlencoded` ignores the
    // extra body field, so it is intentionally absent from this struct.
    pub reviewer_note: Option<String>,
    /// Present on the accept form only: an optional `datetime-local`
    /// schedule. When non-empty, the accepted proposal is promoted to an
    /// Event.
    pub schedule_start: Option<String>,
    pub duration_minutes: Option<String>,
}

async fn submitter_name(
    member_repo: &Arc<dyn MemberRepository>,
    s: &Submission,
) -> (String, String) {
    match member_repo.find_by_id(s.submitter_member_id).await {
        Ok(Some(m)) => (m.full_name, m.email),
        _ => ("(unknown member)".to_string(), String::new()),
    }
}

pub async fn admin_submissions_page(
    State(submission_repo): State<Arc<dyn SubmissionRepository>>,
    State(member_repo): State<Arc<dyn MemberRepository>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session): Extension<SessionInfo>,
) -> impl IntoResponse {
    let base = BaseContext::for_member(&csrf_service, &current_user, &session).await;
    let rows = submission_repo.list_for_review().await.unwrap_or_default();
    let mut submissions = Vec::with_capacity(rows.len());
    for s in rows {
        let (name, _) = submitter_name(&member_repo, &s).await;
        submissions.push(AdminSubmissionRow {
            id: s.id.to_string(),
            title: s.title,
            submitter_name: name,
            status_label: s.status.label().to_string(),
            visibility: s.visibility_requested.as_wire().to_string(),
            created_at: s.created_at.format("%b %d, %Y").to_string(),
            has_attachment: s.attachment_path.is_some(),
        });
    }
    HtmlTemplate(AdminSubmissionsTemplate { base, submissions })
}

pub async fn admin_submission_detail_page(
    State(submission_service): State<Arc<SubmissionService>>,
    State(member_repo): State<Arc<dyn MemberRepository>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session): Extension<SessionInfo>,
    Path(id): Path<String>,
) -> Response {
    let id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return axum::http::StatusCode::NOT_FOUND.into_response(),
    };
    // Admins are exempt from ownership — pass is_admin so review works.
    let s = match submission_service
        .get_authorized(current_user.member.id, true, id)
        .await
    {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    let (name, email) = submitter_name(&member_repo, &s).await;
    let base = BaseContext::for_member(&csrf_service, &current_user, &session).await;
    let detail = AdminSubmissionDetail {
        id: s.id.to_string(),
        submitter_name: name,
        submitter_email: email,
        title: s.title,
        abstract_text: s.abstract_text,
        visibility: s.visibility_requested.as_wire().to_string(),
        status_label: s.status.label().to_string(),
        is_submitted: s.status == SubmissionStatus::Submitted,
        is_decidable: matches!(
            s.status,
            SubmissionStatus::Submitted | SubmissionStatus::UnderReview
        ),
        reviewer_note: s.reviewer_note,
        attachment_path: s.attachment_path,
        schedule_default: s
            .preferred_start
            .map(|dt| dt.format("%Y-%m-%dT%H:%M").to_string())
            .unwrap_or_default(),
        duration_minutes: s.duration_minutes,
        created_at: s.created_at.format("%b %d, %Y %H:%M").to_string(),
        updated_at: s.updated_at.format("%b %d, %Y %H:%M").to_string(),
        event_id: s.event_id.map(|e| e.to_string()),
    };
    HtmlTemplate(AdminSubmissionDetailTemplate {
        base,
        submission: detail,
    })
    .into_response()
}

pub async fn admin_start_review(
    State(submission_service): State<Arc<SubmissionService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Response {
    let id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return axum::http::StatusCode::NOT_FOUND.into_response(),
    };
    match submission_service
        .start_review(current_user.member.id, id)
        .await
    {
        Ok(_) => Redirect::to(&format!("/portal/admin/submissions/{}", id)).into_response(),
        Err(e) => e.into_response(),
    }
}

pub async fn admin_accept_submission(
    State(submission_service): State<Arc<SubmissionService>>,
    State(settings_service): State<Arc<SettingsService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(id): Path<String>,
    Form(form): Form<DecisionForm>,
) -> Response {
    let id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return axum::http::StatusCode::NOT_FOUND.into_response(),
    };
    let reviewer_note = form.reviewer_note.filter(|s| !s.trim().is_empty());

    // Build a schedule only when the admin supplied a start time.
    let schedule = match form
        .schedule_start
        .as_deref()
        .and_then(parse_local_datetime)
    {
        Some(start) => {
            let timezone = settings_service.org_timezone().await.name().to_string();
            let duration_minutes = form
                .duration_minutes
                .as_deref()
                .and_then(|v| v.trim().parse::<i32>().ok())
                .filter(|d| *d > 0);
            Some(PromoteSchedule {
                start,
                timezone,
                duration_minutes,
            })
        }
        None => None,
    };

    match submission_service
        .accept(current_user.member.id, id, reviewer_note, schedule)
        .await
    {
        Ok(_) => Redirect::to(&format!("/portal/admin/submissions/{}", id)).into_response(),
        Err(e) => e.into_response(),
    }
}

pub async fn admin_decline_submission(
    State(submission_service): State<Arc<SubmissionService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(id): Path<String>,
    Form(form): Form<DecisionForm>,
) -> Response {
    let id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return axum::http::StatusCode::NOT_FOUND.into_response(),
    };
    let reviewer_note = form.reviewer_note.filter(|s| !s.trim().is_empty());
    match submission_service
        .decline(current_user.member.id, id, reviewer_note)
        .await
    {
        Ok(_) => Redirect::to(&format!("/portal/admin/submissions/{}", id)).into_response(),
        Err(e) => e.into_response(),
    }
}

/// Parse an HTML `datetime-local` value into a naive-wall-clock
/// `DateTime<Utc>` container. Empty/invalid → None.
fn parse_local_datetime(raw: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parsed = chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S"))
        .ok()?;
    Some(chrono::DateTime::from_naive_utc_and_offset(
        parsed,
        chrono::Utc,
    ))
}
