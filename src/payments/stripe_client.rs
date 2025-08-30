use stripe::{
    Client, CheckoutSession, CheckoutSessionMode, CreateCheckoutSession,
    CreateCheckoutSessionLineItems, Currency, EventObject, EventType, 
    Webhook, WebhookError,
};
use chrono::Utc;
use uuid::Uuid;
use std::sync::Arc;

use crate::{
    domain::{Payment, PaymentMethod, PaymentStatus},
    error::{AppError, Result},
    repository::PaymentRepository,
};

pub struct StripeClient {
    client: Client,
    webhook_secret: String,
    payment_repo: Arc<dyn PaymentRepository>,
}

impl StripeClient {
    pub fn new(
        api_key: String,
        webhook_secret: String,
        payment_repo: Arc<dyn PaymentRepository>,
    ) -> Self {
        let client = Client::new(api_key);
        Self {
            client,
            webhook_secret,
            payment_repo,
        }
    }

    pub async fn create_membership_checkout_session(
        &self,
        member_id: Uuid,
        membership_type: &str,
        amount_cents: i64,
        success_url: String,
        cancel_url: String,
    ) -> Result<String> {
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
                    name: format!("{} Membership", membership_type),
                    description: Some(format!("Annual {} membership dues", membership_type)),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            quantity: Some(1),
            ..Default::default()
        }]);
        
        // Add metadata for tracking
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("member_id".to_string(), member_id.to_string());
        metadata.insert("membership_type".to_string(), membership_type.to_string());
        params.metadata = Some(metadata);
        let member_id_str = member_id.to_string();
        params.client_reference_id = Some(&member_id_str);

        let session = CheckoutSession::create(&self.client, params)
            .await
            .map_err(|e| AppError::External(format!("Stripe error: {}", e)))?;

        // Create pending payment record
        let payment = Payment {
            id: Uuid::new_v4(),
            member_id,
            amount_cents,
            currency: "USD".to_string(),
            status: PaymentStatus::Pending,
            payment_method: PaymentMethod::Stripe,
            stripe_payment_id: Some(session.id.to_string()),
            description: format!("{} Membership Payment", membership_type),
            paid_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.payment_repo.create(payment).await?;

        // Return the checkout URL
        session.url
            .ok_or_else(|| AppError::External("No checkout URL returned".to_string()))
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
            
            // Get member ID from metadata or payment record
            tracing::info!("Payment completed for member: {}", payment.member_id);
            
            // Here you would typically update the member's dues_paid_until date
            // This would be done through a member service
        } else {
            tracing::warn!("Payment not found for Stripe session: {}", session_id);
        }
        
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

    pub async fn create_manual_payment(
        &self,
        member_id: Uuid,
        amount_cents: i64,
        description: String,
    ) -> Result<Payment> {
        let payment = Payment {
            id: Uuid::new_v4(),
            member_id,
            amount_cents,
            currency: "USD".to_string(),
            status: PaymentStatus::Completed,
            payment_method: PaymentMethod::Manual,
            stripe_payment_id: None,
            description,
            paid_at: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.payment_repo.create(payment).await
    }

    pub async fn waive_payment(
        &self,
        member_id: Uuid,
        description: String,
    ) -> Result<Payment> {
        let payment = Payment {
            id: Uuid::new_v4(),
            member_id,
            amount_cents: 0,
            currency: "USD".to_string(),
            status: PaymentStatus::Completed,
            payment_method: PaymentMethod::Waived,
            stripe_payment_id: None,
            description,
            paid_at: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.payment_repo.create(payment).await
    }
}