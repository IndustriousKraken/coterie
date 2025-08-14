use async_trait::async_trait;
use crate::{
    config::UnifiConfig,
    error::{AppError, Result},
    integrations::{Integration, IntegrationEvent, BaseIntegration},
};

pub struct UnifiIntegration {
    base: BaseIntegration,
    config: UnifiConfig,
    // In real implementation, would have HTTP client configured for Unifi
}

impl UnifiIntegration {
    pub fn new(config: Option<UnifiConfig>) -> Option<Self> {
        config.and_then(|cfg| {
            if cfg.enabled {
                Some(Self {
                    base: BaseIntegration::new("Unifi", cfg.enabled),
                    config: cfg,
                })
            } else {
                None
            }
        })
    }

    async fn grant_access(&self, member_email: &str) -> Result<()> {
        // Implementation would:
        // 1. Create user in Unifi Access if not exists
        // 2. Assign access groups
        // 3. Sync to door controllers
        tracing::info!("Would grant Unifi access to: {}", member_email);
        Ok(())
    }

    async fn revoke_access(&self, member_email: &str) -> Result<()> {
        // Implementation would:
        // 1. Find user in Unifi system
        // 2. Remove from access groups
        // 3. Optionally delete user
        tracing::info!("Would revoke Unifi access from: {}", member_email);
        Ok(())
    }

    async fn update_access(&self, member_email: &str, active: bool) -> Result<()> {
        if active {
            self.grant_access(member_email).await
        } else {
            self.revoke_access(member_email).await
        }
    }
}

#[async_trait]
impl Integration for UnifiIntegration {
    fn name(&self) -> &str {
        &self.base.name
    }

    fn is_enabled(&self) -> bool {
        self.base.enabled
    }

    async fn health_check(&self) -> Result<()> {
        // In real implementation, would check Unifi API connectivity
        if self.config.controller_url.is_empty() {
            return Err(AppError::Integration("Unifi controller URL not configured".to_string()));
        }
        Ok(())
    }

    async fn handle_event(&self, event: &IntegrationEvent) -> Result<()> {
        match event {
            IntegrationEvent::MemberActivated(member) => {
                self.grant_access(&member.email).await?;
            }
            IntegrationEvent::MemberExpired(member) | IntegrationEvent::MemberDeleted(member) => {
                self.revoke_access(&member.email).await?;
            }
            IntegrationEvent::MemberUpdated { old: _, new } => {
                // Update access based on new status
                let should_have_access = matches!(
                    new.status,
                    crate::domain::MemberStatus::Active | crate::domain::MemberStatus::Honorary
                );
                self.update_access(&new.email, should_have_access).await?;
            }
            _ => {}
        }
        Ok(())
    }
}