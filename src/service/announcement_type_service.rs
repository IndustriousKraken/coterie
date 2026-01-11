use std::sync::Arc;
use uuid::Uuid;

use crate::{
    domain::{AnnouncementTypeConfig, CreateAnnouncementTypeRequest, UpdateAnnouncementTypeRequest, validate_hex_color},
    error::{AppError, Result},
    repository::AnnouncementTypeRepository,
};

pub struct AnnouncementTypeService {
    repo: Arc<dyn AnnouncementTypeRepository>,
}

impl AnnouncementTypeService {
    pub fn new(repo: Arc<dyn AnnouncementTypeRepository>) -> Self {
        Self { repo }
    }

    /// List all announcement types
    pub async fn list(&self, include_inactive: bool) -> Result<Vec<AnnouncementTypeConfig>> {
        self.repo.list(include_inactive).await
    }

    /// Get an announcement type by ID
    pub async fn get(&self, id: Uuid) -> Result<Option<AnnouncementTypeConfig>> {
        self.repo.find_by_id(id).await
    }

    /// Get an announcement type by slug
    pub async fn get_by_slug(&self, slug: &str) -> Result<Option<AnnouncementTypeConfig>> {
        self.repo.find_by_slug(slug).await
    }

    /// Create a new announcement type
    pub async fn create(&self, request: CreateAnnouncementTypeRequest) -> Result<AnnouncementTypeConfig> {
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
                    "Announcement type with slug '{}' already exists",
                    slug
                )));
            }
        }

        self.repo.create(request).await
    }

    /// Update an existing announcement type
    pub async fn update(&self, id: Uuid, request: UpdateAnnouncementTypeRequest) -> Result<AnnouncementTypeConfig> {
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

    /// Delete an announcement type
    pub async fn delete(&self, id: Uuid) -> Result<()> {
        let _announcement_type = self.repo.find_by_id(id).await?.ok_or_else(|| {
            AppError::NotFound("Announcement type not found".to_string())
        })?;

        // Cannot delete if in use
        let usage_count = self.repo.count_usage(id).await?;
        if usage_count > 0 {
            return Err(AppError::Conflict(format!(
                "Cannot delete announcement type: {} announcements still use this type. Deactivate instead.",
                usage_count
            )));
        }

        self.repo.delete(id).await
    }

    /// Reorder announcement types
    pub async fn reorder(&self, ids: &[Uuid]) -> Result<()> {
        self.repo.reorder(ids).await
    }

    /// Seed default announcement types
    pub async fn seed_defaults(&self) -> Result<Vec<AnnouncementTypeConfig>> {
        self.repo.seed_defaults().await
    }
}
