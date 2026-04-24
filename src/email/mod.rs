//! Email sending infrastructure.
//!
//! At runtime the app uses [`DynamicSender`], which reads its config
//! from the DB on every send and constructs a concrete sender
//! ([`LogSender`] or [`SmtpSender`]) on the fly. This lets admins edit
//! SMTP settings from the UI without restarting the server.

use async_trait::async_trait;

use crate::error::{AppError, Result};

pub mod dynamic_sender;
pub mod log_sender;
pub mod smtp_sender;
pub mod templates;

pub use dynamic_sender::DynamicSender;
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
