use askama::Template;
use axum::{
    extract::{State, Query, Path},
    response::IntoResponse,
    Extension,
};
use serde::Deserialize;

use crate::{
    api::{
        middleware::auth::{CurrentUser, SessionInfo},
        state::AppState,
    },
    web::{portal::admin::partials, templates::{HtmlTemplate, UserInfo}},
};

#[derive(Template)]
#[template(path = "admin/members.html")]
pub struct AdminMembersTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
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
pub struct AdminMemberInfo {
    pub id: String,
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub initials: String,
    pub status: String,
    pub membership_type: String,
    pub joined_at: String,
    pub dues_paid_until: Option<String>,
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
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Query(query): Query<AdminMembersQuery>,
) -> impl IntoResponse {
    let is_htmx = headers.get("HX-Request").is_some();

    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_else(|_| "error".to_string());

    let page = query.page.unwrap_or(1).max(1);
    let per_page: i64 = 20;
    let offset = (page - 1) * per_page;

    let sort_field = query.sort.clone().unwrap_or_else(|| "name".to_string());
    let sort_order = query.order.clone().unwrap_or_else(|| "asc".to_string());

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
        membership_type: query.member_type.as_deref().and_then(crate::domain::MembershipType::from_str),
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

    let (members, total_members) = state.service_context.member_repo
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
                id: m.id.to_string(),
                email: m.email,
                username: m.username,
                full_name: m.full_name,
                initials: if initials.is_empty() { "?".to_string() } else { initials },
                status: m.status.as_str().to_string(),
                membership_type: m.membership_type.as_str().to_string(),
                joined_at: m.joined_at.format("%b %d, %Y").to_string(),
                dues_paid_until: m.dues_paid_until.map(|d| d.format("%b %d, %Y").to_string()),
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
            current_user: Some(user_info),
            is_admin: true,
            csrf_token,
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
    }
}

pub async fn admin_activate_member(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> impl IntoResponse {
    use crate::domain::{UpdateMemberRequest, MemberStatus};

    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return partials::member_row_error("Invalid member ID"),
    };

    let update = UpdateMemberRequest {
        status: Some(MemberStatus::Active),
        ..Default::default()
    };

    match state.service_context.member_repo.update(id, update).await {
        Ok(member) => {
            // Force re-auth so the member picks up their new status on next request.
            if let Err(e) = state.service_context.auth_service
                .invalidate_all_sessions(member.id)
                .await
            {
                tracing::error!(
                    "Activated member {} but failed to invalidate sessions: {}",
                    member.id, e
                );
            }

            state.service_context.audit_service.log(
                Some(current_user.member.id),
                "activate_member",
                "member",
                &id.to_string(),
                None,
                Some(&member.email),
                None,
            ).await;

            // Notify integrations (Discord role sync, future Unifi
            // access provisioning, etc.). The dispatcher is fire-
            // and-forget at the integration level — individual
            // failures are logged inside each impl.
            state.service_context.integration_manager
                .handle_event(crate::integrations::IntegrationEvent::MemberActivated(member.clone()))
                .await;

            // Send welcome email. Soft-fail: activation already succeeded,
            // and an admin can always resend manually if it didn't arrive.
            if let Err(e) = send_welcome_email(&state, &member).await {
                tracing::error!(
                    "Member {} activated but welcome email failed: {}",
                    member.id, e
                );
            }

            partials::member_row_flash(&member, "active")
        }
        Err(e) => partials::member_row_error(&format!("Error: {}", e)),
    }
}

pub async fn admin_suspend_member(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> impl IntoResponse {
    use crate::domain::{UpdateMemberRequest, MemberStatus};

    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return partials::member_row_error("Invalid member ID"),
    };

    // Snapshot the pre-update member so we can dispatch the proper
    // before/after pair to integrations (Discord uses this to decide
    // which roles to remove vs add).
    let old_member = state.service_context.member_repo.find_by_id(id).await.ok().flatten();

    let update = UpdateMemberRequest {
        status: Some(MemberStatus::Suspended),
        ..Default::default()
    };

    match state.service_context.member_repo.update(id, update).await {
        Ok(member) => {
            // Kick the suspended member out of any active sessions immediately.
            // If invalidation fails, middleware still rejects Suspended status
            // on the next request — but log so operators see the failure.
            if let Err(e) = state.service_context.auth_service
                .invalidate_all_sessions(member.id)
                .await
            {
                tracing::error!(
                    "Suspended member {} but failed to invalidate sessions: {}",
                    member.id, e
                );
            }

            // Fire integration event with old/new for status diff.
            if let Some(old) = old_member {
                state.service_context.integration_manager
                    .handle_event(crate::integrations::IntegrationEvent::MemberUpdated {
                        old, new: member.clone()
                    })
                    .await;
            }

            state.service_context.audit_service.log(
                Some(current_user.member.id),
                "suspend_member",
                "member",
                &id.to_string(),
                None,
                Some(&member.email),
                None,
            ).await;

            partials::member_row_flash(&member, "suspended")
        }
        Err(e) => partials::member_row_error(&format!("Error: {}", e)),
    }
}

// Member Detail Page

#[derive(Template)]
#[template(path = "admin/member_detail.html")]
pub struct AdminMemberDetailTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
    pub member: AdminMemberDetailInfo,
}

pub struct AdminMemberDetailInfo {
    pub id: String,
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub initials: String,
    pub status: String,
    pub membership_type: String,
    pub joined_at: String,
    pub dues_paid_until: Option<String>,
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
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(member_id): Path<String>,
) -> axum::response::Response {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Redirect::to("/portal/admin/members").into_response(),
    };

    let member = match state.service_context.member_repo.find_by_id(id).await {
        Ok(Some(m)) => m,
        _ => return axum::response::Redirect::to("/portal/admin/members").into_response(),
    };

    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_else(|_| "error".to_string());

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
    let saved_cards = state.service_context.saved_card_repo
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

    let member_info = AdminMemberDetailInfo {
        id: member.id.to_string(),
        email: member.email.clone(),
        username: member.username,
        full_name: member.full_name,
        initials: if initials.is_empty() { "?".to_string() } else { initials },
        status: member.status.as_str().to_string(),
        membership_type: member.membership_type.as_str().to_string(),
        joined_at: member.joined_at.format("%B %d, %Y").to_string(),
        dues_paid_until: member.dues_paid_until.map(|d| d.format("%B %d, %Y").to_string()),
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
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
        member: member_info,
    };

    HtmlTemplate(template).into_response()
}

#[derive(Debug, Deserialize)]
pub struct AdminUpdateMemberForm {
    pub full_name: String,
    pub membership_type: String,
    pub notes: Option<String>,
    pub bypass_dues: Option<String>,
    #[allow(dead_code)]
    pub csrf_token: String,
}

pub async fn admin_update_member(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
    axum::Form(form): axum::Form<AdminUpdateMemberForm>,
) -> impl IntoResponse {
    use crate::domain::{UpdateMemberRequest, MembershipType};

    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid member ID", false),
    };

    let membership_type = match MembershipType::from_str(&form.membership_type) {
        Some(t) => t,
        None => return partials::admin_alert("error", "Invalid membership type.", false),
    };

    // Snapshot the old member so we can emit a complete MemberUpdated event.
    // Currently this handler doesn't change status — only profile fields —
    // so integrations like Discord won't act on it. We dispatch anyway so
    // future fields (e.g., discord_id editable from the same form) are
    // covered without further wiring.
    let old_member = state.service_context.member_repo.find_by_id(id).await.ok().flatten();

    let update = UpdateMemberRequest {
        full_name: Some(form.full_name),
        membership_type: Some(membership_type),
        notes: Some(form.notes.unwrap_or_default()),
        bypass_dues: Some(form.bypass_dues.is_some()),
        ..Default::default()
    };

    match state.service_context.member_repo.update(id, update).await {
        Ok(new_member) => {
            state.service_context.audit_service.log(
                Some(current_user.member.id),
                "update_member",
                "member",
                &id.to_string(),
                None,
                None,
                None,
            ).await;
            if let Some(old) = old_member {
                state.service_context.integration_manager
                    .handle_event(crate::integrations::IntegrationEvent::MemberUpdated {
                        old, new: new_member,
                    })
                    .await;
            }
            partials::admin_alert("success", "Member updated successfully!", false)
        }
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
// Refund a payment (admin action; called from member detail page)
// =====================================================================

/// Refund a previously-recorded payment. Behavior depends on payment_method:
///
///   - `Stripe`  → call Stripe's Refund API (full refund), then mark
///                 the local Payment row as Refunded
///   - `Manual`  → just mark the local row as Refunded; admin presumably
///                 returned cash / wrote a check etc. out-of-band
///   - `Waived`  → reject (nothing to refund — the row was $0 to begin with)
///
/// Already-Refunded payments return early without calling Stripe again
/// (idempotent against double-clicks).
///
/// Refunds DO NOT roll back `dues_paid_until`. Refunding is usually a
/// customer-service gesture rather than an access revocation; an admin
/// can manually adjust dues afterward via the existing extend/set-dues
/// UI if they actually want to kick someone out.
///
/// Returns to the member detail page on success or with an error flash
/// on failure (HTMX-style fragment for inline rendering).
pub async fn admin_refund_payment(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    headers: axum::http::HeaderMap,
    Path(payment_id): Path<String>,
) -> impl IntoResponse {

    let ip = crate::api::state::client_ip(&headers, state.settings.server.trust_forwarded_for());
    if !state.money_limiter.check_and_record(ip) {
        return refund_result_html(
            false,
            "Too many refund attempts — try again in a minute.",
        );
    }

    let pid = match uuid::Uuid::parse_str(&payment_id) {
        Ok(id) => id,
        Err(_) => return refund_result_html(false, "Invalid payment ID"),
    };

    let payment = match state.service_context.payment_repo.find_by_id(pid).await {
        Ok(Some(p)) => p,
        _ => return refund_result_html(false, "Payment not found"),
    };

    use crate::domain::{PaymentMethod, PaymentStatus};

    if payment.status == PaymentStatus::Refunded {
        return refund_result_html(false, "Payment is already refunded");
    }
    if payment.status != PaymentStatus::Completed {
        return refund_result_html(
            false,
            "Only completed payments can be refunded",
        );
    }
    if payment.payment_method == PaymentMethod::Waived {
        return refund_result_html(
            false,
            "Waived payments are $0 — nothing to refund. Use suspend or expire instead.",
        );
    }

    // Atomic claim BEFORE calling Stripe. Two simultaneous admin
    // clicks both reach this point, but only one wins the
    // Completed→Refunded flip; the other bails. Without this, both
    // calls would invoke the Stripe API (idempotency-keyed so Stripe
    // dedupes, but the audit log would still get two entries with
    // different actors).
    let claimed = match state.service_context.payment_repo
        .claim_payment_for_refund(payment.id).await
    {
        Ok(c) => c,
        Err(e) => return refund_result_html(false, &format!("DB update failed: {}", e)),
    };
    if !claimed {
        return refund_result_html(
            false,
            "Payment was already refunded (or its status changed) by another action.",
        );
    }

    let stripe_refund_id: Option<String> = match payment.payment_method {
        PaymentMethod::Waived => unreachable!("Waived already short-circuited above"),
        PaymentMethod::Stripe => {
            let stripe_ref = match payment.external_id.as_ref() {
                Some(r) if !r.as_str().is_empty() => r,
                _ => {
                    let _ = state.service_context.payment_repo.unclaim_refund(payment.id).await;
                    return refund_result_html(
                        false,
                        "Stripe payment has no Stripe ID on record — can't refund through the API. Mark Refunded manually if needed.",
                    );
                }
            };
            let stripe_client = match state.stripe_client.as_ref() {
                Some(c) => c,
                None => {
                    let _ = state.service_context.payment_repo.unclaim_refund(payment.id).await;
                    return refund_result_html(
                        false,
                        "Stripe isn't configured. Can't issue an API refund.",
                    );
                }
            };
            match stripe_client.refund_payment(stripe_ref, &payment.id.to_string()).await {
                Ok(refund_id) => Some(refund_id),
                Err(e) => {
                    // Stripe rejected — roll the local row back so a
                    // future retry can re-claim and re-issue.
                    let _ = state.service_context.payment_repo.unclaim_refund(payment.id).await;
                    return refund_result_html(
                        false,
                        &format!("Stripe refund failed: {}", e),
                    );
                }
            }
        }
        PaymentMethod::Manual => None, // No external system to update.
    };

    let detail = match (&payment.payment_method, &stripe_refund_id) {
        (PaymentMethod::Stripe, Some(rid)) => format!(
            "Refunded ${:.2} via Stripe (refund {})",
            payment.amount_cents as f64 / 100.0, rid,
        ),
        (PaymentMethod::Manual, _) => format!(
            "Marked ${:.2} manual payment as Refunded (no API call — refund the cash/check yourself)",
            payment.amount_cents as f64 / 100.0,
        ),
        _ => format!("Refunded ${:.2}", payment.amount_cents as f64 / 100.0),
    };

    state.service_context.audit_service.log(
        Some(current_user.member.id),
        "refund_payment",
        "payment",
        &payment_id,
        None,
        Some(&detail),
        None,
    ).await;

    // Visibility: a refund is unusual enough to alert on. Goes to the
    // Discord admin-alerts channel + (per D4.2) the org contact email.
    state.service_context.integration_manager
        .handle_event(crate::integrations::IntegrationEvent::AdminAlert {
            subject: format!("Payment refunded — ${:.2}", payment.amount_cents as f64 / 100.0),
            body: format!(
                "Refunded by: {} <{}>\nPayer: {:?}\nMethod: {:?}\nDetail: {}",
                current_user.member.full_name,
                current_user.member.email,
                payment.payer,
                payment.payment_method,
                detail,
            ),
        })
        .await;

    refund_result_html(true, &detail)
}

fn refund_result_html(ok: bool, detail: &str) -> axum::response::Html<String> {
    let escaped = crate::web::escape_html(detail);
    let (bg, fg) = if ok {
        ("bg-green-50", "text-green-900")
    } else {
        ("bg-red-50", "text-red-900")
    };
    // On success we trigger a soft reload so the payments list re-renders
    // with the new Refunded badge. On failure we just show the message.
    let suffix = if ok {
        r#"<script>setTimeout(() => htmx.trigger('#payments-list', 'refresh'), 800);</script>"#
    } else {
        ""
    };
    axum::response::Html(format!(
        r#"<div class="mt-2 p-3 {bg} {fg} rounded-md text-sm">{escaped}</div>{suffix}"#,
        bg = bg, fg = fg, escaped = escaped, suffix = suffix,
    ))
}

// =====================================================================
// Record-payment page (admin form for entering manual payments)
// =====================================================================

#[derive(askama::Template)]
#[template(path = "admin/record_payment.html")]
pub struct RecordPaymentTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
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
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(member_id): Path<String>,
) -> axum::response::Response {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Redirect::to("/portal/admin/members").into_response(),
    };

    let member = match state.service_context.member_repo.find_by_id(id).await {
        Ok(Some(m)) => m,
        _ => return axum::response::Redirect::to("/portal/admin/members").into_response(),
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_default();

    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let membership_types = state.service_context.membership_type_service
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

    let donation_campaigns = state.service_context.donation_campaign_repo
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
    let current_membership_slug = match member.membership_type_id {
        Some(mt_id) => state.service_context.membership_type_service
            .get(mt_id).await
            .ok().flatten()
            .map(|mt| mt.slug)
            .unwrap_or_default(),
        None => String::new(),
    };

    HtmlTemplate(RecordPaymentTemplate {
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
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
    State(state): State<AppState>,
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
            state, current_user, session_info, member_id,
            "Amount must be a positive dollar amount.",
        ).await,
    };
    if amount_cents > crate::domain::MAX_PAYMENT_CENTS {
        return rerender_with_error(
            state, current_user, session_info, member_id,
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
                    state, current_user, session_info, member_id,
                    "Donation requires a campaign selection.",
                ).await;
            }
            let cid = match uuid::Uuid::parse_str(cid_str) {
                Ok(cid) => cid,
                Err(_) => return rerender_with_error(
                    state, current_user, session_info, member_id,
                    "Invalid campaign id.",
                ).await,
            };
            PaymentKind::Donation { campaign_id: Some(cid) }
        }
        "other" => PaymentKind::Other,
        _ => return rerender_with_error(
            state, current_user, session_info, member_id,
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

    let billing_service = state.service_context.billing_service(
        state.stripe_client.clone(),
        state.settings.server.base_url.clone(),
    );
    let slug_for_dues = if matches!(kind, PaymentKind::Membership) && !form.membership_type_slug.is_empty() {
        Some(form.membership_type_slug.clone())
    } else {
        None
    };
    if let Err(e) = state.service_context.payment_service.record_manual(
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
            state, current_user, session_info, member_id,
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
    state: AppState,
    current_user: CurrentUser,
    session_info: SessionInfo,
    member_id: String,
    error: &str,
) -> axum::response::Response {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Redirect::to("/portal/admin/members").into_response(),
    };
    let member = match state.service_context.member_repo.find_by_id(id).await {
        Ok(Some(m)) => m,
        _ => return axum::response::Redirect::to("/portal/admin/members").into_response(),
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_default();

    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let membership_types = state.service_context.membership_type_service
        .list(false).await.unwrap_or_default()
        .into_iter()
        .map(|mt| RecordPaymentMembershipType {
            slug: mt.slug, name: mt.name,
            fee_display: format!("{:.2}", mt.fee_cents as f64 / 100.0),
            billing_period: mt.billing_period,
        })
        .collect();

    let donation_campaigns = state.service_context.donation_campaign_repo
        .list_active().await.unwrap_or_default()
        .into_iter()
        .map(|c| RecordPaymentCampaign { id: c.id.to_string(), name: c.name })
        .collect();

    let current_membership_slug = match member.membership_type_id {
        Some(mt_id) => state.service_context.membership_type_service
            .get(mt_id).await.ok().flatten()
            .map(|mt| mt.slug).unwrap_or_default(),
        None => String::new(),
    };

    HtmlTemplate(RecordPaymentTemplate {
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
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
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
    axum::Form(form): axum::Form<ExtendDuesForm>,
) -> impl IntoResponse {
    use chrono::Months;

    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid member ID", false),
    };

    let old_member = match state.service_context.member_repo.find_by_id(id).await {
        Ok(Some(m)) => m,
        _ => return partials::admin_alert("error", "Member not found", false),
    };

    // 1..=120 months (10 years). Negative `i32 as u32` would
    // wraparound to ~4.29B and silently no-op via unwrap_or(base_date),
    // masking either a fat-finger or a deliberate audit-log dilution.
    if !(1..=120).contains(&form.months) {
        return partials::admin_alert("error", "Months must be between 1 and 120.", false);
    }

    let now = chrono::Utc::now();
    let base_date = old_member.dues_paid_until
        .filter(|d| *d > now)
        .unwrap_or(now);

    let new_dues_date = base_date
        .checked_add_months(Months::new(form.months as u32))
        .unwrap_or(base_date);

    if let Err(e) = state.service_context.member_repo
        .set_dues_paid_until_with_revival(id, new_dues_date)
        .await
    {
        return partials::admin_alert("error", &format!("Error: {}", e), false);
    }

    state.service_context.audit_service.log(
        Some(current_user.member.id),
        "extend_dues",
        "member",
        &member_id,
        None,
        Some(&format!("+{} months → {}", form.months, new_dues_date.format("%Y-%m-%d"))),
        None,
    ).await;
    dispatch_member_updated(&state, id, old_member).await;
    partials::admin_alert(
        "success",
        &format!("Dues extended! New expiration: {}", new_dues_date.format("%B %d, %Y")),
        true,
    )
}

#[derive(Debug, Deserialize)]
pub struct SetDuesForm {
    pub dues_until: String,
    #[allow(dead_code)]
    pub csrf_token: String,
}

pub async fn admin_set_dues(
    State(state): State<AppState>,
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

    let dues_date = naive_date
        .and_hms_opt(23, 59, 59)
        .unwrap()
        .and_utc();

    let old_member = state.service_context.member_repo.find_by_id(id).await.ok().flatten();

    if let Err(e) = state.service_context.member_repo
        .set_dues_paid_until_with_revival(id, dues_date)
        .await
    {
        return partials::admin_alert("error", &format!("Error: {}", e), false);
    }

    state.service_context.audit_service.log(
        Some(current_user.member.id),
        "set_dues",
        "member",
        &id.to_string(),
        None,
        Some(&dues_date.format("%Y-%m-%d").to_string()),
        None,
    ).await;
    if let Some(old) = old_member {
        dispatch_member_updated(&state, id, old).await;
    }
    partials::admin_alert(
        "success",
        &format!("Dues date set to: {}", dues_date.format("%B %d, %Y")),
        true,
    )
}

pub async fn admin_expire_now(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid member ID", false),
    };

    let old_member = state.service_context.member_repo.find_by_id(id).await.ok().flatten();

    if let Err(e) = state.service_context.member_repo.expire_dues_now(id).await {
        return partials::admin_alert("error", &format!("Error: {}", e), false);
    }

    // Force-logout so the member sees the expiration immediately
    // instead of on their next page load. Even if this fails,
    // middleware re-checks status per-request and bounces them
    // to /portal/restore — but log so operators notice.
    if let Err(e) = state.service_context.auth_service
        .invalidate_all_sessions(id)
        .await
    {
        tracing::error!(
            "Expired dues for member {} but failed to invalidate sessions: {}",
            id, e
        );
    }

    if let Some(old) = old_member {
        dispatch_member_updated(&state, id, old).await;
    }

    state.service_context.audit_service.log(
        Some(current_user.member.id),
        "expire_member_now",
        "member",
        &id.to_string(),
        None,
        None,
        None,
    ).await;
    partials::admin_alert("warning", "Member dues have been expired.", true)
}

/// Re-fetch the member after an update and fire `MemberUpdated` with
/// the old/new pair. Centralizes the post-update integration-event
/// dispatch so handlers don't each re-roll the find_by_id +
/// integration_manager dance, and so a missed dispatch is one
/// consistent fix-site instead of N. Silent on lookup failure — the
/// caller's update already succeeded; we don't want to invent a
/// rollback path.
async fn dispatch_member_updated(
    state: &AppState,
    id: uuid::Uuid,
    old: crate::domain::Member,
) {
    if let Ok(Some(new)) = state.service_context.member_repo.find_by_id(id).await {
        state.service_context.integration_manager
            .handle_event(crate::integrations::IntegrationEvent::MemberUpdated { old, new })
            .await;
    }
}

pub async fn admin_member_payments(
    State(state): State<AppState>,
    Extension(_current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid member ID", false),
    };

    let payments = state.service_context.payment_repo
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
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
}

pub async fn admin_new_member_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> axum::response::Response {
    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_else(|_| "error".to_string());

    let template = AdminNewMemberTemplate {
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
    };

    HtmlTemplate(template).into_response()
}

#[derive(Debug, Deserialize)]
pub struct AdminCreateMemberForm {
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub password: String,
    pub membership_type: String,
    pub status: String,
    pub notes: Option<String>,
    #[allow(dead_code)]
    pub csrf_token: String,
}

pub async fn admin_create_member(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    axum::Form(form): axum::Form<AdminCreateMemberForm>,
) -> axum::response::Response {
    use crate::domain::{CreateMemberRequest, MembershipType, MemberStatus, UpdateMemberRequest};

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

    let membership_type = match MembershipType::from_str(&form.membership_type) {
        Some(t) => t,
        None => return render_error("Invalid membership type."),
    };

    let create_request = CreateMemberRequest {
        email: form.email.clone(),
        username: form.username.clone(),
        full_name: form.full_name.clone(),
        password: form.password,
        membership_type,
    };

    match state.service_context.member_repo.create(create_request).await {
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
                if let Err(e) = state.service_context.member_repo.update(member.id, update).await {
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

            state.service_context.audit_service.log(
                Some(current_user.member.id),
                "create_member",
                "member",
                &member.id.to_string(),
                None,
                Some(&member.email),
                None,
            ).await;

            axum::response::Redirect::to(&format!("/portal/admin/members/{}", member.id)).into_response()
        }
        Err(e) => render_error(&e.to_string()),
    }
}

/// Send the welcome email after an admin activates a member.
async fn send_welcome_email(
    state: &AppState,
    member: &crate::domain::Member,
) -> crate::error::Result<()> {
    use crate::email::{self, templates::{WelcomeHtml, WelcomeText}};

    let portal_url = format!(
        "{}/portal/dashboard",
        state.settings.server.base_url.trim_end_matches('/'),
    );
    let org_name = state.service_context.settings_service
        .get_value("org.name")
        .await
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Coterie".to_string());

    // Pull the Discord invite URL from settings if the operator has
    // configured one. None → the welcome email omits the Discord
    // section entirely. Empty string is treated the same as None.
    let discord_invite = state.service_context.settings_service
        .get_value("discord.invite_url")
        .await
        .ok()
        .filter(|s| !s.is_empty());

    let html = WelcomeHtml {
        full_name: &member.full_name,
        org_name: &org_name,
        portal_url: &portal_url,
        discord_invite: discord_invite.as_deref(),
    };
    let text = WelcomeText {
        full_name: &member.full_name,
        org_name: &org_name,
        portal_url: &portal_url,
        discord_invite: discord_invite.as_deref(),
    };
    let message = email::message_from_templates(
        member.email.clone(),
        format!("Welcome to {}", org_name),
        &html,
        &text,
    )?;
    state.service_context.email_sender.send(&message).await
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
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
    axum::Form(form): axum::Form<UpdateDiscordIdForm>,
) -> impl IntoResponse {
    use crate::integrations::discord::is_valid_snowflake;

    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return discord_id_result(false, "Invalid member ID"),
    };

    let trimmed = form.discord_id.trim();
    let new_value: Option<&str> = if trimmed.is_empty() {
        None
    } else if !is_valid_snowflake(trimmed) {
        return discord_id_result(
            false,
            "Discord ID must be 17–20 digits (snowflake format). Right-click the user in Discord with Developer Mode on → Copy User ID.",
        );
    } else {
        Some(trimmed)
    };

    // Snapshot the old member so the integration sees the diff
    // (it'll strip roles from the old discord_id, apply to the new).
    let old_member = state.service_context.member_repo.find_by_id(id).await.ok().flatten();

    if let Err(e) = state.service_context.member_repo.update_discord_id(id, new_value).await {
        return discord_id_result(false, &format!("Failed to save: {}", e));
    }

    state.service_context.audit_service.log(
        Some(current_user.member.id),
        "update_discord_id",
        "member",
        &id.to_string(),
        old_member.as_ref().and_then(|m| m.discord_id.as_deref()),
        new_value,
        None,
    ).await;

    if let (Some(old), Ok(Some(new))) = (
        old_member,
        state.service_context.member_repo.find_by_id(id).await,
    ) {
        state.service_context.integration_manager
            .handle_event(crate::integrations::IntegrationEvent::MemberUpdated { old, new })
            .await;
    }

    let msg = match new_value {
        Some(v) => format!("Discord ID set to {} (role sync triggered).", v),
        None => "Discord ID cleared.".to_string(),
    };
    discord_id_result(true, &msg)
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
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> axum::response::Response {
    use crate::{
        auth::EmailTokenService,
        email::{self, templates::{VerifyHtml, VerifyText}},
    };


    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return resend_result(false, "Invalid member ID").into_response(),
    };

    let member = match state.service_context.member_repo.find_by_id(id).await {
        Ok(Some(m)) => m,
        Ok(None) => return resend_result(false, "Member not found").into_response(),
        Err(e) => return resend_result(false, &format!("DB error: {}", e)).into_response(),
    };

    if member.email_verified() {
        return resend_result(false, "Member's email is already verified").into_response();
    }

    let service = EmailTokenService::verification(state.service_context.db_pool.clone());

    // Invalidate any existing unconsumed tokens so only the newest link works.
    // If invalidation fails, the new token is still valid and works — but
    // any older tokens out in flight (e.g. in the member's spam folder
    // from a previous send) might still work too. Worth logging.
    if let Err(e) = service.invalidate_for_member(id).await {
        tracing::warn!(
            "Resending verification for {} but couldn't invalidate previous tokens: {}",
            id, e
        );
    }

    let created = match service.create(id, chrono::Duration::hours(24)).await {
        Ok(c) => c,
        Err(e) => return resend_result(false, &format!("Token create failed: {}", e)).into_response(),
    };

    let org_name = state.service_context.settings_service
        .get_value("org.name").await
        .ok().filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Coterie".to_string());
    let verify_url = format!(
        "{}/verify?token={}",
        state.settings.server.base_url.trim_end_matches('/'),
        created.token,
    );
    let html = VerifyHtml { full_name: &member.full_name, org_name: &org_name, verify_url: &verify_url };
    let text = VerifyText { full_name: &member.full_name, org_name: &org_name, verify_url: &verify_url };

    let message = match email::message_from_templates(
        member.email.clone(),
        format!("Verify your email for {}", org_name),
        &html,
        &text,
    ) {
        Ok(m) => m,
        Err(e) => return resend_result(false, &format!("Render failed: {}", e)).into_response(),
    };

    match state.service_context.email_sender.send(&message).await {
        Ok(()) => {
            state.service_context.audit_service.log(
                Some(current_user.member.id),
                "resend_verification",
                "member",
                &id.to_string(),
                None,
                Some(&member.email),
                None,
            ).await;
            resend_result(true, &format!("Verification email resent to {}.", member.email)).into_response()
        }
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
