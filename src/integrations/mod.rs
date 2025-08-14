use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;
use crate::domain::Member;
use crate::error::Result;

pub mod discord;
pub mod unifi;

#[derive(Debug, Clone)]
pub enum IntegrationEvent {
    MemberCreated(Member),
    MemberActivated(Member),
    MemberExpired(Member),
    MemberUpdated { old: Member, new: Member },
    MemberDeleted(Member),
}

#[async_trait]
pub trait Integration: Send + Sync {
    fn name(&self) -> &str;
    fn is_enabled(&self) -> bool;
    async fn health_check(&self) -> Result<()>;
    async fn handle_event(&self, event: &IntegrationEvent) -> Result<()>;
}

pub struct IntegrationManager {
    integrations: RwLock<Vec<Arc<dyn Integration>>>,
}

impl IntegrationManager {
    pub fn new() -> Self {
        Self {
            integrations: RwLock::new(Vec::new()),
        }
    }

    pub async fn register(&self, integration: Arc<dyn Integration>) {
        if integration.is_enabled() {
            let mut integrations = self.integrations.write().await;
            integrations.push(integration);
            tracing::info!("Registered integration: {}", integrations.last().unwrap().name());
        }
    }

    pub async fn handle_event(&self, event: IntegrationEvent) {
        let integrations = self.integrations.read().await;
        
        for integration in integrations.iter() {
            if !integration.is_enabled() {
                continue;
            }

            match integration.handle_event(&event).await {
                Ok(_) => {
                    tracing::debug!(
                        "Integration {} handled event successfully",
                        integration.name()
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "Integration {} failed to handle event: {:?}",
                        integration.name(),
                        e
                    );
                    // Continue processing other integrations even if one fails
                }
            }
        }
    }

    pub async fn health_check_all(&self) -> Vec<(String, Result<()>)> {
        let integrations = self.integrations.read().await;
        let mut results = Vec::new();

        for integration in integrations.iter() {
            let name = integration.name().to_string();
            let result = integration.health_check().await;
            results.push((name, result));
        }

        results
    }
}

// Base implementation for common integration functionality
pub struct BaseIntegration {
    pub name: String,
    pub enabled: bool,
}

impl BaseIntegration {
    pub fn new(name: impl Into<String>, enabled: bool) -> Self {
        Self {
            name: name.into(),
            enabled,
        }
    }
}