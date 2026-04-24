//! Email sending infrastructure.
//!
//! Provides an [`EmailSender`] trait with two built-in implementations:
//! [`LogSender`] (for dev/tests — writes emails to tracing logs) and
//! [`SmtpSender`] (lettre-backed SMTP). Use [`create_sender`] to build
//! one from [`EmailConfig`].

use async_trait::async_trait;
use std::sync::Arc;

use crate::{
    config::{EmailConfig, EmailMode},
    error::{AppError, Result},
};

pub mod log_sender;
pub mod smtp_sender;
pub mod templates;

pub use log_sender::LogSender;
pub use smtp_sender::SmtpSender;

/// A single email message. Both `html_body` and `text_body` are sent as
/// a multipart/alternative so clients can choose.
#[derive(Debug, Clone)]
pub struct EmailMessage {
    pub to: String,
    pub subject: String,
    pub html_body: String,
    pub text_body: String,
}

#[async_trait]
pub trait EmailSender: Send + Sync {
    async fn send(&self, message: &EmailMessage) -> Result<()>;
}

/// Build a sender from config. Returns `LogSender` if mode is Log or if
/// SMTP mode is selected but required fields are missing (with a warning
/// logged) — we prefer falling back to Log over panicking at startup.
pub fn create_sender(config: &EmailConfig) -> Arc<dyn EmailSender> {
    match config.mode {
        EmailMode::Log => {
            tracing::info!("Email mode: log (emails will be written to stdout/logs only)");
            Arc::new(LogSender::new(
                config.from_address.clone().unwrap_or_else(|| "noreply@localhost".to_string()),
                config.from_name.clone().unwrap_or_else(|| "Coterie".to_string()),
            ))
        }
        EmailMode::Smtp => match SmtpSender::from_config(config) {
            Ok(sender) => {
                tracing::info!("Email mode: smtp (host: {:?})", config.smtp_host);
                Arc::new(sender)
            }
            Err(e) => {
                tracing::warn!(
                    "Email mode is smtp but configuration is incomplete ({}). \
                     Falling back to log mode — emails will not actually send.",
                    e
                );
                Arc::new(LogSender::new(
                    config.from_address.clone().unwrap_or_else(|| "noreply@localhost".to_string()),
                    config.from_name.clone().unwrap_or_else(|| "Coterie".to_string()),
                ))
            }
        },
    }
}

/// Convenience: build an EmailMessage from an Askama template pair
/// (HTML + plain text).
pub fn message_from_templates<H, T>(
    to: String,
    subject: String,
    html: &H,
    text: &T,
) -> Result<EmailMessage>
where
    H: askama::Template,
    T: askama::Template,
{
    let html_body = html.render().map_err(|e| {
        AppError::Internal(format!("Failed to render HTML email template: {}", e))
    })?;
    let text_body = text.render().map_err(|e| {
        AppError::Internal(format!("Failed to render text email template: {}", e))
    })?;
    Ok(EmailMessage {
        to,
        subject,
        html_body,
        text_body,
    })
}
