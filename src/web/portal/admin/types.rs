//! Admin handlers for the three configurable type lists.
//!
//! Event and announcement types share one set of handlers parameterized by
//! `BasicTypeKind`; the kind comes from the URL path (`/types/:kind/...`).
//! Membership types keep their own handler set because membership has extra
//! fields (fee, billing period) and extra validation.

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Extension,
};
use serde::Deserialize;

use crate::{
    api::{
        middleware::auth::{CurrentUser, SessionInfo},
        state::{AnnouncementBasicTypeService, EventBasicTypeService},
    },
    auth::CsrfService,
    domain::{
        BasicTypeKind, CreateBasicTypeRequest, CreateMembershipTypeRequest,
        UpdateBasicTypeRequest, UpdateMembershipTypeRequest,
    },
    service::{
        basic_type_service::BasicTypeService, membership_type_service::MembershipTypeService,
    },
    util::string::capitalize_first,
    web::{
        portal::admin::partials,
        templates::{BaseContext, HtmlTemplate},
    },
};

// =============================================================================
// Type Info Structs for Templates
// =============================================================================

#[derive(Clone)]
pub struct TypeInfo {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub sort_order: i32,
    pub is_active: bool,
    pub usage_count: i64,
}

#[derive(Clone)]
pub struct MembershipTypeInfo {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub sort_order: i32,
    pub is_active: bool,
    pub fee_cents: i32,
    pub fee_dollars: String,
    pub billing_period: String,
    pub usage_count: i64,
}

// =============================================================================
// Types Overview Page (lists all three type categories)
// =============================================================================

#[derive(Template)]
#[template(path = "admin/types/index.html")]
pub struct AdminTypesTemplate {
    pub base: BaseContext,
    pub event_types: Vec<TypeInfo>,
    pub announcement_types: Vec<TypeInfo>,
    pub membership_types: Vec<MembershipTypeInfo>,
}

pub async fn admin_types_page(
    State(event_type_service): State<EventBasicTypeService>,
    State(announcement_type_service): State<AnnouncementBasicTypeService>,
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> impl IntoResponse {
    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;
    let event_types = fetch_basic_types(&event_type_service.0, true).await;
    let announcement_types = fetch_basic_types(&announcement_type_service.0, true).await;
    let membership_types = fetch_membership_types(&membership_type_service, true).await;

    HtmlTemplate(AdminTypesTemplate {
        base,
        event_types,
        announcement_types,
        membership_types,
    }).into_response()
}

// =============================================================================
// Basic Types (Event + Announcement) Management
// =============================================================================
//
// Two Askama template structs exist because the two form templates use
// different field names (`event_type` vs `announcement_type`). The handler
// code is shared and dispatches on `BasicTypeKind` at the very edge.

#[derive(Template)]
#[template(path = "admin/types/event_type_form.html")]
pub struct EventTypeFormTemplate {
    pub base: BaseContext,
    pub event_type: Option<TypeInfo>,
    pub is_edit: bool,
}

#[derive(Template)]
#[template(path = "admin/types/announcement_type_form.html")]
pub struct AnnouncementTypeFormTemplate {
    pub base: BaseContext,
    pub announcement_type: Option<TypeInfo>,
    pub is_edit: bool,
}

fn parse_kind(kind_str: &str) -> Option<BasicTypeKind> {
    match kind_str {
        "event" => Some(BasicTypeKind::Event),
        "announcement" => Some(BasicTypeKind::Announcement),
        _ => None,
    }
}

fn invalid_kind_response() -> Response {
    partials::admin_alert("error", "Unknown type kind", false).into_response()
}

fn render_basic_form(
    kind: BasicTypeKind,
    base: BaseContext,
    type_info: Option<TypeInfo>,
    is_edit: bool,
) -> Response {
    match kind {
        BasicTypeKind::Event => HtmlTemplate(EventTypeFormTemplate {
            base,
            event_type: type_info,
            is_edit,
        })
        .into_response(),
        BasicTypeKind::Announcement => HtmlTemplate(AnnouncementTypeFormTemplate {
            base,
            announcement_type: type_info,
            is_edit,
        })
        .into_response(),
    }
}

fn service_for<'a>(
    event_type_service: &'a Arc<BasicTypeService>,
    announcement_type_service: &'a Arc<BasicTypeService>,
    kind: BasicTypeKind,
) -> &'a Arc<BasicTypeService> {
    match kind {
        BasicTypeKind::Event => event_type_service,
        BasicTypeKind::Announcement => announcement_type_service,
    }
}

pub async fn admin_new_basic_type_page(
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(kind_str): Path<String>,
) -> Response {
    let Some(kind) = parse_kind(&kind_str) else {
        return invalid_kind_response();
    };
    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;
    render_basic_form(kind, base, None, false)
}

pub async fn admin_edit_basic_type_page(
    State(event_type_service): State<EventBasicTypeService>,
    State(announcement_type_service): State<AnnouncementBasicTypeService>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path((kind_str, type_id)): Path<(String, String)>,
) -> Response {
    let Some(kind) = parse_kind(&kind_str) else {
        return invalid_kind_response();
    };
    let id = match uuid::Uuid::parse_str(&type_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid type ID", false).into_response(),
    };

    let kind_label = kind.display_name();
    let svc = service_for(&event_type_service.0, &announcement_type_service.0, kind);
    let basic_type = match svc.get(id).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            return partials::admin_alert(
                "error",
                &format!("{} not found", capitalize_first(kind_label)),
                false,
            )
            .into_response()
        }
        Err(_) => {
            return partials::admin_alert(
                "error",
                &format!("Error loading {}", kind_label),
                false,
            )
            .into_response()
        }
    };

    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;
    let type_info = TypeInfo {
        id: basic_type.id.to_string(),
        name: basic_type.name,
        slug: basic_type.slug,
        description: basic_type.description,
        color: basic_type.color,
        icon: basic_type.icon,
        sort_order: basic_type.sort_order,
        is_active: basic_type.is_active,
        usage_count: 0,
    };

    render_basic_form(kind, base, Some(type_info), true)
}

// Form body for both event-type and announcement-type create/update.
// Note: csrf_token is validated via X-CSRF-Token header in middleware,
// not from form body, so it's not included here.
#[derive(Debug, Deserialize)]
pub struct BasicTypeForm {
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub is_active: Option<String>,
}

pub async fn admin_create_basic_type(
    State(event_type_service): State<EventBasicTypeService>,
    State(announcement_type_service): State<AnnouncementBasicTypeService>,
    Extension(_current_user): Extension<CurrentUser>,
    Path(kind_str): Path<String>,
    axum::Form(form): axum::Form<BasicTypeForm>,
) -> Response {
    let Some(kind) = parse_kind(&kind_str) else {
        return invalid_kind_response();
    };
    let request = CreateBasicTypeRequest {
        name: form.name,
        slug: form.slug.filter(|s| !s.is_empty()),
        description: form.description.filter(|s| !s.is_empty()),
        color: form.color.filter(|s| !s.is_empty()),
        icon: form.icon.filter(|s| !s.is_empty()),
    };

    let svc = service_for(&event_type_service.0, &announcement_type_service.0, kind);
    match svc.create(request).await {
        Ok(_) => axum::response::Redirect::to("/portal/admin/types").into_response(),
        Err(e) => partials::admin_alert(
            "error",
            &format!("Error creating {}: {}", kind.display_name(), e),
            false,
        )
        .into_response(),
    }
}

pub async fn admin_update_basic_type(
    State(event_type_service): State<EventBasicTypeService>,
    State(announcement_type_service): State<AnnouncementBasicTypeService>,
    Extension(_current_user): Extension<CurrentUser>,
    Path((kind_str, type_id)): Path<(String, String)>,
    axum::Form(form): axum::Form<BasicTypeForm>,
) -> Response {
    let Some(kind) = parse_kind(&kind_str) else {
        return invalid_kind_response();
    };
    let id = match uuid::Uuid::parse_str(&type_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid type ID", false).into_response(),
    };

    let request = UpdateBasicTypeRequest {
        name: Some(form.name),
        description: form.description,
        color: form.color,
        icon: form.icon,
        sort_order: None,
        is_active: Some(form.is_active.is_some()),
    };

    let svc = service_for(&event_type_service.0, &announcement_type_service.0, kind);
    match svc.update(id, request).await {
        Ok(_) => axum::response::Redirect::to("/portal/admin/types").into_response(),
        Err(e) => partials::admin_alert(
            "error",
            &format!("Error updating {}: {}", kind.display_name(), e),
            false,
        )
        .into_response(),
    }
}

pub async fn admin_delete_basic_type(
    State(event_type_service): State<EventBasicTypeService>,
    State(announcement_type_service): State<AnnouncementBasicTypeService>,
    Extension(_current_user): Extension<CurrentUser>,
    Path((kind_str, type_id)): Path<(String, String)>,
) -> Response {
    let Some(kind) = parse_kind(&kind_str) else {
        return invalid_kind_response();
    };
    let id = match uuid::Uuid::parse_str(&type_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid type ID", false).into_response(),
    };

    let svc = service_for(&event_type_service.0, &announcement_type_service.0, kind);
    match svc.delete(id).await {
        Ok(_) => axum::response::Redirect::to("/portal/admin/types").into_response(),
        Err(e) => partials::admin_alert(
            "error",
            &format!("Error deleting {}: {}", kind.display_name(), e),
            false,
        )
        .into_response(),
    }
}

// =============================================================================
// Membership Types Management
// =============================================================================

#[derive(Template)]
#[template(path = "admin/types/membership_type_form.html")]
pub struct MembershipTypeFormTemplate {
    pub base: BaseContext,
    pub membership_type: Option<MembershipTypeInfo>,
    pub is_edit: bool,
}

pub async fn admin_new_membership_type_page(
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> impl IntoResponse {
    HtmlTemplate(MembershipTypeFormTemplate {
        base: BaseContext::for_member(&csrf_service, &current_user, &session_info).await,
        membership_type: None,
        is_edit: false,
    }).into_response()
}

pub async fn admin_edit_membership_type_page(
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(type_id): Path<String>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&type_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid type ID", false).into_response(),
    };

    let membership_type = match membership_type_service.get(id).await {
        Ok(Some(t)) => t,
        Ok(None) => return partials::admin_alert("error", "Membership type not found", false).into_response(),
        Err(_) => return partials::admin_alert("error", "Error loading membership type", false).into_response(),
    };

    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;

    let fee_dollars = membership_type.fee_dollars();
    let type_info = MembershipTypeInfo {
        id: membership_type.id.to_string(),
        name: membership_type.name,
        slug: membership_type.slug,
        description: membership_type.description,
        color: membership_type.color,
        icon: membership_type.icon,
        sort_order: membership_type.sort_order,
        is_active: membership_type.is_active,
        fee_cents: membership_type.fee_cents,
        fee_dollars: format!("{:.2}", fee_dollars),
        billing_period: membership_type.billing_period,
        usage_count: 0,
    };

    HtmlTemplate(MembershipTypeFormTemplate {
        base,
        membership_type: Some(type_info),
        is_edit: true,
    }).into_response()
}

#[derive(Debug, Deserialize)]
pub struct MembershipTypeForm {
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub fee_dollars: String,
    pub billing_period: String,
    pub is_active: Option<String>,
}

pub async fn admin_create_membership_type(
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    Extension(_current_user): Extension<CurrentUser>,
    axum::Form(form): axum::Form<MembershipTypeForm>,
) -> impl IntoResponse {
    let fee_cents = match form.fee_dollars.parse::<f64>() {
        Ok(dollars) if dollars.is_finite() && dollars >= 0.0 && dollars <= 999_999.99 => {
            (dollars * 100.0).round() as i32
        }
        Ok(_) => return partials::admin_alert("error", "Fee must be between $0.00 and $999,999.99", false).into_response(),
        Err(_) => return partials::admin_alert("error", "Invalid fee amount", false).into_response(),
    };

    let request = CreateMembershipTypeRequest {
        name: form.name,
        slug: form.slug.filter(|s| !s.is_empty()),
        description: form.description.filter(|s| !s.is_empty()),
        color: form.color.filter(|s| !s.is_empty()),
        icon: form.icon.filter(|s| !s.is_empty()),
        fee_cents,
        billing_period: form.billing_period,
    };

    match membership_type_service.create(request).await {
        Ok(_) => axum::response::Redirect::to("/portal/admin/types").into_response(),
        Err(e) => partials::admin_alert("error", &format!("Error creating membership type: {}", e), false).into_response(),
    }
}

pub async fn admin_update_membership_type(
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    Extension(_current_user): Extension<CurrentUser>,
    Path(type_id): Path<String>,
    axum::Form(form): axum::Form<MembershipTypeForm>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&type_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid type ID", false).into_response(),
    };

    let fee_cents = match form.fee_dollars.parse::<f64>() {
        Ok(dollars) if dollars.is_finite() && dollars >= 0.0 && dollars <= 999_999.99 => {
            (dollars * 100.0).round() as i32
        }
        Ok(_) => return partials::admin_alert("error", "Fee must be between $0.00 and $999,999.99", false).into_response(),
        Err(_) => return partials::admin_alert("error", "Invalid fee amount", false).into_response(),
    };

    let request = UpdateMembershipTypeRequest {
        name: Some(form.name),
        description: form.description,
        color: form.color,
        icon: form.icon,
        sort_order: None,
        is_active: Some(form.is_active.is_some()),
        fee_cents: Some(fee_cents),
        billing_period: Some(form.billing_period),
    };

    match membership_type_service.update(id, request).await {
        Ok(_) => axum::response::Redirect::to("/portal/admin/types").into_response(),
        Err(e) => partials::admin_alert("error", &format!("Error updating membership type: {}", e), false).into_response(),
    }
}

pub async fn admin_delete_membership_type(
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    Extension(_current_user): Extension<CurrentUser>,
    Path(type_id): Path<String>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&type_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid type ID", false).into_response(),
    };

    match membership_type_service.delete(id).await {
        Ok(_) => axum::response::Redirect::to("/portal/admin/types").into_response(),
        Err(e) => partials::admin_alert("error", &format!("Error deleting membership type: {}", e), false).into_response(),
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

async fn fetch_basic_types(
    service: &BasicTypeService,
    include_inactive: bool,
) -> Vec<TypeInfo> {
    service
        .list(include_inactive)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|t| TypeInfo {
            id: t.id.to_string(),
            name: t.name,
            slug: t.slug,
            description: t.description,
            color: t.color,
            icon: t.icon,
            sort_order: t.sort_order,
            is_active: t.is_active,
            usage_count: 0,
        })
        .collect()
}

async fn fetch_membership_types(
    service: &MembershipTypeService,
    include_inactive: bool,
) -> Vec<MembershipTypeInfo> {
    service
        .list(include_inactive)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|t| {
            let fee_dollars = t.fee_dollars();
            MembershipTypeInfo {
                id: t.id.to_string(),
                name: t.name,
                slug: t.slug,
                description: t.description,
                color: t.color,
                icon: t.icon,
                sort_order: t.sort_order,
                is_active: t.is_active,
                fee_cents: t.fee_cents,
                fee_dollars: format!("{:.2}", fee_dollars),
                billing_period: t.billing_period,
                usage_count: 0,
            }
        })
        .collect()
}
