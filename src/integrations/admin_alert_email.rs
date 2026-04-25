//! Email backup for AdminAlert events. Sends to `org.contact_email`
//! whenever an admin-alert fires, in addition to whatever the Discord
//! integration does. Rationale: the most common reason we'd dispatch
//! an AdminAlert (Stripe webhook signature failure, billing job
//! crash, etc.) is also the kind of incident where Discord might be
//! unreachable — we want operators to find out by email even if
//! Discord is down or unconfigured.
//!
//! This integration is a no-op when:
//!   - `org.contact_email` is empty (operator hasn't set one)
//!   - the event isn't an AdminAlert
//!
//! All failures are logged but never bubble up — like every other
//! integration handler, we don't want a notification path to fail an
//! upstream admin action.

use async_trait::async_trait;
use std::sync::Arc;

use crate::{
    email::{self, EmailSender, templates::{AdminAlertHtml, AdminAlertText}},
    error::Result,
    integrations::{Integration, IntegrationEvent},
    service::settings_service::SettingsService,
};

pub struct AdminAlertEmailIntegration {
    settings: Arc<SettingsService>,
    sender: Arc<dyn EmailSender>,
}

impl AdminAlertEmailIntegration {
    pub fn new(settings: Arc<SettingsService>, sender: Arc<dyn EmailSender>) -> Self {
        Self { settings, sender }
    }
}

#[async_trait]
impl Integration for AdminAlertEmailIntegration {
    fn name(&self) -> &str {
        "AdminAlertEmail"
    }

    fn is_enabled(&self) -> bool {
        // Always registered. We re-check `org.contact_email` per event,
        // since admins can configure it after startup.
        true
    }

    async fn health_check(&self) -> Result<()> {
        // Nothing to probe — the email sender's own configuration is
        // verified by the email-settings test button.
        Ok(())
    }

    async fn handle_event(&self, event: &IntegrationEvent) -> Result<()> {
        let IntegrationEvent::AdminAlert { subject, body } = event else {
            return Ok(());
        };

        let to = self.settings.get_value("org.contact_email").await
            .ok().filter(|s| !s.is_empty());
        let Some(to) = to else {
            tracing::debug!("AdminAlertEmail skipped: org.contact_email not set");
            return Ok(());
        };

        let org_name = self.settings.get_value("org.name").await
            .ok().filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Coterie".to_string());

        let html = AdminAlertHtml { org_name: &org_name, subject, body };
        let text = AdminAlertText { org_name: &org_name, subject, body };
        let message = match email::message_from_templates(
            to.clone(),
            format!("[{}] Admin alert: {}", org_name, subject),
            &html,
            &text,
        ) {
            Ok(m) => m,
            Err(e) => {
                tracing::error!("AdminAlertEmail template render failed: {}", e);
                return Ok(());
            }
        };

        if let Err(e) = self.sender.send(&message).await {
            tracing::error!("AdminAlertEmail send to {} failed: {}", to, e);
        }
        Ok(())
    }
}
