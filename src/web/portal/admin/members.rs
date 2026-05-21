use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{State, Query, Path, Multipart},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Extension,
};
use serde::Deserialize;

use crate::{
    api::middleware::auth::{CurrentUser, SessionInfo},
    auth::CsrfService,
    repository::{DonationCampaignRepository, MemberRepository, PaymentRepository, SavedCardRepository},
    service::{
        member_service::MemberService,
        membership_type_service::MembershipTypeService, payment_service::PaymentService,
    },
    web::{portal::admin::partials, templates::{BaseContext, HtmlTemplate, filters}},
};

#[derive(Template)]
#[template(path = "admin/members.html")]
pub struct AdminMembersTemplate {
    pub base: BaseContext,
    pub members: Vec<AdminMemberInfo>,
    pub total_members: i64,
    pub current_page: i64,
    pub per_page: i64,
    pub total_pages: i64,
    pub search_query: String,
    pub status_filter: String,
    pub type_filter: String,
    pub type_options: Vec<MembershipTypeOption>,
    pub sort_field: String,
    pub sort_order: String,
}

#[derive(Template)]
#[template(path = "admin/members_table.html")]
pub struct AdminMembersTableTemplate {
    pub members: Vec<AdminMemberInfo>,
    pub total_members: i64,
    pub current_page: i64,
    pub per_page: i64,
    pub total_pages: i64,
    pub search_query: String,
    pub status_filter: String,
    pub type_filter: String,
    pub sort_field: String,
    pub sort_order: String,
}

#[derive(Clone)]
pub struct MembershipTypeOption {
    pub id: String,
    pub slug: String,
    pub name: String,
}

#[derive(Clone)]
pub struct AdminMemberInfo {
    pub id: uuid::Uuid,
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub initials: String,
    pub status: crate::domain::MemberStatus,
    pub membership_type: String,
    pub joined_at: chrono::DateTime<chrono::Utc>,
    pub dues_paid_until: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct AdminMembersQuery {
    pub q: Option<String>,
    pub status: Option<String>,
    #[serde(rename = "type")]
    pub member_type: Option<String>,
    pub page: Option<i64>,
    pub sort: Option<String>,
    pub order: Option<String>,
}

pub async fn admin_members_page(
    State(member_repo): State<Arc<dyn MemberRepository>>,
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    headers: axum::http::HeaderMap,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Query(query): Query<AdminMembersQuery>,
) -> impl IntoResponse {
    let is_htmx = headers.get("HX-Request").is_some();

    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;

    let page = query.page.unwrap_or(1).max(1);
    let per_page: i64 = 20;
    let offset = (page - 1) * per_page;

    let sort_field = query.sort.clone().unwrap_or_else(|| "name".to_string());
    let sort_order = query.order.clone().unwrap_or_else(|| "asc".to_string());

    // Load all membership types up front: drives the filter dropdown,
    // resolves the URL `?type=<slug>` filter to its FK, and provides
    // the per-row display name.
    let all_types = membership_type_service
        .list(true).await
        .unwrap_or_else(|e| {
            tracing::error!("admin members: list membership types failed: {}", e);
            Vec::new()
        });
    let type_filter_id = query.member_type.as_deref()
        .and_then(|slug| all_types.iter().find(|t| t.slug == slug).map(|t| t.id));
    let type_name_by_id: std::collections::HashMap<uuid::Uuid, String> = all_types
        .iter()
        .map(|t| (t.id, t.name.clone()))
        .collect();
    let type_options: Vec<MembershipTypeOption> = all_types
        .iter()
        .filter(|t| t.is_active)
        .map(|t| MembershipTypeOption {
            id: t.id.to_string(),
            slug: t.slug.clone(),
            name: t.name.clone(),
        })
        .collect();

    // Parse the wire-shape query into the typed MemberQuery. Unknown
    // sort/order values fall back to the defaults rather than failing
    // the request (the dropdown only offers known values; keeping the
    // page renderable on a stale URL is friendlier). Unknown
    // status/type filter values resolve to None — matches the prior
    // "no filter" behavior on a malformed value.
    use crate::repository::{MemberQuery, MemberSortField, SortOrder};
    let typed_query = MemberQuery {
        search: query.q.clone().filter(|s| !s.is_empty()),
        status: query.status.as_deref().and_then(crate::domain::MemberStatus::from_str),
        membership_type_id: type_filter_id,
        sort: match sort_field.as_str() {
            "status" => MemberSortField::Status,
            "type" => MemberSortField::MembershipType,
            "joined" => MemberSortField::Joined,
            "dues" => MemberSortField::DuesPaidUntil,
            _ => MemberSortField::Name,
        },
        order: if sort_order == "desc" { SortOrder::Desc } else { SortOrder::Asc },
        limit: per_page,
        offset,
    };

    let (members, total_members) = member_repo
        .search(typed_query)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("admin members search failed: {}", e);
            (Vec::new(), 0)
        });
    let total_pages = (total_members + per_page - 1) / per_page;

    let paginated_members: Vec<AdminMemberInfo> = members
        .into_iter()
        .map(|m| {
            let initials: String = m.full_name
                .split_whitespace()
                .filter_map(|word| word.chars().next())
                .take(2)
                .collect::<String>()
                .to_uppercase();

            AdminMemberInfo {
                id: m.id,
                email: m.email,
                username: m.username,
                full_name: m.full_name,
                initials: if initials.is_empty() { "?".to_string() } else { initials },
                status: m.status,
                membership_type: type_name_by_id
                    .get(&m.membership_type_id)
                    .cloned()
                    .unwrap_or_else(|| "(unknown)".to_string()),
                joined_at: m.joined_at,
                dues_paid_until: m.dues_paid_until,
            }
        })
        .collect();

    let search_query_val = query.q.unwrap_or_default();
    let status_filter_val = query.status.unwrap_or_default();
    let type_filter_val = query.member_type.unwrap_or_default();

    if is_htmx {
        HtmlTemplate(AdminMembersTableTemplate {
            members: paginated_members,
            total_members,
            current_page: page,
            per_page,
            total_pages,
            search_query: search_query_val,
            status_filter: status_filter_val,
            type_filter: type_filter_val,
            sort_field,
            sort_order,
        }).into_response()
    } else {
        HtmlTemplate(AdminMembersTemplate {
            base,
            members: paginated_members,
            total_members,
            current_page: page,
            per_page,
            type_options,
            total_pages,
            search_query: search_query_val,
            status_filter: status_filter_val,
            type_filter: type_filter_val,
            sort_field,
            sort_order,
        }).into_response()
    }
}

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

pub async fn admin_activate_member(
    State(member_service): State<Arc<MemberService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return partials::member_row_error("Invalid member ID"),
    };

    match member_service.activate(current_user.member.id, id).await {
        Ok(member) => {
            let mt_name = member_service
                .membership_type_name(&member).await;
            partials::member_row_flash(&member, mt_name, "active")
        }
        Err(e) => partials::member_row_error(&format!("Error: {}", e)),
    }
}

pub async fn admin_suspend_member(
    State(member_service): State<Arc<MemberService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return partials::member_row_error("Invalid member ID"),
    };

    match member_service.suspend(current_user.member.id, id).await {
        Ok(member) => {
            let mt_name = member_service
                .membership_type_name(&member).await;
            partials::member_row_flash(&member, mt_name, "suspended")
        }
        Err(e) => partials::member_row_error(&format!("Error: {}", e)),
    }
}

// Member Detail Page

#[derive(Template)]
#[template(path = "admin/member_detail.html")]
pub struct AdminMemberDetailTemplate {
    pub base: BaseContext,
    pub member: AdminMemberDetailInfo,
    pub type_options: Vec<MembershipTypeOption>,
}

pub struct AdminMemberDetailInfo {
    pub id: uuid::Uuid,
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub initials: String,
    pub status: crate::domain::MemberStatus,
    pub membership_type_id: String,
    pub membership_type_name: String,
    pub joined_at: chrono::DateTime<chrono::Utc>,
    pub dues_paid_until: Option<chrono::DateTime<chrono::Utc>>,
    pub dues_expired: bool,
    pub bypass_dues: bool,
    pub email_verified: bool,
    pub notes: String,
    pub billing_mode: String,
    pub stripe_customer_id: Option<String>,
    pub stripe_subscription_id: Option<String>,
    pub discord_id: String,
    pub saved_cards: Vec<AdminSavedCardInfo>,
    pub created_at: String,
    pub updated_at: String,
}

pub struct AdminSavedCardInfo {
    pub display_name: String,
    pub exp_display: String,
    pub is_default: bool,
}

pub async fn admin_member_detail_page(
    State(member_repo): State<Arc<dyn MemberRepository>>,
    State(saved_card_repo): State<Arc<dyn SavedCardRepository>>,
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(member_id): Path<String>,
) -> axum::response::Response {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Redirect::to("/portal/admin/members").into_response(),
    };

    let member = match member_repo.find_by_id(id).await {
        Ok(Some(m)) => m,
        _ => return axum::response::Redirect::to("/portal/admin/members").into_response(),
    };

    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;

    let initials: String = member.full_name
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();

    let now = chrono::Utc::now();
    let dues_expired = member.dues_paid_until
        .map(|d| d < now)
        .unwrap_or(true);

    // Fetch saved cards for this member
    let saved_cards = saved_card_repo
        .find_by_member(member.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|c| AdminSavedCardInfo {
            display_name: c.display_name(),
            exp_display: c.exp_display(),
            is_default: c.is_default,
        })
        .collect();

    let email_verified = member.email_verified();

    let all_types = membership_type_service
        .list(true).await.unwrap_or_default();
    let type_name = all_types.iter()
        .find(|t| t.id == member.membership_type_id)
        .map(|t| t.name.clone())
        .unwrap_or_else(|| "(unknown)".to_string());
    let type_options: Vec<MembershipTypeOption> = all_types
        .iter()
        .filter(|t| t.is_active)
        .map(|t| MembershipTypeOption {
            id: t.id.to_string(),
            slug: t.slug.clone(),
            name: t.name.clone(),
        })
        .collect();

    let member_info = AdminMemberDetailInfo {
        id: member.id,
        email: member.email.clone(),
        username: member.username,
        full_name: member.full_name,
        initials: if initials.is_empty() { "?".to_string() } else { initials },
        status: member.status,
        membership_type_id: member.membership_type_id.to_string(),
        membership_type_name: type_name,
        joined_at: member.joined_at,
        dues_paid_until: member.dues_paid_until,
        dues_expired,
        bypass_dues: member.bypass_dues,
        email_verified,
        notes: member.notes.unwrap_or_default(),
        billing_mode: member.billing_mode.as_str().to_string(),
        stripe_customer_id: member.stripe_customer_id,
        stripe_subscription_id: member.stripe_subscription_id,
        discord_id: member.discord_id.unwrap_or_default(),
        saved_cards,
        created_at: member.created_at.format("%B %d, %Y").to_string(),
        updated_at: member.updated_at.format("%B %d, %Y at %l:%M %p").to_string(),
    };

    let template = AdminMemberDetailTemplate {
        base,
        member: member_info,
        type_options,
    };

    HtmlTemplate(template).into_response()
}

#[derive(Debug, Deserialize)]
pub struct AdminUpdateMemberForm {
    pub full_name: String,
    pub membership_type_id: String,
    pub notes: Option<String>,
    pub bypass_dues: Option<String>,
    #[allow(dead_code)]
    pub csrf_token: String,
}

pub async fn admin_update_member(
    State(member_service): State<Arc<MemberService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
    axum::Form(form): axum::Form<AdminUpdateMemberForm>,
) -> impl IntoResponse {
    use crate::domain::UpdateMemberRequest;

    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid member ID", false),
    };

    let membership_type_id = match uuid::Uuid::parse_str(&form.membership_type_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid membership type.", false),
    };

    let update = UpdateMemberRequest {
        full_name: Some(form.full_name),
        membership_type_id: Some(membership_type_id),
        notes: Some(form.notes.unwrap_or_default()),
        bypass_dues: Some(form.bypass_dues.is_some()),
        ..Default::default()
    };

    match member_service.update(current_user.member.id, id, update).await {
        Ok(_) => partials::admin_alert("success", "Member updated successfully!", false),
        Err(e) => partials::admin_alert("error", &format!("Error: {}", e), false),
    }
}

#[derive(Debug, Deserialize)]
pub struct ExtendDuesForm {
    pub months: i32,
    #[allow(dead_code)]
    pub csrf_token: String,
}

// =====================================================================
// Record-payment page (admin form for entering manual payments)
// =====================================================================

#[derive(askama::Template)]
#[template(path = "admin/record_payment.html")]
pub struct RecordPaymentTemplate {
    pub base: BaseContext,
    pub member_id: String,
    pub member_name: String,
    pub member_email: String,
    pub membership_types: Vec<RecordPaymentMembershipType>,
    pub donation_campaigns: Vec<RecordPaymentCampaign>,
    /// The slug of the member's current membership type, so the form
    /// can pre-select it. Empty if not assigned.
    pub current_membership_slug: String,
    pub flash_error: Option<String>,
}

pub struct RecordPaymentMembershipType {
    pub slug: String,
    pub name: String,
    pub fee_display: String,
    pub billing_period: String,
}

pub struct RecordPaymentCampaign {
    pub id: String,
    pub name: String,
}

pub async fn admin_record_payment_page(
    State(member_repo): State<Arc<dyn MemberRepository>>,
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    State(donation_campaign_repo): State<Arc<dyn DonationCampaignRepository>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(member_id): Path<String>,
) -> axum::response::Response {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Redirect::to("/portal/admin/members").into_response(),
    };

    let member = match member_repo.find_by_id(id).await {
        Ok(Some(m)) => m,
        _ => return axum::response::Redirect::to("/portal/admin/members").into_response(),
    };

    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;

    let membership_types = membership_type_service
        .list(false)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|mt| RecordPaymentMembershipType {
            slug: mt.slug,
            name: mt.name,
            fee_display: format!("{:.2}", mt.fee_cents as f64 / 100.0),
            billing_period: mt.billing_period,
        })
        .collect();

    let donation_campaigns = donation_campaign_repo
        .list_active()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|c| RecordPaymentCampaign {
            id: c.id.to_string(),
            name: c.name,
        })
        .collect();

    // Resolve current membership type slug for default selection.
    let current_membership_slug = membership_type_service
        .get(member.membership_type_id).await
        .ok().flatten()
        .map(|mt| mt.slug)
        .unwrap_or_default();

    HtmlTemplate(RecordPaymentTemplate {
        base,
        member_id: member.id.to_string(),
        member_name: member.full_name.clone(),
        member_email: member.email.clone(),
        membership_types,
        donation_campaigns,
        current_membership_slug,
        flash_error: None,
    }).into_response()
}

#[derive(Debug, Deserialize)]
pub struct RecordPaymentForm {
    #[allow(dead_code)]
    pub csrf_token: String,
    /// "membership" | "donation" | "other"
    pub payment_type: String,
    pub amount: String,
    pub description: String,
    /// Set when payment_type=membership
    #[serde(default)]
    pub membership_type_slug: String,
    /// Set when payment_type=donation
    #[serde(default)]
    pub donation_campaign_id: String,
}

pub async fn admin_record_payment_submit(
    State(member_repo): State<Arc<dyn MemberRepository>>,
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    State(donation_campaign_repo): State<Arc<dyn DonationCampaignRepository>>,
    State(payment_service): State<Arc<PaymentService>>,
    State(billing_service): State<Arc<crate::service::billing_service::BillingService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(member_id): Path<String>,
    axum::Form(form): axum::Form<RecordPaymentForm>,
) -> axum::response::Response {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Redirect::to("/portal/admin/members").into_response(),
    };

    // Parse dollars → cents. Accept "100" or "100.00" or "100.5".
    let amount_cents = match parse_dollars_to_cents(&form.amount) {
        Some(c) if c > 0 || form.payment_type == "membership" => c,
        _ => return rerender_with_error(
            &member_repo, &membership_type_service, &donation_campaign_repo, &csrf_service,
            current_user, session_info, member_id,
            "Amount must be a positive dollar amount.",
        ).await,
    };
    if amount_cents > crate::domain::MAX_PAYMENT_CENTS {
        return rerender_with_error(
            &member_repo, &membership_type_service, &donation_campaign_repo, &csrf_service,
            current_user, session_info, member_id,
            &format!(
                "Amount exceeds the ${} cap on a single payment — \
                 split it into multiple records if intentional.",
                crate::domain::MAX_PAYMENT_CENTS / 100,
            ),
        ).await;
    }

    use crate::domain::{PaymentKind, PaymentMethod};
    use crate::service::payment_service::RecordManualPaymentInput;

    // Wire-format parsing: the form's `payment_type` string + the
    // form's separate campaign-id string become a typed `PaymentKind`.
    // Empty/invalid campaign id is rejected here (form-shape validation);
    // existence is checked by `PaymentService::record_manual`.
    let kind = match form.payment_type.as_str() {
        "membership" => PaymentKind::Membership,
        "donation" => {
            let cid_str = form.donation_campaign_id.trim();
            if cid_str.is_empty() {
                return rerender_with_error(
                    &member_repo, &membership_type_service, &donation_campaign_repo, &csrf_service,
                    current_user, session_info, member_id,
                    "Donation requires a campaign selection.",
                ).await;
            }
            let cid = match uuid::Uuid::parse_str(cid_str) {
                Ok(cid) => cid,
                Err(_) => return rerender_with_error(
                    &member_repo, &membership_type_service, &donation_campaign_repo, &csrf_service,
                    current_user, session_info, member_id,
                    "Invalid campaign id.",
                ).await,
            };
            PaymentKind::Donation { campaign_id: Some(cid) }
        }
        "other" => PaymentKind::Other,
        _ => return rerender_with_error(
            &member_repo, &membership_type_service, &donation_campaign_repo, &csrf_service,
            current_user, session_info, member_id,
            "Invalid payment type.",
        ).await,
    };

    let description = if form.description.trim().is_empty() {
        match kind {
            PaymentKind::Membership => "Manual membership payment".to_string(),
            PaymentKind::Donation { .. } => "Donation".to_string(),
            PaymentKind::Other => "Manual payment".to_string(),
        }
    } else {
        form.description.clone()
    };

    let slug_for_dues = if matches!(kind, PaymentKind::Membership) && !form.membership_type_slug.is_empty() {
        Some(form.membership_type_slug.clone())
    } else {
        None
    };
    if let Err(e) = payment_service.record_manual(
        RecordManualPaymentInput {
            member_id: id,
            amount_cents,
            kind,
            description,
            payment_method: PaymentMethod::Manual,
            membership_type_slug: slug_for_dues,
            actor_id: current_user.member.id,
        },
        &billing_service,
    ).await {
        return rerender_with_error(
            &member_repo, &membership_type_service, &donation_campaign_repo, &csrf_service,
            current_user, session_info, member_id,
            &format!("Failed to record payment: {}", e),
        ).await;
    }

    // PaymentService emits the audit event itself, so the handler is
    // done once record_manual returns Ok.
    axum::response::Redirect::to(&format!("/portal/admin/members/{}", id)).into_response()
}

/// "100", "100.00", "100.5" → 10000, 10000, 10050. Returns None on
/// junk input or negative values. Refuses more than 2 decimal places
/// to prevent silent rounding.
fn parse_dollars_to_cents(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (whole, frac) = match s.split_once('.') {
        Some((w, f)) => (w, f),
        None => (s, ""),
    };
    if frac.len() > 2 || !frac.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let whole: i64 = whole.parse().ok()?;
    if whole < 0 {
        return None;
    }
    let frac_padded = format!("{:0<2}", frac);
    let frac: i64 = if frac_padded.is_empty() { 0 } else { frac_padded.parse().ok()? };
    whole.checked_mul(100)?.checked_add(frac)
}

async fn rerender_with_error(
    member_repo: &Arc<dyn MemberRepository>,
    membership_type_service: &Arc<MembershipTypeService>,
    donation_campaign_repo: &Arc<dyn DonationCampaignRepository>,
    csrf_service: &CsrfService,
    current_user: CurrentUser,
    session_info: SessionInfo,
    member_id: String,
    error: &str,
) -> axum::response::Response {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Redirect::to("/portal/admin/members").into_response(),
    };
    let member = match member_repo.find_by_id(id).await {
        Ok(Some(m)) => m,
        _ => return axum::response::Redirect::to("/portal/admin/members").into_response(),
    };

    let base = BaseContext::for_member(csrf_service, &current_user, &session_info).await;

    let membership_types = membership_type_service
        .list(false).await.unwrap_or_default()
        .into_iter()
        .map(|mt| RecordPaymentMembershipType {
            slug: mt.slug, name: mt.name,
            fee_display: format!("{:.2}", mt.fee_cents as f64 / 100.0),
            billing_period: mt.billing_period,
        })
        .collect();

    let donation_campaigns = donation_campaign_repo
        .list_active().await.unwrap_or_default()
        .into_iter()
        .map(|c| RecordPaymentCampaign { id: c.id.to_string(), name: c.name })
        .collect();

    let current_membership_slug = membership_type_service
        .get(member.membership_type_id).await.ok().flatten()
        .map(|mt| mt.slug).unwrap_or_default();

    HtmlTemplate(RecordPaymentTemplate {
        base,
        member_id: member.id.to_string(),
        member_name: member.full_name.clone(),
        member_email: member.email.clone(),
        membership_types,
        donation_campaigns,
        current_membership_slug,
        flash_error: Some(error.to_string()),
    }).into_response()
}

pub async fn admin_extend_dues(
    State(member_service): State<Arc<MemberService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
    axum::Form(form): axum::Form<ExtendDuesForm>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid member ID", false),
    };

    match member_service
        .extend_dues(current_user.member.id, id, form.months)
        .await
    {
        Ok(member) => {
            let new_dues = member.dues_paid_until
                .map(|d| d.format("%B %d, %Y").to_string())
                .unwrap_or_else(|| "—".to_string());
            partials::admin_alert(
                "success",
                &format!("Dues extended! New expiration: {}", new_dues),
                true,
            )
        }
        Err(e) => partials::admin_alert("error", &format!("Error: {}", e), false),
    }
}

#[derive(Debug, Deserialize)]
pub struct SetDuesForm {
    pub dues_until: String,
    #[allow(dead_code)]
    pub csrf_token: String,
}

pub async fn admin_set_dues(
    State(member_service): State<Arc<MemberService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
    axum::Form(form): axum::Form<SetDuesForm>,
) -> impl IntoResponse {
    use chrono::NaiveDate;

    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid member ID", false),
    };

    let naive_date = match NaiveDate::parse_from_str(&form.dues_until, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => return partials::admin_alert("error", "Invalid date format", false),
    };

    match member_service
        .set_dues(current_user.member.id, id, naive_date)
        .await
    {
        Ok(member) => {
            let dues = member.dues_paid_until
                .map(|d| d.format("%B %d, %Y").to_string())
                .unwrap_or_else(|| "—".to_string());
            partials::admin_alert(
                "success",
                &format!("Dues date set to: {}", dues),
                true,
            )
        }
        Err(e) => partials::admin_alert("error", &format!("Error: {}", e), false),
    }
}

pub async fn admin_expire_now(
    State(member_service): State<Arc<MemberService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid member ID", false),
    };

    match member_service
        .expire_now(current_user.member.id, id)
        .await
    {
        Ok(_) => partials::admin_alert("warning", "Member dues have been expired.", true),
        Err(e) => partials::admin_alert("error", &format!("Error: {}", e), false),
    }
}

pub async fn admin_member_payments(
    State(payment_repo): State<Arc<dyn PaymentRepository>>,
    Extension(_current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid member ID", false),
    };

    let payments = payment_repo
        .find_by_member(id)
        .await
        .unwrap_or_default();

    let rows = payments.iter().map(partials::admin_payment_row_from).collect();
    partials::admin_payment_list(rows)
}

// New Member Page

#[derive(Template)]
#[template(path = "admin/member_new.html")]
pub struct AdminNewMemberTemplate {
    pub base: BaseContext,
    pub type_options: Vec<MembershipTypeOption>,
}

pub async fn admin_new_member_page(
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> axum::response::Response {
    let type_options: Vec<MembershipTypeOption> = membership_type_service
        .list(false).await
        .unwrap_or_default()
        .into_iter()
        .map(|t| MembershipTypeOption {
            id: t.id.to_string(),
            slug: t.slug,
            name: t.name,
        })
        .collect();

    let template = AdminNewMemberTemplate {
        base: BaseContext::for_member(&csrf_service, &current_user, &session_info).await,
        type_options,
    };

    HtmlTemplate(template).into_response()
}

#[derive(Debug, Deserialize)]
pub struct AdminCreateMemberForm {
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub password: String,
    pub membership_type_id: String,
    pub status: String,
    pub notes: Option<String>,
    #[allow(dead_code)]
    pub csrf_token: String,
}

pub async fn admin_create_member(
    State(member_service): State<Arc<MemberService>>,
    Extension(current_user): Extension<CurrentUser>,
    axum::Form(form): axum::Form<AdminCreateMemberForm>,
) -> axum::response::Response {
    use crate::domain::{CreateMemberRequest, MemberStatus, UpdateMemberRequest};

    fn render_error(message: &str) -> axum::response::Response {
        axum::response::Html(format!(
            r#"<!DOCTYPE html>
            <html>
            <head>
                <title>Error - Coterie</title>
                <link rel="stylesheet" href="/static/style.css">
            </head>
            <body class="bg-gray-100 min-h-screen flex items-center justify-center">
                <div class="bg-white p-8 rounded-lg shadow-md max-w-md">
                    <h1 class="text-xl font-bold text-red-600 mb-4">Error Creating Member</h1>
                    <p class="text-gray-700 mb-4">{}</p>
                    <a href="/portal/admin/members/new" class="text-blue-600 hover:underline">Go back and try again</a>
                </div>
            </body>
            </html>"#,
            crate::web::escape_html(message),
        )).into_response()
    }

    let membership_type_id = match uuid::Uuid::parse_str(&form.membership_type_id) {
        Ok(id) => id,
        Err(_) => return render_error("Invalid membership type."),
    };

    let create_request = CreateMemberRequest {
        email: form.email.clone(),
        username: form.username.clone(),
        full_name: form.full_name.clone(),
        password: form.password,
        membership_type_id: Some(membership_type_id),
    };

    match member_service.create(current_user.member.id, create_request).await {
        Ok(member) => {
            // Pending is the default already set by `create`, so an
            // empty / "Pending" form value is a no-op — only override
            // when the admin picked a different status. Unknown values
            // (typo, forged form) skip the override rather than silently
            // landing on a default.
            let status = match form.status.as_str() {
                "" | "Pending" => None,
                s => MemberStatus::from_str(s),
            };

            if status.is_some() || form.notes.is_some() {
                let update = UpdateMemberRequest {
                    status,
                    notes: form.notes,
                    ..Default::default()
                };
                if let Err(e) = member_service
                    .update(current_user.member.id, member.id, update).await
                {
                    // Member was created but the status/notes follow-up
                    // failed. The admin will see the detail page with
                    // the original (Pending, no notes) state — not
                    // catastrophic but worth logging so they know why
                    // the form values didn't take.
                    tracing::error!(
                        "Created member {} but follow-up status/notes update failed: {}",
                        member.id, e
                    );
                }
            }

            axum::response::Redirect::to(&format!("/portal/admin/members/{}", member.id)).into_response()
        }
        Err(e) => render_error(&e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateDiscordIdForm {
    /// Empty string means "clear the discord_id".
    pub discord_id: String,
    #[allow(dead_code)]
    pub csrf_token: String,
}

/// Admin sets or clears a member's Discord snowflake ID. Validates the
/// format up-front; on success, fires a MemberUpdated event so Discord
/// integration can re-sync roles to the new ID (and strip them from
/// the old, if any).
pub async fn admin_update_discord_id(
    State(member_service): State<Arc<MemberService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
    axum::Form(form): axum::Form<UpdateDiscordIdForm>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return discord_id_result(false, "Invalid member ID"),
    };

    let new_value = if form.discord_id.trim().is_empty() {
        None
    } else {
        Some(form.discord_id.clone())
    };

    match member_service
        .update_discord_id(current_user.member.id, id, new_value)
        .await
    {
        Ok(member) => {
            let msg = match &member.discord_id {
                Some(v) => format!("Discord ID set to {} (role sync triggered).", v),
                None => "Discord ID cleared.".to_string(),
            };
            discord_id_result(true, &msg)
        }
        Err(e) => discord_id_result(false, &format!("Failed to save: {}", e)),
    }
}

fn discord_id_result(ok: bool, detail: &str) -> axum::response::Response {
    let escaped = crate::web::escape_html(detail);
    let (bg, fg) = if ok { ("bg-green-50", "text-green-900") } else { ("bg-red-50", "text-red-900") };
    axum::response::Html(format!(
        r#"<div id="discord-id-result" class="mt-2 p-2 {bg} {fg} rounded text-sm">{detail}</div>"#,
        bg = bg, fg = fg, detail = escaped,
    )).into_response()
}

/// Admin-triggered: regenerate a verification token for an unverified
/// member and email them the fresh link. Invalidates any previously
/// outstanding tokens so the old email (if the member still has it)
/// can't be used.
pub async fn admin_resend_verification(
    State(member_repo): State<Arc<dyn MemberRepository>>,
    State(member_service): State<Arc<MemberService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> axum::response::Response {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return resend_result(false, "Invalid member ID").into_response(),
    };

    // The service refetches the member to render the success message;
    // for the resend flow we need the member's email for the success
    // string, so re-fetch here too. The service-level audit fires on
    // success of the email send.
    let email = match member_repo.find_by_id(id).await {
        Ok(Some(m)) => m.email,
        Ok(None) => return resend_result(false, "Member not found").into_response(),
        Err(e) => return resend_result(false, &format!("DB error: {}", e)).into_response(),
    };

    match member_service
        .resend_verification(current_user.member.id, id)
        .await
    {
        Ok(()) => resend_result(true, &format!("Verification email resent to {}.", email)).into_response(),
        Err(e) => resend_result(false, &format!("Send failed: {}", e)).into_response(),
    }
}

fn resend_result(ok: bool, detail: &str) -> axum::response::Html<String> {
    let escaped = crate::web::escape_html(detail);
    let (bg, fg) = if ok {
        ("bg-green-50", "text-green-900")
    } else {
        ("bg-red-50", "text-red-900")
    };
    axum::response::Html(format!(
        r#"<div id="verify-resend-result" class="mt-2 p-2 {bg} {fg} rounded text-sm">{detail}</div>"#,
        bg = bg, fg = fg, detail = escaped,
    ))
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
    use crate::service::member_service::ImportRow;

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

        let status = get_opt(status_idx).and_then(|s| crate::domain::MemberStatus::from_str(s.trim()));

        rows.push(ImportRow {
            email: get(email_idx),
            username: get(username_idx),
            full_name: get(full_name_idx),
            membership_type_slug: get(mtype_idx),
            status,
            notes: get_opt(notes_idx),
            discord_id: get_opt(discord_idx),
        });
    }

    Ok(rows)
}

fn import_error_fragment(message: &str) -> impl IntoResponse {
    HtmlTemplate(AdminMemberImportErrorTemplate {
        message: message.to_string(),
    })
}
