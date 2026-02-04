use stripe::{
    Client, CheckoutSession, CheckoutSessionMode, CreateCheckoutSession,
    CreateCheckoutSessionLineItems, Currency, EventObject, EventType,
    Webhook, WebhookError,
};
use chrono::{Months, Utc};
use uuid::Uuid;
use std::sync::Arc;
use sqlx::SqlitePool;

use crate::{
    domain::{Payment, PaymentMethod, PaymentStatus, configurable_types::BillingPeriod},
    error::{AppError, Result},
    repository::PaymentRepository,
    service::membership_type_service::MembershipTypeService,
};

pub struct StripeClient {
    client: Client,
    webhook_secret: String,
    payment_repo: Arc<dyn PaymentRepository>,
    membership_type_service: Arc<MembershipTypeService>,
    db_pool: SqlitePool,
}

impl StripeClient {
    pub fn new(
        api_key: String,
        webhook_secret: String,
        payment_repo: Arc<dyn PaymentRepository>,
        membership_type_service: Arc<MembershipTypeService>,
        db_pool: SqlitePool,
    ) -> Self {
        let client = Client::new(api_key);
        Self {
            client,
            webhook_secret,
            payment_repo,
            membership_type_service,
            db_pool,
        }
    }

    pub async fn create_membership_checkout_session(
        &self,
        member_id: Uuid,
        membership_type_name: &str,
        membership_type_slug: &str,
        amount_cents: i64,
        success_url: String,
        cancel_url: String,
    ) -> Result<(String, Uuid)> {
        // Create checkout session with inline price data
        let mut params = CreateCheckoutSession::new();
        params.mode = Some(CheckoutSessionMode::Payment);
        params.success_url = Some(&success_url);
        params.cancel_url = Some(&cancel_url);

        // Create line items with inline price data
        params.line_items = Some(vec![CreateCheckoutSessionLineItems {
            price_data: Some(stripe::CreateCheckoutSessionLineItemsPriceData {
                currency: Currency::USD,
                unit_amount: Some(amount_cents),
                product_data: Some(stripe::CreateCheckoutSessionLineItemsPriceDataProductData {
                    name: format!("{} Membership", membership_type_name),
                    description: Some(format!("{} membership dues", membership_type_name)),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            quantity: Some(1),
            ..Default::default()
        }]);

        // Add metadata for tracking (store slug for dues extension lookup)
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("member_id".to_string(), member_id.to_string());
        metadata.insert("membership_type".to_string(), membership_type_name.to_string());
        metadata.insert("membership_type_slug".to_string(), membership_type_slug.to_string());
        params.metadata = Some(metadata);
        let member_id_str = member_id.to_string();
        params.client_reference_id = Some(&member_id_str);

        let session = CheckoutSession::create(&self.client, params)
            .await
            .map_err(|e| AppError::External(format!("Stripe error: {}", e)))?;

        // Create pending payment record
        let payment_id = Uuid::new_v4();
        let payment = Payment {
            id: payment_id,
            member_id,
            amount_cents,
            currency: "USD".to_string(),
            status: PaymentStatus::Pending,
            payment_method: PaymentMethod::Stripe,
            stripe_payment_id: Some(session.id.to_string()),
            description: format!("{} Membership Payment", membership_type_name),
            paid_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.payment_repo.create(payment).await?;

        // Return the checkout URL and payment ID
        let url = session.url
            .ok_or_else(|| AppError::External("No checkout URL returned".to_string()))?;
        Ok((url, payment_id))
    }

    pub async fn handle_webhook(
        &self,
        payload: &str,
        stripe_signature: &str,
    ) -> Result<()> {
        // Verify webhook signature and construct event
        let event = Webhook::construct_event(
            payload,
            stripe_signature,
            &self.webhook_secret,
        )
        .map_err(|e| match e {
            WebhookError::BadSignature => AppError::BadRequest("Invalid signature".to_string()),
            _ => AppError::External(format!("Webhook error: {}", e)),
        })?;

        // Handle different event types
        match event.type_ {
            EventType::CheckoutSessionCompleted => {
                if let EventObject::CheckoutSession(session) = event.data.object {
                    self.handle_successful_payment(session).await?;
                }
            }
            EventType::CheckoutSessionExpired => {
                if let EventObject::CheckoutSession(session) = event.data.object {
                    self.handle_expired_session(session).await?;
                }
            }
            EventType::PaymentIntentPaymentFailed => {
                if let EventObject::PaymentIntent(intent) = event.data.object {
                    self.handle_failed_payment(intent.id.to_string()).await?;
                }
            }
            _ => {
                tracing::debug!("Unhandled webhook event type: {:?}", event.type_);
            }
        }

        Ok(())
    }

    async fn handle_successful_payment(
        &self,
        session: CheckoutSession,
    ) -> Result<()> {
        let session_id = session.id.to_string();

        // Find the payment by Stripe ID
        if let Some(mut payment) = self.payment_repo.find_by_stripe_id(&session_id).await? {
            // Update payment status
            payment.status = PaymentStatus::Completed;
            payment.paid_at = Some(Utc::now());
            payment.updated_at = Utc::now();

            self.payment_repo.update(payment.id, payment.clone()).await?;

            // Extend member's dues_paid_until based on membership type billing period
            let membership_type_slug = session.metadata
                .as_ref()
                .and_then(|m| m.get("membership_type_slug"))
                .cloned();

            if let Some(slug) = membership_type_slug {
                self.extend_member_dues(payment.member_id, &slug).await?;
            } else {
                tracing::warn!("No membership_type_slug in session metadata for session: {}", session_id);
            }

            tracing::info!("Payment completed for member: {}", payment.member_id);
        } else {
            tracing::warn!("Payment not found for Stripe session: {}", session_id);
        }

        Ok(())
    }

    async fn extend_member_dues(
        &self,
        member_id: Uuid,
        membership_type_slug: &str,
    ) -> Result<()> {
        let membership_type = self.membership_type_service
            .get_by_slug(membership_type_slug)
            .await?
            .ok_or_else(|| AppError::NotFound(format!(
                "Membership type '{}' not found", membership_type_slug
            )))?;

        let billing_period = membership_type.billing_period_enum()
            .unwrap_or(BillingPeriod::Yearly);

        // Get the member's current dues_paid_until
        let row = sqlx::query_scalar::<_, Option<chrono::DateTime<Utc>>>(
            "SELECT dues_paid_until FROM members WHERE id = ?"
        )
            .bind(member_id.to_string())
            .fetch_optional(&self.db_pool)
            .await
            .map_err(|e| AppError::Internal(format!("Database error: {}", e)))?;

        let current_dues = row.flatten();
        let now = Utc::now();
        let base_date = current_dues
            .filter(|d| *d > now)
            .unwrap_or(now);

        let new_dues_date = match billing_period {
            BillingPeriod::Monthly => base_date.checked_add_months(Months::new(1)).unwrap_or(base_date),
            BillingPeriod::Yearly => base_date.checked_add_months(Months::new(12)).unwrap_or(base_date),
            BillingPeriod::Lifetime => chrono::DateTime::<Utc>::MAX_UTC,
        };

        sqlx::query("UPDATE members SET dues_paid_until = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(new_dues_date)
            .bind(member_id.to_string())
            .execute(&self.db_pool)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to update dues: {}", e)))?;

        tracing::info!(
            "Extended dues for member {} to {} (billing period: {:?})",
            member_id, new_dues_date, billing_period
        );

        Ok(())
    }

    async fn handle_expired_session(
        &self,
        session: CheckoutSession,
    ) -> Result<()> {
        let session_id = session.id.to_string();
        
        if let Some(mut payment) = self.payment_repo.find_by_stripe_id(&session_id).await? {
            payment.status = PaymentStatus::Failed;
            payment.updated_at = Utc::now();
            
            self.payment_repo.update(payment.id, payment).await?;
            
            tracing::info!("Checkout session expired: {}", session_id);
        }
        
        Ok(())
    }

    async fn handle_failed_payment(&self, stripe_payment_id: String) -> Result<()> {
        if let Some(mut payment) = self.payment_repo.find_by_stripe_id(&stripe_payment_id).await? {
            payment.status = PaymentStatus::Failed;
            payment.updated_at = Utc::now();
            
            self.payment_repo.update(payment.id, payment).await?;
            
            tracing::warn!("Payment failed: {}", stripe_payment_id);
        }
        
        Ok(())
    }

}