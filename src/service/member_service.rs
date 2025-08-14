use std::sync::Arc;
use uuid::Uuid;
use chrono::Utc;
use crate::{
    domain::*,
    error::{AppError, Result},
    repository::MemberRepository,
    integrations::{IntegrationEvent, IntegrationManager},
};

pub struct MemberService {
    repo: Arc<dyn MemberRepository>,
    integration_manager: Arc<IntegrationManager>,
}

impl MemberService {
    pub fn new(
        repo: Arc<dyn MemberRepository>,
        integration_manager: Arc<IntegrationManager>,
    ) -> Self {
        Self { repo, integration_manager }
    }

    pub async fn create_member(&self, request: CreateMemberRequest) -> Result<Member> {
        // Check for duplicate email
        if let Some(_) = self.repo.find_by_email(&request.email).await? {
            return Err(AppError::Conflict("Email already exists".to_string()));
        }

        // Check for duplicate username
        if let Some(_) = self.repo.find_by_username(&request.username).await? {
            return Err(AppError::Conflict("Username already exists".to_string()));
        }

        let member = self.repo.create(request).await?;
        
        // Notify integrations
        self.integration_manager
            .handle_event(IntegrationEvent::MemberCreated(member.clone()))
            .await;

        Ok(member)
    }

    pub async fn activate_member(&self, id: Uuid) -> Result<Member> {
        let member = self.repo.find_by_id(id).await?
            .ok_or_else(|| AppError::NotFound("Member not found".to_string()))?;

        if matches!(member.status, MemberStatus::Active) {
            return Ok(member);
        }

        let update = UpdateMemberRequest {
            status: Some(MemberStatus::Active),
            ..Default::default()
        };

        let updated = self.repo.update(id, update).await?;
        
        // Notify integrations
        self.integration_manager
            .handle_event(IntegrationEvent::MemberActivated(updated.clone()))
            .await;

        Ok(updated)
    }

    pub async fn expire_member(&self, id: Uuid) -> Result<Member> {
        let member = self.repo.find_by_id(id).await?
            .ok_or_else(|| AppError::NotFound("Member not found".to_string()))?;

        if member.bypass_dues {
            return Err(AppError::BadRequest("Member has dues bypass enabled".to_string()));
        }

        let update = UpdateMemberRequest {
            status: Some(MemberStatus::Expired),
            expires_at: Some(Utc::now()),
            ..Default::default()
        };

        let updated = self.repo.update(id, update).await?;
        
        // Notify integrations
        self.integration_manager
            .handle_event(IntegrationEvent::MemberExpired(updated.clone()))
            .await;

        Ok(updated)
    }

    pub async fn check_expired_members(&self) -> Result<Vec<Member>> {
        let active_members = self.repo.list_active().await?;
        let mut expired = Vec::new();

        for member in active_members {
            if member.bypass_dues {
                continue;
            }

            let should_expire = member.dues_paid_until
                .map(|date| date < Utc::now())
                .unwrap_or(false);

            if should_expire {
                if let Ok(expired_member) = self.expire_member(member.id).await {
                    expired.push(expired_member);
                }
            }
        }

        Ok(expired)
    }
}

// Default is already derived in domain/member.rs