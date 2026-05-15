//! Unified service for event-type and announcement-type CRUD. One concrete
//! type, two instances (one per kind) wired up in `ServiceContext::new`.
//! Call sites continue to use `state.service_context.event_type_service` /
//! `announcement_type_service` exactly as before — the kind is invisible to
//! the caller because it's baked into the service instance.

use std::sync::Arc;
use uuid::Uuid;

use crate::{
    domain::{BasicType, BasicTypeKind, CreateBasicTypeRequest, UpdateBasicTypeRequest},
    error::{AppError, Result},
    repository::BasicTypeRepository,
    service::configurable_types::{
        check_delete_unused_for_basic, check_unique_slug_for_basic,
        validate_hex_color_for_request,
    },
};

pub struct BasicTypeService {
    repo: Arc<dyn BasicTypeRepository>,
    kind: BasicTypeKind,
}

impl BasicTypeService {
    pub fn new(repo: Arc<dyn BasicTypeRepository>, kind: BasicTypeKind) -> Self {
        Self { repo, kind }
    }

    pub fn kind(&self) -> BasicTypeKind {
        self.kind
    }

    pub async fn list(&self, include_inactive: bool) -> Result<Vec<BasicType>> {
        self.repo.list(self.kind, include_inactive).await
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<BasicType>> {
        self.repo.find_by_id(self.kind, id).await
    }

    pub async fn get_by_slug(&self, slug: &str) -> Result<Option<BasicType>> {
        self.repo.find_by_slug(self.kind, slug).await
    }

    pub async fn create(&self, request: CreateBasicTypeRequest) -> Result<BasicType> {
        validate_hex_color_for_request(request.color.as_deref())?;

        if let Some(ref slug) = request.slug {
            check_unique_slug_for_basic(
                self.repo.as_ref(),
                self.kind,
                slug,
                self.kind.display_name(),
            )
            .await?;
        }

        self.repo.create(self.kind, request).await
    }

    pub async fn update(
        &self,
        id: Uuid,
        request: UpdateBasicTypeRequest,
    ) -> Result<BasicType> {
        validate_hex_color_for_request(request.color.as_deref())?;
        self.repo.update(self.kind, id, request).await
    }

    pub async fn delete(&self, id: Uuid) -> Result<()> {
        let _existing = self
            .repo
            .find_by_id(self.kind, id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "{} not found",
                    capitalize_first(self.kind.display_name())
                ))
            })?;

        check_delete_unused_for_basic(self.repo.as_ref(), self.kind, id).await?;

        self.repo.delete(self.kind, id).await
    }
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}
