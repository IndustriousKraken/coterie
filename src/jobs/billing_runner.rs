use std::sync::Arc;
use tokio::time::{self, Duration};

use crate::service::billing_service::BillingService;

pub struct BillingRunner {
    billing_service: Arc<BillingService>,
    interval: Duration,
}

impl BillingRunner {
    pub fn new(billing_service: Arc<BillingService>, interval_secs: u64) -> Self {
        Self {
            billing_service,
            interval: Duration::from_secs(interval_secs),
        }
    }

    /// Spawn the billing runner as a background task.
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            tracing::info!(
                "Billing runner started (interval: {}s)",
                self.interval.as_secs()
            );

            loop {
                time::sleep(self.interval).await;
                self.run_cycle().await;
            }
        })
    }

    async fn run_cycle(&self) {
        // Process due scheduled payments
        match self.billing_service.run_billing_cycle().await {
            Ok((succeeded, total)) => {
                if total > 0 {
                    tracing::info!(
                        "Billing cycle: {}/{} payments processed",
                        succeeded,
                        total
                    );
                }
            }
            Err(e) => {
                tracing::error!("Billing cycle error: {}", e);
            }
        }

        // Check for expired members
        match self.billing_service.check_expired_members().await {
            Ok(count) => {
                if count > 0 {
                    tracing::info!("Expired {} members past grace period", count);
                }
            }
            Err(e) => {
                tracing::error!("Member expiration check error: {}", e);
            }
        }

        // Send dues-expiring-soon reminders (idempotent per cycle via
        // dues_reminder_sent_at flag, so running hourly is fine — only
        // newly-eligible members get email).
        match self.billing_service.send_dues_reminders().await {
            Ok(_) => {}
            Err(e) => {
                tracing::error!("Dues reminder cycle error: {}", e);
            }
        }
    }
}
