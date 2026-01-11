use std::sync::Arc;
use uuid::Uuid;

use crate::{
    domain::{
        BillingPeriod, CreateMembershipTypeRequest, MembershipTypeConfig,
        UpdateMembershipTypeRequest, validate_hex_color,
    },
    error::{AppError, Result},
    repository::MembershipTypeRepository,
};

pub struct MembershipTypeService {
    repo: Arc<dyn MembershipTypeRepository>,
}

impl MembershipTypeService {
    pub fn new(repo: Arc<dyn MembershipTypeRepository>) -> Self {
        Self { repo }
    }

    /// List all membership types
    pub async fn list(&self, include_inactive: bool) -> Result<Vec<MembershipTypeConfig>> {
        self.repo.list(include_inactive).await
    }

    /// Get a membership type by ID
    pub async fn get(&self, id: Uuid) -> Result<Option<MembershipTypeConfig>> {
        self.repo.find_by_id(id).await
    }

    /// Get a membership type by slug
    pub async fn get_by_slug(&self, slug: &str) -> Result<Option<MembershipTypeConfig>> {
        self.repo.find_by_slug(slug).await
    }

    /// Create a new membership type
    pub async fn create(&self, request: CreateMembershipTypeRequest) -> Result<MembershipTypeConfig> {
        // Validate color format if provided
        if let Some(ref color) = request.color {
            if !validate_hex_color(color) {
                return Err(AppError::BadRequest(format!(
                    "Invalid color format: {}. Expected hex color like #FF0000",
                    color
                )));
            }
        }

        // Validate billing period
        if BillingPeriod::from_str(&request.billing_period).is_none() {
            return Err(AppError::BadRequest(format!(
                "Invalid billing period: {}. Expected one of: monthly, yearly, lifetime",
                request.billing_period
            )));
        }

        // Validate fee is non-negative
        if request.fee_cents < 0 {
            return Err(AppError::BadRequest(
                "Fee cannot be negative".to_string()
            ));
        }

        // Check for duplicate slug if provided
        if let Some(ref slug) = request.slug {
            if self.repo.find_by_slug(slug).await?.is_some() {
                return Err(AppError::Conflict(format!(
                    "Membership type with slug '{}' already exists",
                    slug
                )));
            }
        }

        self.repo.create(request).await
    }

    /// Update an existing membership type
    pub async fn update(&self, id: Uuid, request: UpdateMembershipTypeRequest) -> Result<MembershipTypeConfig> {
        // Validate color format if provided
        if let Some(ref color) = request.color {
            if !validate_hex_color(color) {
                return Err(AppError::BadRequest(format!(
                    "Invalid color format: {}. Expected hex color like #FF0000",
                    color
                )));
            }
        }

        // Validate billing period if provided
        if let Some(ref billing_period) = request.billing_period {
            if BillingPeriod::from_str(billing_period).is_none() {
                return Err(AppError::BadRequest(format!(
                    "Invalid billing period: {}. Expected one of: monthly, yearly, lifetime",
                    billing_period
                )));
            }
        }

        // Validate fee is non-negative if provided
        if let Some(fee_cents) = request.fee_cents {
            if fee_cents < 0 {
                return Err(AppError::BadRequest(
                    "Fee cannot be negative".to_string()
                ));
            }
        }

        self.repo.update(id, request).await
    }

    /// Delete a membership type
    pub async fn delete(&self, id: Uuid) -> Result<()> {
        let _membership_type = self.repo.find_by_id(id).await?.ok_or_else(|| {
            AppError::NotFound("Membership type not found".to_string())
        })?;

        // Cannot delete if in use
        let usage_count = self.repo.count_usage(id).await?;
        if usage_count > 0 {
            return Err(AppError::Conflict(format!(
                "Cannot delete membership type: {} members still use this type. Deactivate instead.",
                usage_count
            )));
        }

        self.repo.delete(id).await
    }

    /// Reorder membership types
    pub async fn reorder(&self, ids: &[Uuid]) -> Result<()> {
        self.repo.reorder(ids).await
    }

    /// Seed default membership types
    pub async fn seed_defaults(&self) -> Result<Vec<MembershipTypeConfig>> {
        self.repo.seed_defaults().await
    }

    /// Get pricing info for a membership type
    pub async fn get_pricing(&self, id: Uuid) -> Result<MembershipPricing> {
        let membership_type = self.repo.find_by_id(id).await?.ok_or_else(|| {
            AppError::NotFound("Membership type not found".to_string())
        })?;

        let fee_dollars = membership_type.fee_dollars();
        Ok(MembershipPricing {
            type_id: membership_type.id,
            type_name: membership_type.name,
            type_slug: membership_type.slug,
            fee_cents: membership_type.fee_cents,
            fee_dollars,
            billing_period: membership_type.billing_period,
        })
    }

    /// Get pricing for all active membership types
    pub async fn get_all_pricing(&self) -> Result<Vec<MembershipPricing>> {
        let types = self.repo.list(false).await?;
        Ok(types
            .into_iter()
            .map(|t| {
                let fee_dollars = t.fee_dollars();
                MembershipPricing {
                    type_id: t.id,
                    type_name: t.name,
                    type_slug: t.slug,
                    fee_cents: t.fee_cents,
                    fee_dollars,
                    billing_period: t.billing_period,
                }
            })
            .collect())
    }
}

/// Pricing information for a membership type
#[derive(Debug, Clone, serde::Serialize)]
pub struct MembershipPricing {
    pub type_id: Uuid,
    pub type_name: String,
    pub type_slug: String,
    pub fee_cents: i32,
    pub fee_dollars: f64,
    pub billing_period: String,
}
