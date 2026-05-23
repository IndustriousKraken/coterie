use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Extension,
};
use serde::Deserialize;

use crate::{
    api::middleware::auth::{CurrentUser, SessionInfo},
    auth::CsrfService,
    repository::{MemberRepository, SavedCardRepository},
    service::{member_service::MemberService, membership_type_service::MembershipTypeService},
    web::{
        portal::admin::partials,
        templates::{filters, BaseContext, HtmlTemplate},
    },
};

use super::MembershipTypeOption;

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

    let initials: String = member
        .full_name
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();

    let now = chrono::Utc::now();
    let dues_expired = member.dues_paid_until.map(|d| d < now).unwrap_or(true);

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

    let all_types = membership_type_service.list(true).await.unwrap_or_default();
    let type_name = all_types
        .iter()
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
        initials: if initials.is_empty() {
            "?".to_string()
        } else {
            initials
        },
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
        updated_at: member
            .updated_at
            .format("%B %d, %Y at %l:%M %p")
            .to_string(),
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

    match member_service
        .update(current_user.member.id, id, update)
        .await
    {
        Ok(_) => partials::admin_alert("success", "Member updated successfully!", false),
        Err(e) => partials::admin_alert("error", &format!("Error: {}", e), false),
    }
}
