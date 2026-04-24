//! Sender that reads its configuration from the DB on every send.
//!
//! This is the production sender. It lets admins change email
//! settings through the UI with no server restart — the next email
//! uses the updated config. Cost is one SQL read per send (SQLite,
//! cheap). In exchange we avoid any live-reload / config-swap
//! plumbing in the rest of the app.

use async_trait::async_trait;
use std::sync::Arc;

use super::{EmailMessage, EmailSender, LogSender, SmtpSender};
use crate::{
    config::{EmailConfig, EmailMode},
    error::Result,
    service::settings_service::SettingsService,
};

pub struct DynamicSender {
    settings: Arc<SettingsService>,
}

impl DynamicSender {
    pub fn new(settings: Arc<SettingsService>) -> Self {
        Self { settings }
    }
}

#[async_trait]
impl EmailSender for DynamicSender {
    async fn send(&self, message: &EmailMessage) -> Result<()> {
        // Translate the DB config shape into the struct the existing
        // sender constructors already know.
        let db = self.settings.get_email_config().await?;
        let mode = match db.mode.as_str() {
            "smtp" => EmailMode::Smtp,
            _ => EmailMode::Log,
        };

        let cfg = EmailConfig {
            mode: mode.clone(),
            from_address: non_empty(&db.from_address),
            from_name: non_empty(&db.from_name),
            smtp_host: non_empty(&db.smtp_host),
            smtp_port: Some(db.smtp_port),
            smtp_username: non_empty(&db.smtp_username),
            smtp_password: non_empty(&db.smtp_password),
        };

        // Build the concrete sender for this one send. LogSender is
        // cheap; SmtpSender creates a transport that is also cheap
        // (no connection is opened until `.send` runs).
        let sender: Arc<dyn EmailSender> = match mode {
            EmailMode::Log => Arc::new(LogSender::new(
                db.from_address.clone(),
                db.from_name.clone(),
            )),
            EmailMode::Smtp => match SmtpSender::from_config(&cfg) {
                Ok(s) => Arc::new(s),
                Err(e) => {
                    tracing::warn!(
                        "SMTP config incomplete ({}). Falling back to log mode for this send.",
                        e
                    );
                    Arc::new(LogSender::new(
                        db.from_address.clone(),
                        db.from_name.clone(),
                    ))
                }
            },
        };

        sender.send(message).await
    }
}

fn non_empty(s: &str) -> Option<String> {
    if s.is_empty() { None } else { Some(s.to_string()) }
}
