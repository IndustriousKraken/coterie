//! Member-facing proposal submissions: list own, create, view/edit,
//! withdraw, and the authorization-gated attachment download.
//!
//! Every member-supplied field is rendered through Askama auto-escaping
//! in the templates (no `|safe`), so a `<script>`-bearing title is inert
//! in the reviewer's origin — the primary threat this feature guards.
//! Ownership is enforced in the service (`get_authorized` / `update_owned`
//! / `withdraw_owned`), which denies non-owners without disclosure.

use std::path::PathBuf;
use std::sync::Arc;

use askama::Template;
use axum::{
    body::Body,
    extract::{Multipart, Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Extension,
};
use tokio::fs;
use tokio_util::io::ReaderStream;

use crate::{
    api::middleware::auth::{CurrentUser, SessionInfo},
    auth::CsrfService,
    config::Settings,
    domain::{Submission, SubmissionStatus, SubmissionVisibility},
    repository::SubmissionRepository,
    service::settings_service::SettingsService,
    service::submission_service::{
        CreateSubmissionInput, SubmissionService, UpdateSubmissionInput,
    },
    web::templates::{BaseContext, HtmlTemplate},
    web::uploads::save_uploaded_document,
};

// --- View models ---------------------------------------------------------

pub struct SubmissionRow {
    pub id: String,
    pub title: String,
    pub status_label: String,
    pub visibility: String,
    pub created_at: String,
    pub has_attachment: bool,
    /// Owner may delete (terminal `withdrawn`/`declined`).
    pub can_delete: bool,
    /// Owner may re-open (`withdrawn` only).
    pub can_reopen: bool,
}

#[derive(Template)]
#[template(path = "portal/submissions.html")]
pub struct SubmissionsTemplate {
    pub base: BaseContext,
    pub submissions: Vec<SubmissionRow>,
}

/// Detail view. Title/abstract/reviewer_note are rendered through Askama
/// auto-escaping in the template — never `|safe`.
pub struct SubmissionDetail {
    pub id: String,
    pub title: String,
    pub abstract_text: String,
    pub visibility: String,
    pub status_label: String,
    pub is_editable: bool,
    pub is_open: bool,
    pub can_delete: bool,
    pub can_reopen: bool,
    pub reviewer_note: Option<String>,
    pub attachment_path: Option<String>,
    pub preferred_start_input: String,
    pub duration_minutes: Option<i32>,
    pub created_at: String,
    pub updated_at: String,
    pub event_id: Option<String>,
}

#[derive(Template)]
#[template(path = "portal/submission_detail.html")]
pub struct SubmissionDetailTemplate {
    pub base: BaseContext,
    pub submission: SubmissionDetail,
}

#[derive(Template)]
#[template(path = "portal/submission_new.html")]
pub struct NewSubmissionTemplate {
    pub base: BaseContext,
}

// --- Helpers -------------------------------------------------------------

/// Parse an HTML `datetime-local` value (`YYYY-MM-DDTHH:MM`[`:SS`]) into a
/// naive-wall-clock `DateTime<Utc>` container. Empty/invalid → None.
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

fn to_detail(s: Submission) -> SubmissionDetail {
    let is_editable = s.status == SubmissionStatus::Submitted;
    let is_open = s.is_open();
    let can_delete = matches!(
        s.status,
        SubmissionStatus::Withdrawn | SubmissionStatus::Declined
    );
    let can_reopen = s.status == SubmissionStatus::Withdrawn;
    SubmissionDetail {
        id: s.id.to_string(),
        title: s.title,
        abstract_text: s.abstract_text,
        visibility: s.visibility_requested.as_wire().to_string(),
        status_label: s.status.label().to_string(),
        is_editable,
        is_open,
        can_delete,
        can_reopen,
        reviewer_note: s.reviewer_note,
        attachment_path: s.attachment_path,
        preferred_start_input: s
            .preferred_start
            .map(|dt| dt.format("%Y-%m-%dT%H:%M").to_string())
            .unwrap_or_default(),
        duration_minutes: s.duration_minutes,
        created_at: s.created_at.format("%b %d, %Y %H:%M").to_string(),
        updated_at: s.updated_at.format("%b %d, %Y %H:%M").to_string(),
        event_id: s.event_id.map(|e| e.to_string()),
    }
}

/// Parsed submission form fields (create + edit share the same shape).
struct SubmissionForm {
    title: String,
    abstract_text: String,
    visibility: SubmissionVisibility,
    preferred_start: Option<chrono::DateTime<chrono::Utc>>,
    duration_minutes: Option<i32>,
    /// Freshly-saved attachment path, or an upload error to surface.
    attachment_path: Option<String>,
    upload_error: Option<String>,
}

async fn parse_submission_form(uploads_dir: &str, multipart: &mut Multipart) -> SubmissionForm {
    let mut form = SubmissionForm {
        title: String::new(),
        abstract_text: String::new(),
        visibility: SubmissionVisibility::Members,
        preferred_start: None,
        duration_minutes: None,
        attachment_path: None,
        upload_error: None,
    };
    let mut preferred_start_str = String::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "csrf_token" => {
                let _ = field.text().await;
            }
            "title" => form.title = field.text().await.unwrap_or_default(),
            "abstract" => form.abstract_text = field.text().await.unwrap_or_default(),
            "visibility" => {
                form.visibility =
                    SubmissionVisibility::from_wire(&field.text().await.unwrap_or_default());
            }
            "preferred_start" => {
                preferred_start_str = field.text().await.unwrap_or_default();
            }
            "duration_minutes" => {
                let v = field.text().await.unwrap_or_default();
                form.duration_minutes = v.trim().parse::<i32>().ok().filter(|d| *d > 0);
            }
            "attachment" => {
                let filename = field.file_name().unwrap_or("").to_string();
                if !filename.is_empty() {
                    if let Ok(data) = field.bytes().await {
                        if !data.is_empty() {
                            match save_uploaded_document(uploads_dir, &data).await {
                                Ok(path) => form.attachment_path = Some(path),
                                Err(e) => form.upload_error = Some(e.to_string()),
                            }
                        }
                    }
                }
            }
            _ => {
                let _ = field.bytes().await;
            }
        }
    }
    form.preferred_start = parse_local_datetime(&preferred_start_str);
    form
}

// --- Handlers ------------------------------------------------------------

pub async fn submissions_page(
    State(submission_repo): State<Arc<dyn SubmissionRepository>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session): Extension<SessionInfo>,
) -> Response {
    // Surface a DB failure as a 500 rather than an empty list — silently
    // showing "no submissions" would hide a real outage from the member.
    let rows = match submission_repo.list_for_member(current_user.member.id).await {
        Ok(rows) => rows,
        Err(e) => return e.into_response(),
    };
    let submissions = rows
        .into_iter()
        .map(|s| SubmissionRow {
            id: s.id.to_string(),
            title: s.title,
            status_label: s.status.label().to_string(),
            visibility: s.visibility_requested.as_wire().to_string(),
            created_at: s.created_at.format("%b %d, %Y").to_string(),
            has_attachment: s.attachment_path.is_some(),
            can_delete: matches!(
                s.status,
                SubmissionStatus::Withdrawn | SubmissionStatus::Declined
            ),
            can_reopen: s.status == SubmissionStatus::Withdrawn,
        })
        .collect();
    let base = BaseContext::for_member(&csrf_service, &current_user, &session).await;
    HtmlTemplate(SubmissionsTemplate { base, submissions }).into_response()
}

pub async fn new_submission_page(
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session): Extension<SessionInfo>,
) -> impl IntoResponse {
    let base = BaseContext::for_member(&csrf_service, &current_user, &session).await;
    HtmlTemplate(NewSubmissionTemplate { base })
}

pub async fn create_submission(
    State(settings): State<Arc<Settings>>,
    State(settings_service): State<Arc<SettingsService>>,
    State(submission_service): State<Arc<SubmissionService>>,
    Extension(current_user): Extension<CurrentUser>,
    mut multipart: Multipart,
) -> Response {
    let uploads_dir = settings.server.uploads_path();
    let form = parse_submission_form(&uploads_dir, &mut multipart).await;
    if let Some(err) = form.upload_error {
        return (StatusCode::UNPROCESSABLE_ENTITY, err).into_response();
    }
    let timezone = settings_service.org_timezone().await.name().to_string();

    let input = CreateSubmissionInput {
        title: form.title,
        abstract_text: form.abstract_text,
        visibility_requested: form.visibility,
        attachment_path: form.attachment_path.clone(),
        preferred_start: form.preferred_start,
        timezone,
        duration_minutes: form.duration_minutes,
    };

    match submission_service
        .create(current_user.member.id, input)
        .await
    {
        Ok(created) => Redirect::to(&format!("/portal/submissions/{}", created.id)).into_response(),
        Err(e) => {
            // Roll back an orphaned upload if the create failed after save.
            crate::web::uploads::delete_if_upload(&uploads_dir, form.attachment_path.as_deref())
                .await;
            (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()).into_response()
        }
    }
}

pub async fn submission_detail_page(
    State(submission_service): State<Arc<SubmissionService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session): Extension<SessionInfo>,
    Path(id): Path<String>,
) -> Response {
    let id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let submission = match submission_service
        .get_authorized(current_user.member.id, current_user.member.is_admin, id)
        .await
    {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    let base = BaseContext::for_member(&csrf_service, &current_user, &session).await;
    HtmlTemplate(SubmissionDetailTemplate {
        base,
        submission: to_detail(submission),
    })
    .into_response()
}

pub async fn update_submission(
    State(settings): State<Arc<Settings>>,
    State(settings_service): State<Arc<SettingsService>>,
    State(submission_service): State<Arc<SubmissionService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(id): Path<String>,
    mut multipart: Multipart,
) -> Response {
    let id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let uploads_dir = settings.server.uploads_path();
    let form = parse_submission_form(&uploads_dir, &mut multipart).await;
    if let Some(err) = form.upload_error {
        return (StatusCode::UNPROCESSABLE_ENTITY, err).into_response();
    }
    let timezone = settings_service.org_timezone().await.name().to_string();

    let input = UpdateSubmissionInput {
        title: form.title,
        abstract_text: form.abstract_text,
        visibility_requested: form.visibility,
        preferred_start: form.preferred_start,
        timezone,
        duration_minutes: form.duration_minutes,
        new_attachment_path: form.attachment_path.clone(),
    };

    match submission_service
        .update_owned(current_user.member.id, id, input)
        .await
    {
        Ok(_) => Redirect::to(&format!("/portal/submissions/{}", id)).into_response(),
        Err(e) => {
            crate::web::uploads::delete_if_upload(&uploads_dir, form.attachment_path.as_deref())
                .await;
            e.into_response()
        }
    }
}

pub async fn withdraw_submission(
    State(submission_service): State<Arc<SubmissionService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Response {
    let id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    match submission_service
        .withdraw_owned(current_user.member.id, id)
        .await
    {
        Ok(_) => Redirect::to(&format!("/portal/submissions/{}", id)).into_response(),
        Err(e) => e.into_response(),
    }
}

/// Owner deletes their own terminal (`withdrawn`/`declined`) submission.
/// Ownership + state guard live in the service; a refusal returns its
/// error unchanged. On success the row is gone, so redirect to the list.
pub async fn delete_submission(
    State(settings): State<Arc<Settings>>,
    State(submission_service): State<Arc<SubmissionService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Response {
    let id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    match submission_service
        .delete(current_user.member.id, id, &settings.server.uploads_path())
        .await
    {
        Ok(_) => Redirect::to("/portal/submissions").into_response(),
        Err(e) => e.into_response(),
    }
}

/// Owner re-opens their own `withdrawn` submission back to `submitted`.
pub async fn reopen_submission(
    State(submission_service): State<Arc<SubmissionService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Response {
    let id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    match submission_service
        .reopen(current_user.member.id, id)
        .await
    {
        Ok(_) => Redirect::to(&format!("/portal/submissions/{}", id)).into_response(),
        Err(e) => e.into_response(),
    }
}

/// Authorization-gated attachment download. Permits the submitter and any
/// admin — OR anyone, if the submission was accepted with `public`
/// visibility. Always served `Content-Disposition: attachment` +
/// `X-Content-Type-Options: nosniff` so a PDF never renders inline in the
/// viewer's origin. Non-public attachments are NEVER served via the
/// public `/uploads/:filename` route.
pub async fn download_attachment(
    State(settings): State<Arc<Settings>>,
    State(submission_repo): State<Arc<dyn SubmissionRepository>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Response {
    let id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let submission = match submission_repo.find_by_id(id).await {
        Ok(Some(s)) => s,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };

    let is_owner = submission.submitter_member_id == current_user.member.id;
    let is_admin = current_user.member.is_admin;
    let accepted_public = matches!(
        submission.status,
        SubmissionStatus::Accepted | SubmissionStatus::Scheduled
    ) && submission.visibility_requested == SubmissionVisibility::Public;

    if !(is_owner || is_admin || accepted_public) {
        // Deny without disclosure.
        return StatusCode::NOT_FOUND.into_response();
    }

    let Some(stored) = submission.attachment_path else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // Stored path is `uploads/<name>`; anything else isn't ours.
    let Some(filename) = stored.strip_prefix("uploads/") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if filename.is_empty()
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains("..")
    {
        return StatusCode::NOT_FOUND.into_response();
    }

    let path = PathBuf::from(settings.server.uploads_path()).join(filename);
    let file = match fs::File::open(&path).await {
        Ok(f) => f,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/pdf"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"submission.pdf\"",
            ),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        body,
    )
        .into_response()
}
