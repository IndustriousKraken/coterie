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
    domain::{Announcement, AnnouncementType},
    error::{AppError, Result},
};

#[derive(Debug, Deserialize)]
pub struct CreateAnnouncementRequest {
    pub title: String,
    pub content: String,
    pub announcement_type: AnnouncementType,
    pub is_public: bool,
    pub featured: bool,
    pub publish_now: bool, // If true, set published_at to now
}

#[derive(Debug, Deserialize)]
pub struct UpdateAnnouncementRequest {
    pub title: Option<String>,
    pub content: Option<String>,
    pub announcement_type: Option<AnnouncementType>,
    pub is_public: Option<bool>,
    pub featured: Option<bool>,
    pub published_at: Option<Option<DateTime<Utc>>>,
}

#[derive(Debug, Deserialize)]
pub struct ListAnnouncementsQuery {
    pub limit: Option<i64>,
    pub public_only: Option<bool>,
    pub featured_only: Option<bool>,
}

pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListAnnouncementsQuery>,
    user: Option<Extension<CurrentUser>>,
) -> Result<Json<Vec<Announcement>>> {
    let limit = params.limit.unwrap_or(20).min(100);
    
    let announcements = if params.featured_only.unwrap_or(false) {
        state.service_context.announcement_repo.list_featured().await?
    } else if params.public_only.unwrap_or(false) || user.is_none() {
        state.service_context.announcement_repo.list_public().await?
    } else {
        state.service_context.announcement_repo.list_recent(limit).await?
    };
    
    Ok(Json(announcements))
}

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    user: Option<Extension<CurrentUser>>,
) -> Result<Json<Announcement>> {
    let announcement = state.service_context.announcement_repo
        .find_by_id(id)
        .await?
        .ok_or(AppError::NotFound("Announcement not found".to_string()))?;
    
    // Check visibility
    if !announcement.is_public && user.is_none() {
        return Err(AppError::Forbidden);
    }
    
    // Check if published (or if user is the creator)
    if announcement.published_at.is_none() {
        if let Some(user) = user {
            if announcement.created_by != user.member.id {
                return Err(AppError::NotFound("Announcement not found".to_string()));
            }
        } else {
            return Err(AppError::NotFound("Announcement not found".to_string()));
        }
    }
    
    Ok(Json(announcement))
}

pub async fn create(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<CreateAnnouncementRequest>,
) -> Result<(StatusCode, Json<Announcement>)> {
    let announcement = Announcement {
        id: Uuid::new_v4(),
        title: request.title,
        content: request.content,
        announcement_type: request.announcement_type,
        announcement_type_id: None,
        is_public: request.is_public,
        featured: request.featured,
        image_url: None,
        published_at: if request.publish_now {
            Some(Utc::now())
        } else {
            None
        },
        created_by: user.member.id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    
    let created_announcement = state.service_context.announcement_repo.create(announcement).await?;
    
    Ok((StatusCode::CREATED, Json(created_announcement)))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<UpdateAnnouncementRequest>,
) -> Result<Json<Announcement>> {
    // Get the existing announcement
    let mut announcement = state.service_context.announcement_repo
        .find_by_id(id)
        .await?
        .ok_or(AppError::NotFound("Announcement not found".to_string()))?;
    
    // Check if user can update (must be creator or admin)
    if announcement.created_by != user.member.id {
        // TODO: Add admin check here
        return Err(AppError::Forbidden);
    }
    
    // Apply updates
    if let Some(title) = request.title {
        announcement.title = title;
    }
    if let Some(content) = request.content {
        announcement.content = content;
    }
    if let Some(announcement_type) = request.announcement_type {
        announcement.announcement_type = announcement_type;
    }
    if let Some(is_public) = request.is_public {
        announcement.is_public = is_public;
    }
    if let Some(featured) = request.featured {
        announcement.featured = featured;
    }
    if let Some(published_at) = request.published_at {
        announcement.published_at = published_at;
    }
    
    announcement.updated_at = Utc::now();
    
    let updated_announcement = state.service_context.announcement_repo.update(id, announcement).await?;
    
    Ok(Json(updated_announcement))
}

pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Extension(user): Extension<CurrentUser>,
) -> Result<StatusCode> {
    // Get the announcement to check permissions
    let announcement = state.service_context.announcement_repo
        .find_by_id(id)
        .await?
        .ok_or(AppError::NotFound("Announcement not found".to_string()))?;
    
    // Check if user can delete (must be creator or admin)
    if announcement.created_by != user.member.id {
        // TODO: Add admin check here
        return Err(AppError::Forbidden);
    }
    
    state.service_context.announcement_repo.delete(id).await?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct PrivateAnnouncementCount {
    pub count: i64,
}

/// Returns the count of members-only published announcements.
/// This is a public endpoint to let visitors know there's exclusive content.
pub async fn private_count(
    State(state): State<AppState>,
) -> Result<Json<PrivateAnnouncementCount>> {
    let count = state.service_context.announcement_repo
        .count_private_published()
        .await?;

    Ok(Json(PrivateAnnouncementCount { count }))
}