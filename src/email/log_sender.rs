//! Dev/test email sender: writes the message to tracing logs instead of
//! actually sending. Useful for local development and for CI.

use async_trait::async_trait;

use super::{EmailMessage, EmailSender};
use crate::error::Result;

pub struct LogSender {
    pub from_address: String,
    pub from_name: String,
}

impl LogSender {
    pub fn new(from_address: String, from_name: String) -> Self {
        Self { from_address, from_name }
    }
}

#[async_trait]
impl EmailSender for LogSender {
    async fn send(&self, message: &EmailMessage) -> Result<()> {
        tracing::info!(
            "=== Email (log mode) ===\n\
             From: {} <{}>\n\
             To: {}\n\
             Subject: {}\n\
             ---- Text body ----\n{}\n\
             ========================",
            self.from_name, self.from_address,
            message.to,
            message.subject,
            message.text_body,
        );
        Ok(())
    }
}
