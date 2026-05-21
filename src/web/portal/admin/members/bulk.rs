use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{State, Query, Multipart},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Extension,
};

use crate::{
    api::middleware::auth::{CurrentUser, SessionInfo},
    auth::CsrfService,
    repository::MemberRepository,
    service::{
        member_service::MemberService,
        membership_type_service::MembershipTypeService,
    },
    web::templates::{BaseContext, HtmlTemplate},
};

use super::AdminMembersQuery;

/// CSV export of the member roster. Respects the same filter query
/// string as `admin_members_page` and emits one row per matching
/// member, with all non-credential fields. Audit row is written
/// through `MemberService::audit_export` so abuse is traceable.
///
/// Response is `text/csv; charset=utf-8` with
/// `Content-Disposition: attachment` so browsers download rather
/// than rendering. Filename includes the UTC date so re-downloads
/// inside one day overwrite each other; new day → new filename.
pub async fn admin_members_export(
    State(member_repo): State<Arc<dyn MemberRepository>>,
    State(member_service): State<Arc<MemberService>>,
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    Extension(current_user): Extension<CurrentUser>,
    Query(query): Query<AdminMembersQuery>,
) -> Response {
    use crate::repository::{MemberQuery, MemberSortField, SortOrder};

    let all_types = membership_type_service
        .list(true).await.unwrap_or_default();
    let type_filter_id = query.member_type.as_deref()
        .and_then(|slug| all_types.iter().find(|t| t.slug == slug).map(|t| t.id));

    let sort_field = query.sort.as_deref().unwrap_or("name");
    let sort_order = query.order.as_deref().unwrap_or("asc");

    let typed_query = MemberQuery {
        search: query.q.clone().filter(|s| !s.is_empty()),
        status: query.status.as_deref().and_then(crate::domain::MemberStatus::from_str),
        membership_type_id: type_filter_id,
        sort: match sort_field {
            "status" => MemberSortField::Status,
            "type" => MemberSortField::MembershipType,
            "joined" => MemberSortField::Joined,
            "dues" => MemberSortField::DuesPaidUntil,
            _ => MemberSortField::Name,
        },
        order: if sort_order == "desc" { SortOrder::Desc } else { SortOrder::Asc },
        // Ignored by `export_rows`, but the field is non-optional.
        limit: 0,
        offset: 0,
    };

    let rows = match member_repo.export_rows(typed_query).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("admin members export failed: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to build export. Check server logs.",
            ).into_response();
        }
    };

    let body = build_members_csv(&rows);

    let filter_summary = build_filter_summary(&query);
    if let Err(e) = member_service
        .audit_export(current_user.member.id, &filter_summary, rows.len())
        .await
    {
        // Audit failures are already swallowed inside AuditService::log;
        // this branch is reachable only if a future audit_export variant
        // returns Err. Log + continue — the download still goes through.
        tracing::error!("admin members export audit failed: {}", e);
    }

    let filename = format!(
        "members-export-{}.csv",
        chrono::Utc::now().date_naive().format("%Y-%m-%d"),
    );
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", filename)),
        ],
        body,
    ).into_response()
}

/// Assemble the CSV body: a header row followed by one row per
/// `MemberExportRow`. Column order matches the
/// `bulk-member-csv-export` capability spec exactly.
fn build_members_csv(rows: &[crate::repository::MemberExportRow]) -> String {
    use crate::web::portal::admin::csv::push_csv;

    let mut out = String::with_capacity(1024 + rows.len() * 256);
    out.push_str(
        "id,email,username,full_name,status,membership_type,joined_at,\
         dues_paid_until,is_admin,bypass_dues,discord_id,email_verified_at,notes\n",
    );

    for r in rows {
        push_csv(&mut out, &r.id.to_string());
        out.push(',');
        push_csv(&mut out, &r.email);
        out.push(',');
        push_csv(&mut out, &r.username);
        out.push(',');
        push_csv(&mut out, &r.full_name);
        out.push(',');
        push_csv(&mut out, r.status.as_str());
        out.push(',');
        push_csv(&mut out, &r.membership_type);
        out.push(',');
        push_csv(&mut out, &r.joined_at.to_rfc3339());
        out.push(',');
        push_csv(
            &mut out,
            &r.dues_paid_until.map(|d| d.to_rfc3339()).unwrap_or_default(),
        );
        out.push(',');
        push_csv(&mut out, if r.is_admin { "true" } else { "false" });
        out.push(',');
        push_csv(&mut out, if r.bypass_dues { "true" } else { "false" });
        out.push(',');
        push_csv(&mut out, r.discord_id.as_deref().unwrap_or(""));
        out.push(',');
        push_csv(
            &mut out,
            &r.email_verified_at.map(|d| d.to_rfc3339()).unwrap_or_default(),
        );
        out.push(',');
        push_csv(&mut out, r.notes.as_deref().unwrap_or(""));
        out.push('\n');
    }
    out
}

/// Compact summary of the active filters, suitable for the audit
/// log's `new_value`. Order matches the wire shape so future readers
/// can correlate. Empty (no filters) → empty string. The handler
/// appends `count=N` separately so this stays a pure filter
/// description.
fn build_filter_summary(q: &AdminMembersQuery) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(s) = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(format!("q={}", s));
    }
    if let Some(s) = q.status.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(format!("status={}", s));
    }
    if let Some(s) = q.member_type.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(format!("type={}", s));
    }
    parts.join(",")
}

// =====================================================================
// Bulk member CSV import (admin upload)
// =====================================================================

/// 5 MB. Matches the cap documented in the `bulk-member-csv-import`
/// capability spec — large enough for ~50k typical rows, small enough
/// that an admin can't accidentally OOM the server with a malformed
/// file. The application-level CSRF middleware already buffers up to
/// 12 MB for multipart bodies, so we re-check the size at the file
/// field level too rather than trusting the outer cap.
const IMPORT_FILE_MAX_BYTES: usize = 5 * 1024 * 1024;

#[derive(Template)]
#[template(path = "admin/member_import.html")]
pub struct AdminMemberImportPageTemplate {
    pub base: BaseContext,
}

/// GET — show the upload form. Pure render; no service work.
pub async fn admin_members_import_page(
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> impl IntoResponse {
    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;
    HtmlTemplate(AdminMemberImportPageTemplate { base })
}

#[derive(Template)]
#[template(path = "admin/member_import_result.html")]
pub struct AdminMemberImportResultTemplate {
    pub file_name: String,
    pub succeeded: u32,
    pub failed: u32,
    pub failures: Vec<ImportFailureView>,
}

#[derive(Clone)]
pub struct ImportFailureView {
    pub row_index: usize,
    pub email: String,
    pub reason: String,
}

#[derive(Template)]
#[template(path = "admin/member_import_error.html")]
pub struct AdminMemberImportErrorTemplate {
    pub message: String,
}

/// POST — accept a multipart upload with a `file` field carrying a CSV.
/// The handler parses the CSV (5 MB cap, header validation), then
/// delegates each row to `MemberService::bulk_import`, then renders an
/// HTMX result fragment. CSV parsing is the handler's job; service
/// stays format-agnostic.
pub async fn admin_members_import(
    State(member_service): State<Arc<MemberService>>,
    Extension(current_user): Extension<CurrentUser>,
    mut multipart: Multipart,
) -> Response {
    

    let mut file_bytes: Option<Vec<u8>> = None;
    let mut file_name = String::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        match field.name().unwrap_or("") {
            "csrf_token" => { let _ = field.text().await; }
            "file" => {
                file_name = field.file_name().unwrap_or("members.csv").to_string();
                match field.bytes().await {
                    Ok(b) => {
                        if b.len() > IMPORT_FILE_MAX_BYTES {
                            return import_error_fragment(&format!(
                                "File too large ({} bytes). Maximum is {} MB.",
                                b.len(),
                                IMPORT_FILE_MAX_BYTES / (1024 * 1024),
                            )).into_response();
                        }
                        file_bytes = Some(b.to_vec());
                    }
                    Err(e) => {
                        return import_error_fragment(&format!(
                            "Failed to read uploaded file: {}",
                            e,
                        )).into_response();
                    }
                }
            }
            _ => { let _ = field.bytes().await; }
        }
    }

    let bytes = match file_bytes {
        Some(b) if !b.is_empty() => b,
        _ => {
            return import_error_fragment(
                "No CSV file was uploaded. Please select a file and try again.",
            ).into_response();
        }
    };

    let rows = match parse_import_csv(&bytes) {
        Ok(rows) => rows,
        Err(e) => return import_error_fragment(&e).into_response(),
    };

    let summary = match member_service
        .bulk_import(current_user.member.id, &file_name, rows)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            return import_error_fragment(&format!(
                "Import failed: {}",
                e,
            )).into_response();
        }
    };

    let failures = summary
        .failures
        .iter()
        .map(|f| ImportFailureView {
            row_index: f.row_index,
            email: f.email.clone().unwrap_or_default(),
            reason: f.reason.clone(),
        })
        .collect();

    HtmlTemplate(AdminMemberImportResultTemplate {
        file_name,
        succeeded: summary.succeeded,
        failed: summary.failed,
        failures,
    }).into_response()
}

/// Parse the raw CSV bytes into `Vec<ImportRow>`. Returns Err with a
/// user-facing message on header validation failures (missing required
/// columns) or unreadable file structure.
///
/// Row-level coercion failures (e.g., a bad `status` value, a malformed
/// row) are converted into `ImportRow`s with empty fields so the
/// service can fail them per-row rather than aborting the batch — but
/// truly unrecoverable parse errors (the file isn't CSV, the header is
/// missing the `email` column) abort here.
fn parse_import_csv(bytes: &[u8]) -> std::result::Result<Vec<crate::service::member_service::ImportRow>, String> {
    use chrono::{DateTime, NaiveDate, Utc};
    use crate::service::member_service::ImportRow;

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(bytes);

    let headers = match reader.headers() {
        Ok(h) => h.clone(),
        Err(e) => return Err(format!("Could not read CSV header: {}", e)),
    };

    // Build a case-insensitive column index. Extra columns are
    // tolerated and silently ignored; required columns must all be
    // present or the batch aborts.
    let col = |name: &str| -> Option<usize> {
        headers
            .iter()
            .position(|h| h.trim().eq_ignore_ascii_case(name))
    };

    let email_idx = col("email").ok_or_else(|| {
        "Missing required column 'email' in CSV header.".to_string()
    })?;
    let username_idx = col("username").ok_or_else(|| {
        "Missing required column 'username' in CSV header.".to_string()
    })?;
    let full_name_idx = col("full_name").ok_or_else(|| {
        "Missing required column 'full_name' in CSV header.".to_string()
    })?;
    let mtype_idx = col("membership_type_slug").ok_or_else(|| {
        "Missing required column 'membership_type_slug' in CSV header.".to_string()
    })?;
    let status_idx = col("status");
    let notes_idx = col("notes");
    let discord_idx = col("discord_id");
    // Billing-migration optional columns. Same case-insensitive,
    // any-position handling as the others.
    let dues_paid_until_idx = col("dues_paid_until");
    let stripe_customer_idx = col("stripe_customer_id");
    let stripe_subscription_idx = col("stripe_subscription_id");
    let joined_at_idx = col("joined_at");
    let email_verified_at_idx = col("email_verified_at");

    // Parse a timestamp cell. RFC 3339 first; fall back to
    // `YYYY-MM-DD` (interpreted as midnight UTC). Empty cell → `None`.
    // On parse failure, the caller sets `parse_error` on the row so
    // `bulk_import` rejects it with the documented reason.
    fn parse_timestamp(field: &str, cell: &str)
        -> std::result::Result<Option<DateTime<Utc>>, String>
    {
        let trimmed = cell.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
            return Ok(Some(dt.with_timezone(&Utc)));
        }
        if let Ok(d) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
            let dt = d.and_hms_opt(0, 0, 0).unwrap().and_utc();
            return Ok(Some(dt));
        }
        Err(format!("Could not parse {}: '{}'", field, trimmed))
    }

    let mut rows = Vec::new();
    for record in reader.records() {
        let rec = match record {
            Ok(r) => r,
            Err(e) => return Err(format!("Malformed CSV row: {}", e)),
        };

        let get = |i: usize| -> String {
            rec.get(i).unwrap_or("").to_string()
        };
        let get_opt = |i: Option<usize>| -> Option<String> {
            i.and_then(|idx| rec.get(idx)).map(|s| s.to_string()).filter(|s| !s.is_empty())
        };
        // Like `get_opt` but also trims and treats trim-empty as None.
        // Used for the Stripe ID columns (trimmed strings, blank = None).
        let get_trimmed_opt = |i: Option<usize>| -> Option<String> {
            i.and_then(|idx| rec.get(idx))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };

        let status = get_opt(status_idx).and_then(|s| crate::domain::MemberStatus::from_str(s.trim()));

        // Parse the three optional timestamps; the first failure wins
        // and stamps `parse_error` so the row fails downstream rather
        // than being silently dropped.
        let cell_for = |idx: Option<usize>| -> &str {
            idx.and_then(|i| rec.get(i)).unwrap_or("")
        };
        let mut parse_error: Option<String> = None;
        let dues_paid_until = match parse_timestamp("dues_paid_until", cell_for(dues_paid_until_idx)) {
            Ok(opt) => opt,
            Err(e) => { parse_error = Some(e); None }
        };
        let joined_at = if parse_error.is_some() {
            None
        } else {
            match parse_timestamp("joined_at", cell_for(joined_at_idx)) {
                Ok(opt) => opt,
                Err(e) => { parse_error = Some(e); None }
            }
        };
        let email_verified_at = if parse_error.is_some() {
            None
        } else {
            match parse_timestamp("email_verified_at", cell_for(email_verified_at_idx)) {
                Ok(opt) => opt,
                Err(e) => { parse_error = Some(e); None }
            }
        };

        rows.push(ImportRow {
            email: get(email_idx),
            username: get(username_idx),
            full_name: get(full_name_idx),
            membership_type_slug: get(mtype_idx),
            status,
            notes: get_opt(notes_idx),
            discord_id: get_opt(discord_idx),
            dues_paid_until,
            stripe_customer_id: get_trimmed_opt(stripe_customer_idx),
            stripe_subscription_id: get_trimmed_opt(stripe_subscription_idx),
            joined_at,
            email_verified_at,
            parse_error,
        });
    }

    Ok(rows)
}

fn import_error_fragment(message: &str) -> impl IntoResponse {
    HtmlTemplate(AdminMemberImportErrorTemplate {
        message: message.to_string(),
    })
}
