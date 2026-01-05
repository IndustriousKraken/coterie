use std::sync::Arc;
use uuid::Uuid;

use crate::{
    domain::{CreateEventTypeRequest, EventTypeConfig, UpdateEventTypeRequest, validate_hex_color},
    error::{AppError, Result},
    repository::EventTypeRepository,
};

pub struct EventTypeService {
    repo: Arc<dyn EventTypeRepository>,
}

impl EventTypeService {
    pub fn new(repo: Arc<dyn EventTypeRepository>) -> Self {
        Self { repo }
    }

    /// List all event types
    pub async fn list(&self, include_inactive: bool) -> Result<Vec<EventTypeConfig>> {
        self.repo.list(include_inactive).await
    }

    /// Get an event type by ID
    pub async fn get(&self, id: Uuid) -> Result<Option<EventTypeConfig>> {
        self.repo.find_by_id(id).await
    }

    /// Get an event type by slug
    pub async fn get_by_slug(&self, slug: &str) -> Result<Option<EventTypeConfig>> {
        self.repo.find_by_slug(slug).await
    }

    /// Create a new event type
    pub async fn create(&self, request: CreateEventTypeRequest) -> Result<EventTypeConfig> {
        // Validate color format if provided
        if let Some(ref color) = request.color {
            if !validate_hex_color(color) {
                return Err(AppError::BadRequest(format!(
                    "Invalid color format: {}. Expected hex color like #FF0000",
                    color
                )));
            }
        }

        // Check for duplicate slug if provided
        if let Some(ref slug) = request.slug {
            if self.repo.find_by_slug(slug).await?.is_some() {
                return Err(AppError::Conflict(format!(
                    "Event type with slug '{}' already exists",
                    slug
                )));
            }
        }

        self.repo.create(request).await
    }

    /// Update an existing event type
    pub async fn update(&self, id: Uuid, request: UpdateEventTypeRequest) -> Result<EventTypeConfig> {
        // Validate color format if provided
        if let Some(ref color) = request.color {
            if !validate_hex_color(color) {
                return Err(AppError::BadRequest(format!(
                    "Invalid color format: {}. Expected hex color like #FF0000",
                    color
                )));
            }
        }

        self.repo.update(id, request).await
    }

    /// Delete an event type
    pub async fn delete(&self, id: Uuid) -> Result<()> {
        let event_type = self.repo.find_by_id(id).await?.ok_or_else(|| {
            AppError::NotFound("Event type not found".to_string())
        })?;

        // Cannot delete system types
        if event_type.is_system {
            return Err(AppError::BadRequest(
                "Cannot delete system event types. Deactivate instead.".to_string()
            ));
        }

        // Cannot delete if in use
        let usage_count = self.repo.count_usage(id).await?;
        if usage_count > 0 {
            return Err(AppError::Conflict(format!(
                "Cannot delete event type: {} events still use this type. Deactivate instead.",
                usage_count
            )));
        }

        self.repo.delete(id).await
    }

    /// Reorder event types
    pub async fn reorder(&self, ids: &[Uuid]) -> Result<()> {
        self.repo.reorder(ids).await
    }

    /// Seed default event types
    pub async fn seed_defaults(&self) -> Result<Vec<EventTypeConfig>> {
        self.repo.seed_defaults().await
    }
}
