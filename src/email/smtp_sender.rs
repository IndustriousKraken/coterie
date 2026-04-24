//! SMTP email sender backed by lettre. Uses STARTTLS by default.

use async_trait::async_trait;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{MultiPart, SinglePart, header},
    transport::smtp::authentication::Credentials,
};

use super::{EmailMessage, EmailSender};
use crate::{
    config::EmailConfig,
    error::{AppError, Result},
};

pub struct SmtpSender {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from_address: String,
    from_name: String,
}

impl SmtpSender {
    pub fn from_config(config: &EmailConfig) -> Result<Self> {
        let host = config.smtp_host.as_ref().ok_or_else(|| {
            AppError::Internal("email.smtp_host not configured".to_string())
        })?;
        let from_address = config.from_address.clone().ok_or_else(|| {
            AppError::Internal("email.from_address not configured".to_string())
        })?;
        let from_name = config.from_name.clone().unwrap_or_else(|| "Coterie".to_string());

        let mut builder = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
            .map_err(|e| AppError::Internal(format!("SMTP init failed: {}", e)))?;

        if let Some(port) = config.smtp_port {
            builder = builder.port(port);
        }

        if let (Some(user), Some(pass)) = (&config.smtp_username, &config.smtp_password) {
            builder = builder.credentials(Credentials::new(user.clone(), pass.clone()));
        }

        Ok(Self {
            transport: builder.build(),
            from_address,
            from_name,
        })
    }
}

#[async_trait]
impl EmailSender for SmtpSender {
    async fn send(&self, message: &EmailMessage) -> Result<()> {
        let from: lettre::message::Mailbox = format!("{} <{}>", self.from_name, self.from_address)
            .parse()
            .map_err(|e| AppError::Internal(format!("Invalid From address: {}", e)))?;

        let to: lettre::message::Mailbox = message
            .to
            .parse()
            .map_err(|e| AppError::Validation(format!("Invalid recipient address: {}", e)))?;

        let email = Message::builder()
            .from(from)
            .to(to)
            .subject(&message.subject)
            .multipart(
                MultiPart::alternative()
                    .singlepart(
                        SinglePart::builder()
                            .header(header::ContentType::TEXT_PLAIN)
                            .body(message.text_body.clone()),
                    )
                    .singlepart(
                        SinglePart::builder()
                            .header(header::ContentType::TEXT_HTML)
                            .body(message.html_body.clone()),
                    ),
            )
            .map_err(|e| AppError::Internal(format!("Failed to build email: {}", e)))?;

        self.transport
            .send(email)
            .await
            .map_err(|e| AppError::External(format!("SMTP send failed: {}", e)))?;

        Ok(())
    }
}
