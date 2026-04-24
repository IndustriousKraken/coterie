use stripe::{
    Client, CheckoutSession, CheckoutSessionMode, CreateCheckoutSession,
    CreateCheckoutSessionLineItems, Currency, EventObject, EventType,
    Webhook, WebhookError, Customer, CreateCustomer, SetupIntent, CreateSetupIntent,
    PaymentIntent, CreatePaymentIntent, PaymentIntentConfirmationMethod,
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

        // Idempotency: claim the event ID atomically. If another worker
        // or a retry already processed this event, the INSERT affects 0
        // rows and we bail early. Without this, Stripe's "at-least-once"
        // delivery would let retries double-extend dues.
        let event_id = event.id.to_string();
        let event_type = format!("{:?}", event.type_);
        let claim = sqlx::query(
            "INSERT OR IGNORE INTO processed_stripe_events (event_id, event_type) VALUES (?, ?)"
        )
        .bind(&event_id)
        .bind(&event_type)
        .execute(&self.db_pool)
        .await
        .map_err(|e| AppError::Internal(format!("Idempotency claim failed: {}", e)))?;

        if claim.rows_affected() == 0 {
            tracing::info!("Skipping already-processed Stripe event {}", event_id);
            return Ok(());
        }

        // Handle different event types
        match event.type_ {
            // One-time checkout payments
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

            // Legacy Stripe subscription events
            EventType::InvoicePaid => {
                if let EventObject::Invoice(invoice) = event.data.object {
                    self.handle_invoice_paid(invoice).await?;
                }
            }
            EventType::InvoicePaymentFailed => {
                if let EventObject::Invoice(invoice) = event.data.object {
                    self.handle_invoice_payment_failed(invoice).await?;
                }
            }
            EventType::CustomerSubscriptionDeleted => {
                if let EventObject::Subscription(subscription) = event.data.object {
                    self.handle_subscription_deleted(subscription).await?;
                }
            }
            EventType::CustomerSubscriptionUpdated => {
                if let EventObject::Subscription(subscription) = event.data.object {
                    self.handle_subscription_updated(subscription).await?;
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

    pub async fn extend_member_dues(
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

        // Also restore Expired -> Active so the member regains access after
        // paying, and clear the dues_reminder_sent_at flag so the next
        // dues cycle can trigger a fresh reminder. We only touch the
        // status when it's currently Expired — we don't want to
        // overwrite Suspended (admin-initiated) or Honorary.
        sqlx::query(
            "UPDATE members \
             SET dues_paid_until = ?, \
                 status = CASE WHEN status = 'Expired' THEN 'Active' ELSE status END, \
                 dues_reminder_sent_at = NULL, \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?"
        )
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

    // ================================================================
    // Legacy Stripe Subscription Handlers
    // ================================================================

    /// Handle invoice.paid - extend member dues for subscription payments
    async fn handle_invoice_paid(&self, invoice: stripe::Invoice) -> Result<()> {
        // Only care about subscription invoices
        let subscription_id = match &invoice.subscription {
            Some(sub) => sub.id().to_string(),
            None => return Ok(()),
        };

        let customer_id = match &invoice.customer {
            Some(customer) => customer.id().to_string(),
            None => {
                tracing::warn!("Invoice {} has no customer", invoice.id);
                return Ok(());
            }
        };

        // Find member by stripe_customer_id
        let member_id: Option<String> = sqlx::query_scalar(
            "SELECT id FROM members WHERE stripe_customer_id = ?"
        )
        .bind(&customer_id)
        .fetch_optional(&self.db_pool)
        .await
        .map_err(|e| AppError::Internal(format!("Database error: {}", e)))?;

        let member_id = match member_id {
            Some(id) => id,
            None => {
                tracing::debug!("No member found for Stripe customer {}", customer_id);
                return Ok(());
            }
        };

        let member_uuid = Uuid::parse_str(&member_id)
            .map_err(|e| AppError::Internal(e.to_string()))?;

        let amount_cents = invoice.amount_paid.unwrap_or(0);

        // Create payment record
        let payment = Payment {
            id: Uuid::new_v4(),
            member_id: member_uuid,
            amount_cents,
            currency: invoice.currency.map(|c| c.to_string()).unwrap_or_else(|| "usd".to_string()),
            status: PaymentStatus::Completed,
            payment_method: PaymentMethod::Stripe,
            stripe_payment_id: Some(invoice.id.to_string()),
            description: format!("Subscription payment ({})", subscription_id),
            paid_at: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.payment_repo.create(payment).await?;

        // Extend dues - look up membership type from member's current type
        let membership_type_slug: Option<String> = sqlx::query_scalar(
            r#"
            SELECT mt.slug FROM members m
            JOIN membership_types mt ON mt.id = m.membership_type_id
            WHERE m.id = ?
            "#
        )
        .bind(&member_id)
        .fetch_optional(&self.db_pool)
        .await
        .map_err(|e| AppError::Internal(format!("Database error: {}", e)))?;

        if let Some(slug) = membership_type_slug {
            self.extend_member_dues(member_uuid, &slug).await?;
        } else {
            // Fallback: extend by 1 month (conservative default for subscriptions).
            // Restore Expired -> Active but don't overwrite Suspended.
            let now = Utc::now();
            let new_date = now.checked_add_months(chrono::Months::new(1)).unwrap_or(now);
            sqlx::query(
                "UPDATE members \
                 SET dues_paid_until = ?, \
                     status = CASE WHEN status = 'Expired' THEN 'Active' ELSE status END, \
                     dues_reminder_sent_at = NULL, \
                     updated_at = CURRENT_TIMESTAMP \
                 WHERE id = ?"
            )
            .bind(new_date)
            .bind(&member_id)
            .execute(&self.db_pool)
            .await
            .map_err(|e| AppError::Internal(format!("Database error: {}", e)))?;
        }

        tracing::info!(
            "Subscription invoice paid for member {} (subscription: {})",
            member_id, subscription_id
        );

        Ok(())
    }

    /// Handle invoice.payment_failed - log warning (Stripe handles subscription retries)
    async fn handle_invoice_payment_failed(&self, invoice: stripe::Invoice) -> Result<()> {
        let customer_id = invoice.customer
            .as_ref()
            .map(|c| c.id().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let subscription_id = invoice.subscription
            .as_ref()
            .map(|s| s.id().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        tracing::warn!(
            "Subscription invoice payment failed for customer {} (subscription: {})",
            customer_id, subscription_id
        );

        Ok(())
    }

    /// Handle customer.subscription.deleted - transition member to manual billing
    async fn handle_subscription_deleted(&self, subscription: stripe::Subscription) -> Result<()> {
        let customer_id = subscription.customer.id().to_string();

        // Find and update member
        let result = sqlx::query(
            r#"
            UPDATE members
            SET stripe_subscription_id = NULL,
                billing_mode = 'manual',
                updated_at = CURRENT_TIMESTAMP
            WHERE stripe_customer_id = ?
            "#
        )
        .bind(&customer_id)
        .execute(&self.db_pool)
        .await
        .map_err(|e| AppError::Internal(format!("Database error: {}", e)))?;

        if result.rows_affected() > 0 {
            tracing::info!(
                "Subscription deleted for customer {}. Member transitioned to manual billing.",
                customer_id
            );
        } else {
            tracing::debug!(
                "Subscription deleted for customer {} but no matching member found",
                customer_id
            );
        }

        Ok(())
    }

    /// Handle customer.subscription.updated - update cached subscription info
    async fn handle_subscription_updated(&self, subscription: stripe::Subscription) -> Result<()> {
        let customer_id = subscription.customer.id().to_string();
        let subscription_id = subscription.id.to_string();
        let status = format!("{:?}", subscription.status);

        // Update the subscription ID in case it changed
        sqlx::query(
            r#"
            UPDATE members
            SET stripe_subscription_id = ?,
                updated_at = CURRENT_TIMESTAMP
            WHERE stripe_customer_id = ?
            "#
        )
        .bind(&subscription_id)
        .bind(&customer_id)
        .execute(&self.db_pool)
        .await
        .map_err(|e| AppError::Internal(format!("Database error: {}", e)))?;

        tracing::debug!(
            "Subscription {} updated for customer {} (status: {})",
            subscription_id, customer_id, status
        );

        Ok(())
    }

    /// Get or create a Stripe Customer for a member
    pub async fn get_or_create_customer(
        &self,
        member_id: Uuid,
        email: &str,
        name: &str,
    ) -> Result<String> {
        // Check if member already has a stripe_customer_id
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT stripe_customer_id FROM members WHERE id = ?"
        )
            .bind(member_id.to_string())
            .fetch_optional(&self.db_pool)
            .await
            .map_err(|e| AppError::Internal(format!("Database error: {}", e)))?
            .flatten();

        if let Some(customer_id) = existing {
            return Ok(customer_id);
        }

        // Create new Stripe Customer
        let mut params = CreateCustomer::new();
        params.email = Some(email);
        params.name = Some(name);
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("member_id".to_string(), member_id.to_string());
        params.metadata = Some(metadata);

        let customer = Customer::create(&self.client, params)
            .await
            .map_err(|e| AppError::External(format!("Stripe error: {}", e)))?;

        let customer_id = customer.id.to_string();

        // Store in member record
        sqlx::query("UPDATE members SET stripe_customer_id = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(&customer_id)
            .bind(member_id.to_string())
            .execute(&self.db_pool)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to store customer ID: {}", e)))?;

        tracing::info!("Created Stripe customer {} for member {}", customer_id, member_id);
        Ok(customer_id)
    }

    /// Create a SetupIntent for adding a payment method
    /// Returns the client_secret needed by Stripe.js
    pub async fn create_setup_intent(
        &self,
        member_id: Uuid,
        email: &str,
        name: &str,
    ) -> Result<String> {
        let customer_id = self.get_or_create_customer(member_id, email, name).await?;

        let mut params = CreateSetupIntent::new();
        params.customer = Some(customer_id.parse().map_err(|_| {
            AppError::Internal("Invalid customer ID".to_string())
        })?);
        params.payment_method_types = Some(vec!["card".to_string()]);

        let mut metadata = std::collections::HashMap::new();
        metadata.insert("member_id".to_string(), member_id.to_string());
        params.metadata = Some(metadata);

        let setup_intent = SetupIntent::create(&self.client, params)
            .await
            .map_err(|e| AppError::External(format!("Stripe error: {}", e)))?;

        setup_intent.client_secret
            .ok_or_else(|| AppError::External("No client_secret in SetupIntent".to_string()))
    }

    /// Charge a saved payment method (card).
    ///
    /// `idempotency_key` must be stable across retries of the same logical
    /// payment attempt. If the user double-clicks "Pay", both requests should
    /// pass the same key so Stripe returns the cached response and the card
    /// is only charged once. Callers typically generate this at form-render
    /// time (UUID in a hidden field) and thread it through.
    ///
    /// Returns the PaymentIntent ID if successful.
    pub async fn charge_saved_card(
        &self,
        member_id: Uuid,
        stripe_payment_method_id: &str,
        amount_cents: i64,
        description: &str,
        idempotency_key: &str,
    ) -> Result<String> {
        // Get the member's stripe_customer_id
        let customer_id: Option<String> = sqlx::query_scalar(
            "SELECT stripe_customer_id FROM members WHERE id = ?"
        )
            .bind(member_id.to_string())
            .fetch_optional(&self.db_pool)
            .await
            .map_err(|e| AppError::Internal(format!("Database error: {}", e)))?
            .flatten();

        let customer_id = customer_id
            .ok_or_else(|| AppError::BadRequest("Member has no Stripe customer".to_string()))?;

        let mut params = CreatePaymentIntent::new(amount_cents, Currency::USD);
        params.customer = Some(customer_id.parse().map_err(|_| {
            AppError::Internal("Invalid customer ID".to_string())
        })?);
        params.payment_method = Some(stripe_payment_method_id.parse().map_err(|_| {
            AppError::Internal("Invalid payment method ID".to_string())
        })?);
        params.confirm = Some(true);
        params.confirmation_method = Some(PaymentIntentConfirmationMethod::Automatic);
        params.description = Some(description);
        params.off_session = Some(stripe::PaymentIntentOffSession::exists(true));

        let mut metadata = std::collections::HashMap::new();
        metadata.insert("member_id".to_string(), member_id.to_string());
        params.metadata = Some(metadata);

        // Attach the idempotency key to this specific request.
        let idempotent_client = self.client.clone().with_strategy(
            stripe::RequestStrategy::Idempotent(idempotency_key.to_string())
        );

        let payment_intent = PaymentIntent::create(&idempotent_client, params)
            .await
            .map_err(|e| AppError::External(format!("Stripe charge failed: {}", e)))?;

        // Check status
        match payment_intent.status {
            stripe::PaymentIntentStatus::Succeeded => {
                tracing::info!("Successfully charged {} for member {}", amount_cents, member_id);
                Ok(payment_intent.id.to_string())
            }
            stripe::PaymentIntentStatus::RequiresAction => {
                Err(AppError::External("Payment requires additional authentication".to_string()))
            }
            status => {
                Err(AppError::External(format!("Payment failed with status: {:?}", status)))
            }
        }
    }

    /// Retrieve card details from a Stripe PaymentMethod
    pub async fn get_payment_method_details(
        &self,
        payment_method_id: &str,
    ) -> Result<CardDetails> {
        let pm_id: stripe::PaymentMethodId = payment_method_id.parse().map_err(|_| {
            AppError::Internal("Invalid payment method ID".to_string())
        })?;

        let pm = stripe::PaymentMethod::retrieve(
            &self.client,
            &pm_id,
            &[]
        )
            .await
            .map_err(|e| AppError::External(format!("Stripe error: {}", e)))?;

        let card = pm.card
            .ok_or_else(|| AppError::External("PaymentMethod has no card details".to_string()))?;

        Ok(CardDetails {
            last_four: card.last4,
            brand: card.brand.to_lowercase(),
            exp_month: card.exp_month as i32,
            exp_year: card.exp_year as i32,
        })
    }
}

/// Card details retrieved from Stripe
pub struct CardDetails {
    pub last_four: String,
    pub brand: String,
    pub exp_month: i32,
    pub exp_year: i32,
}