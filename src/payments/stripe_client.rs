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
    domain::{Payment, PaymentMethod, PaymentStatus, PaymentType, configurable_types::BillingPeriod},
    error::{AppError, Result},
    repository::PaymentRepository,
    service::{billing_service::BillingService, membership_type_service::MembershipTypeService},
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

        // Add metadata for tracking (store slug for dues extension lookup).
        // payment_type makes the webhook handler's branching explicit
        // — it pairs with the donation flow which sets payment_type=donation.
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("member_id".to_string(), member_id.to_string());
        metadata.insert("payment_type".to_string(), "membership".to_string());
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
            payment_type: PaymentType::Membership,
            donation_campaign_id: None,
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

    /// Build a Stripe Checkout session for a one-time donation. The
    /// session metadata includes `payment_type=donation` so the webhook
    /// handler knows NOT to extend dues. A pending Payment row is
    /// recorded with `payment_type=Donation` and (optionally)
    /// `donation_campaign_id` so totals attribute correctly even
    /// before the webhook flips the row to Completed.
    pub async fn create_donation_checkout_session(
        &self,
        member_id: Uuid,
        campaign_name: &str,
        campaign_id: Option<Uuid>,
        amount_cents: i64,
        success_url: String,
        cancel_url: String,
    ) -> Result<(String, Uuid)> {
        let mut params = CreateCheckoutSession::new();
        params.mode = Some(CheckoutSessionMode::Payment);
        params.success_url = Some(&success_url);
        params.cancel_url = Some(&cancel_url);

        let product_name = if campaign_id.is_some() {
            format!("Donation to {}", campaign_name)
        } else {
            "Donation".to_string()
        };

        params.line_items = Some(vec![CreateCheckoutSessionLineItems {
            price_data: Some(stripe::CreateCheckoutSessionLineItemsPriceData {
                currency: Currency::USD,
                unit_amount: Some(amount_cents),
                product_data: Some(stripe::CreateCheckoutSessionLineItemsPriceDataProductData {
                    name: product_name.clone(),
                    description: Some(product_name.clone()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            quantity: Some(1),
            ..Default::default()
        }]);

        let mut metadata = std::collections::HashMap::new();
        metadata.insert("member_id".to_string(), member_id.to_string());
        metadata.insert("payment_type".to_string(), "donation".to_string());
        if let Some(cid) = campaign_id {
            metadata.insert("donation_campaign_id".to_string(), cid.to_string());
        }
        params.metadata = Some(metadata);
        let member_id_str = member_id.to_string();
        params.client_reference_id = Some(&member_id_str);

        let session = CheckoutSession::create(&self.client, params)
            .await
            .map_err(|e| AppError::External(format!("Stripe error: {}", e)))?;

        let payment_id = Uuid::new_v4();
        let payment = Payment {
            id: payment_id,
            member_id,
            amount_cents,
            currency: "USD".to_string(),
            status: PaymentStatus::Pending,
            payment_method: PaymentMethod::Stripe,
            stripe_payment_id: Some(session.id.to_string()),
            description: product_name,
            payment_type: PaymentType::Donation,
            donation_campaign_id: campaign_id,
            paid_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        self.payment_repo.create(payment).await?;

        let url = session.url
            .ok_or_else(|| AppError::External("No checkout URL returned".to_string()))?;
        Ok((url, payment_id))
    }

    pub async fn handle_webhook(
        &self,
        payload: &str,
        stripe_signature: &str,
        billing_service: &BillingService,
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
                    self.handle_successful_payment(session, billing_service).await?;
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
                    self.handle_invoice_payment_failed(invoice, billing_service).await?;
                }
            }
            EventType::CustomerSubscriptionDeleted => {
                if let EventObject::Subscription(subscription) = event.data.object {
                    self.handle_subscription_deleted(subscription, billing_service).await?;
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
        billing_service: &BillingService,
    ) -> Result<()> {
        let session_id = session.id.to_string();

        let mut payment = match self.payment_repo.find_by_stripe_id(&session_id).await? {
            Some(p) => p,
            None => {
                tracing::warn!("Payment not found for Stripe session: {}", session_id);
                return Ok(());
            }
        };

        // Always flip Pending → Completed on a successful checkout.
        payment.status = PaymentStatus::Completed;
        payment.paid_at = Some(Utc::now());
        payment.updated_at = Utc::now();
        self.payment_repo.update(payment.id, payment.clone()).await?;

        // Branch on payment type. Donations don't extend dues and
        // don't refresh auto-renew schedules — they're a separate
        // bucket from membership renewal. Reading from session
        // metadata avoids a second DB lookup; falling back to the
        // payment row's stored type covers older sessions that
        // didn't write the metadata key (i.e. created before this
        // code shipped).
        let payment_type_str = session.metadata
            .as_ref()
            .and_then(|m| m.get("payment_type"))
            .cloned()
            .unwrap_or_else(|| payment.payment_type.as_str().to_string());

        if payment_type_str == "donation" {
            tracing::info!(
                "Donation completed for member {} (campaign: {:?}, amount: {})",
                payment.member_id, payment.donation_campaign_id, payment.amount_cents,
            );
            return Ok(());
        }

        // Membership flow. Look up the slug from metadata and run
        // the dues-extend + reschedule-if-enrolled chain.
        let membership_type_slug = session.metadata
            .as_ref()
            .and_then(|m| m.get("membership_type_slug"))
            .cloned();

        if let Some(slug) = &membership_type_slug {
            self.extend_member_dues(payment.member_id, slug).await?;

            if let Err(e) = billing_service
                .reschedule_after_payment(payment.member_id, slug)
                .await
            {
                tracing::error!(
                    "Member {} paid via Checkout but reschedule failed: {}",
                    payment.member_id, e,
                );
            }
        } else {
            tracing::warn!(
                "No membership_type_slug in session metadata for session: {}",
                session_id,
            );
        }

        tracing::info!("Payment completed for member: {}", payment.member_id);
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
            payment_type: PaymentType::Membership,
            donation_campaign_id: None,
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

    /// Handle invoice.payment_failed for Stripe-managed subscriptions.
    ///
    /// Stripe retries on its side over several days; we'd spam the
    /// member if we emailed on every attempt. So we only notify on:
    ///   - the first failure (attempt_count == 1) — "your card was
    ///     declined, please update; we'll retry automatically"
    ///   - the final failure (next_payment_attempt is None) — "this
    ///     was the last try; update + manually re-pay or your
    ///     membership will lapse"
    ///
    /// Doesn't touch member status or dues_paid_until — the existing
    /// expiration job catches them naturally once Stripe gives up
    /// and the paid-through date passes.
    async fn handle_invoice_payment_failed(
        &self,
        invoice: stripe::Invoice,
        billing_service: &BillingService,
    ) -> Result<()> {
        let customer_id = match &invoice.customer {
            Some(c) => c.id().to_string(),
            None => {
                tracing::warn!("invoice.payment_failed without a customer; skipping");
                return Ok(());
            }
        };
        let subscription_id = invoice.subscription
            .as_ref()
            .map(|s| s.id().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        // Find the member behind this Stripe customer.
        let member_id_str: Option<String> = sqlx::query_scalar(
            "SELECT id FROM members WHERE stripe_customer_id = ?",
        )
        .bind(&customer_id)
        .fetch_optional(&self.db_pool)
        .await
        .map_err(|e| AppError::Internal(format!("Database error: {}", e)))?;

        let member_id = match member_id_str.and_then(|s| Uuid::parse_str(&s).ok()) {
            Some(id) => id,
            None => {
                tracing::warn!(
                    "invoice.payment_failed for unknown Stripe customer {} (subscription: {})",
                    customer_id, subscription_id,
                );
                return Ok(());
            }
        };

        let attempt_count = invoice.attempt_count.unwrap_or(0);
        let is_final = invoice.next_payment_attempt.is_none();
        let is_first = attempt_count <= 1;

        // Always log so operators have a paper trail of every retry.
        tracing::warn!(
            "Subscription charge failed for member {} (customer {}, subscription {}, attempt {}, final: {})",
            member_id, customer_id, subscription_id, attempt_count, is_final,
        );

        // Only notify on first + final to avoid spam during retries.
        if !is_first && !is_final {
            return Ok(());
        }

        // Format the amount for display. amount_due is the canonical
        // figure for "what we tried to charge"; fall back to amount_remaining.
        let amount_cents = invoice.amount_due
            .or(invoice.amount_remaining)
            .unwrap_or(0);
        let amount_display = if amount_cents > 0 {
            Some(format!("${:.2}", amount_cents as f64 / 100.0))
        } else {
            None
        };

        if let Err(e) = billing_service
            .notify_subscription_payment_failed(member_id, amount_display, is_final)
            .await
        {
            tracing::error!(
                "Couldn't notify member {} of failed subscription charge: {}",
                member_id, e,
            );
        }

        Ok(())
    }

    /// Handle customer.subscription.deleted.
    ///
    /// This webhook fires in two distinct situations:
    ///
    /// 1. **Member cancelled out-of-band** (e.g. via Stripe's hosted
    ///    customer portal). billing_mode is still 'stripe_subscription'
    ///    when we get here. Flip them to 'manual' AND email them so
    ///    they know auto-renew is off but their access continues.
    ///
    /// 2. **Echo from our own migration**. The migration cancelled
    ///    Stripe sub as part of moving the member to coterie_managed,
    ///    so by the time the webhook arrives billing_mode is already
    ///    'coterie_managed'. Skip silently — emailing them about a
    ///    "cancellation" they don't know about would be confusing.
    async fn handle_subscription_deleted(
        &self,
        subscription: stripe::Subscription,
        billing_service: &BillingService,
    ) -> Result<()> {
        let customer_id = subscription.customer.id().to_string();

        // Look up state BEFORE writing — we need to distinguish the
        // two cases above by reading billing_mode.
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT id, billing_mode FROM members WHERE stripe_customer_id = ?",
        )
        .bind(&customer_id)
        .fetch_optional(&self.db_pool)
        .await
        .map_err(|e| AppError::Internal(format!("Database error: {}", e)))?;

        let (member_id_str, current_mode) = match row {
            Some(r) => r,
            None => {
                tracing::debug!(
                    "subscription.deleted for customer {} — no matching member",
                    customer_id,
                );
                return Ok(());
            }
        };

        if current_mode != "stripe_subscription" {
            // Echo from our own migration; nothing to do.
            tracing::debug!(
                "subscription.deleted echo for migrated customer {} (mode={}); ignoring",
                customer_id, current_mode,
            );
            return Ok(());
        }

        // Real out-of-band cancellation. Flip + notify.
        sqlx::query(
            r#"
            UPDATE members
            SET stripe_subscription_id = NULL,
                billing_mode = 'manual',
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?
            "#,
        )
        .bind(&member_id_str)
        .execute(&self.db_pool)
        .await
        .map_err(|e| AppError::Internal(format!("Database error: {}", e)))?;

        tracing::info!(
            "Subscription cancelled out-of-band for customer {}; switched member to manual",
            customer_id,
        );

        if let Ok(member_id) = Uuid::parse_str(&member_id_str) {
            if let Err(e) = billing_service
                .notify_subscription_cancelled(member_id)
                .await
            {
                tracing::error!(
                    "Switched member {} to manual but notification failed: {}",
                    member_id, e,
                );
            }
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

    /// Pull every card-type PaymentMethod attached to this Stripe
    /// customer, marking which one is their `invoice_settings.
    /// default_payment_method`. Used by the Stripe-subscription →
    /// Coterie-managed migration so we can hydrate Coterie's
    /// SavedCard table without making the member re-enter card info.
    ///
    /// Returns an empty list if the customer has no cards on file.
    /// Bails on Stripe API errors — caller should treat that as
    /// "don't migrate this member yet."
    pub async fn list_customer_cards(
        &self,
        customer_id: &str,
    ) -> Result<Vec<CustomerCardSummary>> {
        let cid: stripe::CustomerId = customer_id.parse().map_err(|_| {
            AppError::BadRequest(format!("Invalid customer ID: {}", customer_id))
        })?;

        // Default PM lives on Customer.invoice_settings.default_payment_method.
        // We retrieve once so we can flag which list entry is the default.
        let customer = stripe::Customer::retrieve(&self.client, &cid, &[])
            .await
            .map_err(|e| AppError::External(format!("Stripe customer lookup: {}", e)))?;

        let default_pm_id: Option<String> = customer.invoice_settings
            .as_ref()
            .and_then(|s| s.default_payment_method.as_ref())
            .map(|exp| exp.id().to_string());

        // Single page is fine — typical members have 1–2 cards. Stripe's
        // page size cap is 100 which is plenty.
        let mut params = stripe::ListPaymentMethods::new();
        params.customer = Some(cid);
        params.type_ = Some(stripe::PaymentMethodTypeFilter::Card);
        params.limit = Some(100);

        let list = stripe::PaymentMethod::list(&self.client, &params)
            .await
            .map_err(|e| AppError::External(format!("Stripe PM list: {}", e)))?;

        let mut out = Vec::new();
        for pm in list.data {
            let pm_id = pm.id.to_string();
            let card = match pm.card {
                Some(c) => c,
                None => continue, // Defensive — type filter should prevent this
            };
            out.push(CustomerCardSummary {
                is_default: default_pm_id.as_deref() == Some(pm_id.as_str()),
                payment_method_id: pm_id,
                last_four: card.last4,
                brand: card.brand.to_lowercase(),
                exp_month: card.exp_month as i32,
                exp_year: card.exp_year as i32,
            });
        }

        // Defensive: if Stripe didn't tell us which is default and we
        // got exactly one, treat it as default. (Stripe sometimes
        // leaves invoice_settings.default_payment_method null for
        // older customers — the subscription was using "the only card
        // on file" implicitly.)
        if !out.iter().any(|c| c.is_default) && out.len() == 1 {
            out[0].is_default = true;
        }

        Ok(out)
    }

    /// Immediately cancel a Stripe subscription. Used during the
    /// migration from Stripe-managed to Coterie-managed auto-renew —
    /// once we own the schedule, Stripe shouldn't keep charging.
    ///
    /// Stripe will fire `customer.subscription.deleted` to our
    /// webhook in response. The webhook handler is intentionally
    /// idempotent against members already in `coterie_managed`
    /// (won't clobber their billing_mode back to manual).
    pub async fn cancel_subscription(&self, subscription_id: &str) -> Result<()> {
        let sub_id: stripe::SubscriptionId = subscription_id.parse().map_err(|_| {
            AppError::BadRequest(format!("Invalid subscription ID: {}", subscription_id))
        })?;
        stripe::Subscription::delete(&self.client, &sub_id)
            .await
            .map_err(|e| AppError::External(format!("Stripe cancel failed: {}", e)))?;
        tracing::info!("Cancelled Stripe subscription {}", subscription_id);
        Ok(())
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

/// One card-type PaymentMethod attached to a Stripe customer, plus a
/// flag for whether it's the customer's invoice-default. Returned by
/// `list_customer_cards` for use during stripe-subscription migration.
pub struct CustomerCardSummary {
    pub payment_method_id: String,
    pub last_four: String,
    pub brand: String,
    pub exp_month: i32,
    pub exp_year: i32,
    pub is_default: bool,
}