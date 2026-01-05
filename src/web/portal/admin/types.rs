use askama::Template;
use axum::{
    extract::{State, Path},
    response::IntoResponse,
    Extension,
};
use serde::Deserialize;

use crate::{
    api::{
        middleware::auth::{CurrentUser, SessionInfo},
        state::AppState,
    },
    domain::{
        CreateEventTypeRequest, CreateAnnouncementTypeRequest, CreateMembershipTypeRequest,
        UpdateEventTypeRequest, UpdateAnnouncementTypeRequest, UpdateMembershipTypeRequest,
    },
    web::templates::{HtmlTemplate, UserInfo},
};
use crate::web::portal::is_admin;

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
    pub is_system: bool,
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
    pub is_system: bool,
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
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
    pub event_types: Vec<TypeInfo>,
    pub announcement_types: Vec<TypeInfo>,
    pub membership_types: Vec<MembershipTypeInfo>,
}

pub async fn admin_types_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_else(|_| "error".to_string());

    // Fetch all types (including inactive for admin view)
    let event_types = fetch_event_types(&state, true).await;
    let announcement_types = fetch_announcement_types(&state, true).await;
    let membership_types = fetch_membership_types(&state, true).await;

    HtmlTemplate(AdminTypesTemplate {
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
        event_types,
        announcement_types,
        membership_types,
    }).into_response()
}

// =============================================================================
// Event Types Management
// =============================================================================

#[derive(Template)]
#[template(path = "admin/types/event_type_form.html")]
pub struct EventTypeFormTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
    pub event_type: Option<TypeInfo>,
    pub is_edit: bool,
}

pub async fn admin_new_event_type_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_else(|_| "error".to_string());

    HtmlTemplate(EventTypeFormTemplate {
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
        event_type: None,
        is_edit: false,
    }).into_response()
}

pub async fn admin_edit_event_type_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(type_id): Path<String>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let id = match uuid::Uuid::parse_str(&type_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("Invalid type ID".to_string()).into_response(),
    };

    let event_type = match state.service_context.event_type_service.get(id).await {
        Ok(Some(t)) => t,
        Ok(None) => return axum::response::Html("Event type not found".to_string()).into_response(),
        Err(_) => return axum::response::Html("Error loading event type".to_string()).into_response(),
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

    let type_info = TypeInfo {
        id: event_type.id.to_string(),
        name: event_type.name,
        slug: event_type.slug,
        description: event_type.description,
        color: event_type.color,
        icon: event_type.icon,
        sort_order: event_type.sort_order,
        is_active: event_type.is_active,
        is_system: event_type.is_system,
        usage_count: 0,
    };

    HtmlTemplate(EventTypeFormTemplate {
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
        event_type: Some(type_info),
        is_edit: true,
    }).into_response()
}

#[derive(Debug, Deserialize)]
pub struct EventTypeForm {
    pub csrf_token: String,
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub is_active: Option<String>,
}

pub async fn admin_create_event_type(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    axum::Form(form): axum::Form<EventTypeForm>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let request = CreateEventTypeRequest {
        name: form.name,
        slug: form.slug.filter(|s| !s.is_empty()),
        description: form.description.filter(|s| !s.is_empty()),
        color: form.color.filter(|s| !s.is_empty()),
        icon: form.icon.filter(|s| !s.is_empty()),
    };

    match state.service_context.event_type_service.create(request).await {
        Ok(_) => axum::response::Redirect::to("/portal/admin/types").into_response(),
        Err(e) => axum::response::Html(format!("Error creating event type: {}", e)).into_response(),
    }
}

pub async fn admin_update_event_type(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(type_id): Path<String>,
    axum::Form(form): axum::Form<EventTypeForm>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let id = match uuid::Uuid::parse_str(&type_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("Invalid type ID".to_string()).into_response(),
    };

    let request = UpdateEventTypeRequest {
        name: Some(form.name),
        description: form.description,
        color: form.color,
        icon: form.icon,
        sort_order: None,
        is_active: Some(form.is_active.is_some()),
    };

    match state.service_context.event_type_service.update(id, request).await {
        Ok(_) => axum::response::Redirect::to("/portal/admin/types").into_response(),
        Err(e) => axum::response::Html(format!("Error updating event type: {}", e)).into_response(),
    }
}

pub async fn admin_delete_event_type(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(type_id): Path<String>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let id = match uuid::Uuid::parse_str(&type_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("Invalid type ID".to_string()).into_response(),
    };

    match state.service_context.event_type_service.delete(id).await {
        Ok(_) => axum::response::Redirect::to("/portal/admin/types").into_response(),
        Err(e) => axum::response::Html(format!("Error deleting event type: {}", e)).into_response(),
    }
}

// =============================================================================
// Announcement Types Management
// =============================================================================

#[derive(Template)]
#[template(path = "admin/types/announcement_type_form.html")]
pub struct AnnouncementTypeFormTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
    pub announcement_type: Option<TypeInfo>,
    pub is_edit: bool,
}

pub async fn admin_new_announcement_type_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_else(|_| "error".to_string());

    HtmlTemplate(AnnouncementTypeFormTemplate {
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
        announcement_type: None,
        is_edit: false,
    }).into_response()
}

pub async fn admin_edit_announcement_type_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(type_id): Path<String>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let id = match uuid::Uuid::parse_str(&type_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("Invalid type ID".to_string()).into_response(),
    };

    let announcement_type = match state.service_context.announcement_type_service.get(id).await {
        Ok(Some(t)) => t,
        Ok(None) => return axum::response::Html("Announcement type not found".to_string()).into_response(),
        Err(_) => return axum::response::Html("Error loading announcement type".to_string()).into_response(),
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

    let type_info = TypeInfo {
        id: announcement_type.id.to_string(),
        name: announcement_type.name,
        slug: announcement_type.slug,
        description: announcement_type.description,
        color: announcement_type.color,
        icon: announcement_type.icon,
        sort_order: announcement_type.sort_order,
        is_active: announcement_type.is_active,
        is_system: announcement_type.is_system,
        usage_count: 0,
    };

    HtmlTemplate(AnnouncementTypeFormTemplate {
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
        announcement_type: Some(type_info),
        is_edit: true,
    }).into_response()
}

#[derive(Debug, Deserialize)]
pub struct AnnouncementTypeForm {
    pub csrf_token: String,
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub is_active: Option<String>,
}

pub async fn admin_create_announcement_type(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    axum::Form(form): axum::Form<AnnouncementTypeForm>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let request = CreateAnnouncementTypeRequest {
        name: form.name,
        slug: form.slug.filter(|s| !s.is_empty()),
        description: form.description.filter(|s| !s.is_empty()),
        color: form.color.filter(|s| !s.is_empty()),
        icon: form.icon.filter(|s| !s.is_empty()),
    };

    match state.service_context.announcement_type_service.create(request).await {
        Ok(_) => axum::response::Redirect::to("/portal/admin/types").into_response(),
        Err(e) => axum::response::Html(format!("Error creating announcement type: {}", e)).into_response(),
    }
}

pub async fn admin_update_announcement_type(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(type_id): Path<String>,
    axum::Form(form): axum::Form<AnnouncementTypeForm>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let id = match uuid::Uuid::parse_str(&type_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("Invalid type ID".to_string()).into_response(),
    };

    let request = UpdateAnnouncementTypeRequest {
        name: Some(form.name),
        description: form.description,
        color: form.color,
        icon: form.icon,
        sort_order: None,
        is_active: Some(form.is_active.is_some()),
    };

    match state.service_context.announcement_type_service.update(id, request).await {
        Ok(_) => axum::response::Redirect::to("/portal/admin/types").into_response(),
        Err(e) => axum::response::Html(format!("Error updating announcement type: {}", e)).into_response(),
    }
}

pub async fn admin_delete_announcement_type(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(type_id): Path<String>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let id = match uuid::Uuid::parse_str(&type_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("Invalid type ID".to_string()).into_response(),
    };

    match state.service_context.announcement_type_service.delete(id).await {
        Ok(_) => axum::response::Redirect::to("/portal/admin/types").into_response(),
        Err(e) => axum::response::Html(format!("Error deleting announcement type: {}", e)).into_response(),
    }
}

// =============================================================================
// Membership Types Management
// =============================================================================

#[derive(Template)]
#[template(path = "admin/types/membership_type_form.html")]
pub struct MembershipTypeFormTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
    pub membership_type: Option<MembershipTypeInfo>,
    pub is_edit: bool,
}

pub async fn admin_new_membership_type_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_else(|_| "error".to_string());

    HtmlTemplate(MembershipTypeFormTemplate {
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
        membership_type: None,
        is_edit: false,
    }).into_response()
}

pub async fn admin_edit_membership_type_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(type_id): Path<String>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let id = match uuid::Uuid::parse_str(&type_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("Invalid type ID".to_string()).into_response(),
    };

    let membership_type = match state.service_context.membership_type_service.get(id).await {
        Ok(Some(t)) => t,
        Ok(None) => return axum::response::Html("Membership type not found".to_string()).into_response(),
        Err(_) => return axum::response::Html("Error loading membership type".to_string()).into_response(),
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
        is_system: membership_type.is_system,
        fee_cents: membership_type.fee_cents,
        fee_dollars: format!("{:.2}", fee_dollars),
        billing_period: membership_type.billing_period,
        usage_count: 0,
    };

    HtmlTemplate(MembershipTypeFormTemplate {
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
        membership_type: Some(type_info),
        is_edit: true,
    }).into_response()
}

#[derive(Debug, Deserialize)]
pub struct MembershipTypeForm {
    pub csrf_token: String,
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
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    axum::Form(form): axum::Form<MembershipTypeForm>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    // Parse fee from dollars to cents
    let fee_cents = match form.fee_dollars.parse::<f64>() {
        Ok(dollars) => (dollars * 100.0).round() as i32,
        Err(_) => return axum::response::Html("Invalid fee amount".to_string()).into_response(),
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

    match state.service_context.membership_type_service.create(request).await {
        Ok(_) => axum::response::Redirect::to("/portal/admin/types").into_response(),
        Err(e) => axum::response::Html(format!("Error creating membership type: {}", e)).into_response(),
    }
}

pub async fn admin_update_membership_type(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(type_id): Path<String>,
    axum::Form(form): axum::Form<MembershipTypeForm>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let id = match uuid::Uuid::parse_str(&type_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("Invalid type ID".to_string()).into_response(),
    };

    // Parse fee from dollars to cents
    let fee_cents = match form.fee_dollars.parse::<f64>() {
        Ok(dollars) => (dollars * 100.0).round() as i32,
        Err(_) => return axum::response::Html("Invalid fee amount".to_string()).into_response(),
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

    match state.service_context.membership_type_service.update(id, request).await {
        Ok(_) => axum::response::Redirect::to("/portal/admin/types").into_response(),
        Err(e) => axum::response::Html(format!("Error updating membership type: {}", e)).into_response(),
    }
}

pub async fn admin_delete_membership_type(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(type_id): Path<String>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let id = match uuid::Uuid::parse_str(&type_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("Invalid type ID".to_string()).into_response(),
    };

    match state.service_context.membership_type_service.delete(id).await {
        Ok(_) => axum::response::Redirect::to("/portal/admin/types").into_response(),
        Err(e) => axum::response::Html(format!("Error deleting membership type: {}", e)).into_response(),
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

async fn fetch_event_types(state: &AppState, include_inactive: bool) -> Vec<TypeInfo> {
    state.service_context.event_type_service
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
            is_system: t.is_system,
            usage_count: 0,
        })
        .collect()
}

async fn fetch_announcement_types(state: &AppState, include_inactive: bool) -> Vec<TypeInfo> {
    state.service_context.announcement_type_service
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
            is_system: t.is_system,
            usage_count: 0,
        })
        .collect()
}

async fn fetch_membership_types(state: &AppState, include_inactive: bool) -> Vec<MembershipTypeInfo> {
    state.service_context.membership_type_service
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
                is_system: t.is_system,
                fee_cents: t.fee_cents,
                fee_dollars: format!("{:.2}", fee_dollars),
                billing_period: t.billing_period,
                usage_count: 0,
            }
        })
        .collect()
}
