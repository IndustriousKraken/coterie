use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
    Extension,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    api::{state::AppState, middleware::auth::CurrentUser},
    domain::{Event, EventType, EventVisibility},
    error::{AppError, Result},
};

#[derive(Debug, Deserialize)]
pub struct CreateEventRequest {
    pub title: String,
    pub description: String,
    pub event_type: EventType,
    pub visibility: EventVisibility,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub location: Option<String>,
    pub max_attendees: Option<i32>,
    pub rsvp_required: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateEventRequest {
    pub title: Option<String>,
    pub description: Option<String>,
    pub event_type: Option<EventType>,
    pub visibility: Option<EventVisibility>,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<Option<DateTime<Utc>>>,
    pub location: Option<Option<String>>,
    pub max_attendees: Option<Option<i32>>,
    pub rsvp_required: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ListEventsQuery {
    pub limit: Option<i64>,
    pub public_only: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct EventResponse {
    pub event: Event,
    pub attendee_count: Option<i32>,
}

pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListEventsQuery>,
    user: Option<Extension<CurrentUser>>,
) -> Result<Json<Vec<Event>>> {
    let limit = params.limit.unwrap_or(50).min(100);
    
    let events = if params.public_only.unwrap_or(false) || user.is_none() {
        // Show only public events if not authenticated or explicitly requested
        state.service_context.event_repo.list_public().await?
    } else {
        // Show upcoming events for authenticated users
        state.service_context.event_repo.list_upcoming(limit).await?
    };
    
    Ok(Json(events))
}

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    user: Option<Extension<CurrentUser>>,
) -> Result<Json<Event>> {
    let event = state.service_context.event_repo
        .find_by_id(id)
        .await?
        .ok_or(AppError::NotFound("Event not found".to_string()))?;
    
    // Check visibility permissions
    match event.visibility {
        EventVisibility::Public => {},
        EventVisibility::MembersOnly => {
            if user.is_none() {
                return Err(AppError::Forbidden);
            }
        },
        EventVisibility::AdminOnly => {
            // Would need to check admin status here
            if user.is_none() {
                return Err(AppError::Forbidden);
            }
        },
    }
    
    Ok(Json(event))
}

pub async fn create(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<CreateEventRequest>,
) -> Result<(StatusCode, Json<Event>)> {
    let event = Event {
        id: Uuid::new_v4(),
        title: request.title,
        description: request.description,
        event_type: request.event_type,
        event_type_id: None,
        visibility: request.visibility,
        start_time: request.start_time,
        end_time: request.end_time,
        location: request.location,
        max_attendees: request.max_attendees,
        rsvp_required: request.rsvp_required,
        created_by: user.member.id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    
    let created_event = state.service_context.event_repo.create(event).await?;
    
    Ok((StatusCode::CREATED, Json(created_event)))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<UpdateEventRequest>,
) -> Result<Json<Event>> {
    // Get the existing event
    let mut event = state.service_context.event_repo
        .find_by_id(id)
        .await?
        .ok_or(AppError::NotFound("Event not found".to_string()))?;
    
    // Check if user can update (must be creator or admin)
    if event.created_by != user.member.id {
        // TODO: Add admin check here
        return Err(AppError::Forbidden);
    }
    
    // Apply updates
    if let Some(title) = request.title {
        event.title = title;
    }
    if let Some(description) = request.description {
        event.description = description;
    }
    if let Some(event_type) = request.event_type {
        event.event_type = event_type;
    }
    if let Some(visibility) = request.visibility {
        event.visibility = visibility;
    }
    if let Some(start_time) = request.start_time {
        event.start_time = start_time;
    }
    if let Some(end_time) = request.end_time {
        event.end_time = end_time;
    }
    if let Some(location) = request.location {
        event.location = location;
    }
    if let Some(max_attendees) = request.max_attendees {
        event.max_attendees = max_attendees;
    }
    if let Some(rsvp_required) = request.rsvp_required {
        event.rsvp_required = rsvp_required;
    }
    
    event.updated_at = Utc::now();
    
    let updated_event = state.service_context.event_repo.update(id, event).await?;
    
    Ok(Json(updated_event))
}

pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Extension(user): Extension<CurrentUser>,
) -> Result<StatusCode> {
    // Get the event to check permissions
    let event = state.service_context.event_repo
        .find_by_id(id)
        .await?
        .ok_or(AppError::NotFound("Event not found".to_string()))?;
    
    // Check if user can delete (must be creator or admin)
    if event.created_by != user.member.id {
        // TODO: Add admin check here
        return Err(AppError::Forbidden);
    }
    
    state.service_context.event_repo.delete(id).await?;
    
    Ok(StatusCode::NO_CONTENT)
}

pub async fn register(
    State(state): State<AppState>,
    Path(event_id): Path<Uuid>,
    Extension(user): Extension<CurrentUser>,
) -> Result<(StatusCode, Json<serde_json::Value>)> {
    // Check if event exists and is open for registration
    let event = state.service_context.event_repo
        .find_by_id(event_id)
        .await?
        .ok_or(AppError::NotFound("Event not found".to_string()))?;
    
    // Check visibility permissions
    match event.visibility {
        EventVisibility::Public | EventVisibility::MembersOnly => {},
        EventVisibility::AdminOnly => {
            // TODO: Add admin check
            return Err(AppError::Forbidden);
        },
    }
    
    // Check if registration is required
    if !event.rsvp_required {
        return Ok((StatusCode::OK, Json(serde_json::json!({
            "message": "This event does not require registration"
        }))));
    }
    
    // Register the user
    state.service_context.event_repo
        .register_attendance(event_id, user.member.id)
        .await?;
    
    Ok((StatusCode::CREATED, Json(serde_json::json!({
        "status": "registered",
        "event_id": event_id,
        "member_id": user.member.id
    }))))
}

pub async fn cancel(
    State(state): State<AppState>,
    Path(event_id): Path<Uuid>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<serde_json::Value>> {
    // Check if event exists
    let event = state.service_context.event_repo
        .find_by_id(event_id)
        .await?
        .ok_or(AppError::NotFound("Event not found".to_string()))?;
    
    if !event.rsvp_required {
        return Ok(Json(serde_json::json!({
            "message": "This event does not require registration"
        })));
    }
    
    // Cancel the registration
    state.service_context.event_repo
        .cancel_attendance(event_id, user.member.id)
        .await?;
    
    Ok(Json(serde_json::json!({
        "status": "cancelled",
        "event_id": event_id,
        "member_id": user.member.id
    })))
}