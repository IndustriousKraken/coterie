use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
    Extension,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    api::{state::AppState, middleware::auth::CurrentUser},
    domain::{
        EventTypeConfig, AnnouncementTypeConfig, MembershipTypeConfig,
        CreateEventTypeRequest, CreateAnnouncementTypeRequest, CreateMembershipTypeRequest,
        UpdateEventTypeRequest, UpdateAnnouncementTypeRequest, UpdateMembershipTypeRequest,
    },
    error::Result,
    service::MembershipPricing,
};

// =============================================================================
// Response Types
// =============================================================================

#[derive(Debug, Serialize)]
pub struct TypesOverview {
    pub event_types: Vec<EventTypeConfig>,
    pub announcement_types: Vec<AnnouncementTypeConfig>,
    pub membership_types: Vec<MembershipTypeConfig>,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub include_inactive: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ReorderRequest {
    pub ids: Vec<Uuid>,
}

// =============================================================================
// Overview Endpoint - Get all types at once
// =============================================================================

pub async fn get_all_types(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    axum::extract::Query(query): axum::extract::Query<ListQuery>,
) -> Result<Json<TypesOverview>> {
    let include_inactive = query.include_inactive.unwrap_or(false);

    let (event_types, announcement_types, membership_types) = tokio::join!(
        state.service_context.event_type_service.list(include_inactive),
        state.service_context.announcement_type_service.list(include_inactive),
        state.service_context.membership_type_service.list(include_inactive),
    );

    Ok(Json(TypesOverview {
        event_types: event_types?,
        announcement_types: announcement_types?,
        membership_types: membership_types?,
    }))
}

// =============================================================================
// Event Types
// =============================================================================

pub async fn list_event_types(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<ListQuery>,
) -> Result<Json<Vec<EventTypeConfig>>> {
    let include_inactive = query.include_inactive.unwrap_or(false);
    let types = state.service_context.event_type_service.list(include_inactive).await?;
    Ok(Json(types))
}

pub async fn get_event_type(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<EventTypeConfig>> {
    let type_config = state.service_context.event_type_service.get(id).await?
        .ok_or_else(|| crate::error::AppError::NotFound("Event type not found".to_string()))?;
    Ok(Json(type_config))
}

pub async fn get_event_type_by_slug(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<EventTypeConfig>> {
    let type_config = state.service_context.event_type_service.get_by_slug(&slug).await?
        .ok_or_else(|| crate::error::AppError::NotFound("Event type not found".to_string()))?;
    Ok(Json(type_config))
}

pub async fn create_event_type(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Json(request): Json<CreateEventTypeRequest>,
) -> Result<(StatusCode, Json<EventTypeConfig>)> {
    let created = state.service_context.event_type_service.create(request).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

pub async fn update_event_type(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateEventTypeRequest>,
) -> Result<Json<EventTypeConfig>> {
    let updated = state.service_context.event_type_service.update(id, request).await?;
    Ok(Json(updated))
}

pub async fn delete_event_type(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode> {
    state.service_context.event_type_service.delete(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn reorder_event_types(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Json(request): Json<ReorderRequest>,
) -> Result<StatusCode> {
    state.service_context.event_type_service.reorder(&request.ids).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn seed_event_types(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<Vec<EventTypeConfig>>> {
    let types = state.service_context.event_type_service.seed_defaults().await?;
    Ok(Json(types))
}

// =============================================================================
// Announcement Types
// =============================================================================

pub async fn list_announcement_types(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<ListQuery>,
) -> Result<Json<Vec<AnnouncementTypeConfig>>> {
    let include_inactive = query.include_inactive.unwrap_or(false);
    let types = state.service_context.announcement_type_service.list(include_inactive).await?;
    Ok(Json(types))
}

pub async fn get_announcement_type(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<AnnouncementTypeConfig>> {
    let type_config = state.service_context.announcement_type_service.get(id).await?
        .ok_or_else(|| crate::error::AppError::NotFound("Announcement type not found".to_string()))?;
    Ok(Json(type_config))
}

pub async fn get_announcement_type_by_slug(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<AnnouncementTypeConfig>> {
    let type_config = state.service_context.announcement_type_service.get_by_slug(&slug).await?
        .ok_or_else(|| crate::error::AppError::NotFound("Announcement type not found".to_string()))?;
    Ok(Json(type_config))
}

pub async fn create_announcement_type(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Json(request): Json<CreateAnnouncementTypeRequest>,
) -> Result<(StatusCode, Json<AnnouncementTypeConfig>)> {
    let created = state.service_context.announcement_type_service.create(request).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

pub async fn update_announcement_type(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateAnnouncementTypeRequest>,
) -> Result<Json<AnnouncementTypeConfig>> {
    let updated = state.service_context.announcement_type_service.update(id, request).await?;
    Ok(Json(updated))
}

pub async fn delete_announcement_type(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode> {
    state.service_context.announcement_type_service.delete(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn reorder_announcement_types(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Json(request): Json<ReorderRequest>,
) -> Result<StatusCode> {
    state.service_context.announcement_type_service.reorder(&request.ids).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn seed_announcement_types(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<Vec<AnnouncementTypeConfig>>> {
    let types = state.service_context.announcement_type_service.seed_defaults().await?;
    Ok(Json(types))
}

// =============================================================================
// Membership Types
// =============================================================================

pub async fn list_membership_types(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<ListQuery>,
) -> Result<Json<Vec<MembershipTypeConfig>>> {
    let include_inactive = query.include_inactive.unwrap_or(false);
    let types = state.service_context.membership_type_service.list(include_inactive).await?;
    Ok(Json(types))
}

pub async fn get_membership_type(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<MembershipTypeConfig>> {
    let type_config = state.service_context.membership_type_service.get(id).await?
        .ok_or_else(|| crate::error::AppError::NotFound("Membership type not found".to_string()))?;
    Ok(Json(type_config))
}

pub async fn get_membership_type_by_slug(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<MembershipTypeConfig>> {
    let type_config = state.service_context.membership_type_service.get_by_slug(&slug).await?
        .ok_or_else(|| crate::error::AppError::NotFound("Membership type not found".to_string()))?;
    Ok(Json(type_config))
}

pub async fn create_membership_type(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Json(request): Json<CreateMembershipTypeRequest>,
) -> Result<(StatusCode, Json<MembershipTypeConfig>)> {
    let created = state.service_context.membership_type_service.create(request).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

pub async fn update_membership_type(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateMembershipTypeRequest>,
) -> Result<Json<MembershipTypeConfig>> {
    let updated = state.service_context.membership_type_service.update(id, request).await?;
    Ok(Json(updated))
}

pub async fn delete_membership_type(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode> {
    state.service_context.membership_type_service.delete(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn reorder_membership_types(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Json(request): Json<ReorderRequest>,
) -> Result<StatusCode> {
    state.service_context.membership_type_service.reorder(&request.ids).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn seed_membership_types(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<Vec<MembershipTypeConfig>>> {
    let types = state.service_context.membership_type_service.seed_defaults().await?;
    Ok(Json(types))
}

// =============================================================================
// Membership Pricing Endpoints
// =============================================================================

pub async fn get_membership_pricing(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<MembershipPricing>> {
    let pricing = state.service_context.membership_type_service.get_pricing(id).await?;
    Ok(Json(pricing))
}

pub async fn list_all_pricing(
    State(state): State<AppState>,
) -> Result<Json<Vec<MembershipPricing>>> {
    let pricing = state.service_context.membership_type_service.get_all_pricing().await?;
    Ok(Json(pricing))
}
