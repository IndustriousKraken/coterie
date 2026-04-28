use stripe::{CheckoutSession, EventObject, EventType, Webhook, WebhookError};
use chrono::Utc;
use uuid::Uuid;
use std::sync::Arc;
use sqlx::SqlitePool;

use crate::{
    domain::{Payment, PaymentMethod, PaymentStatus, PaymentType, configurable_types::BillingPeriod},
    error::{AppError, Result},
    integrations::{IntegrationEvent, IntegrationManager},
    payments::gateway::{
        CreateCheckoutInput, CreateCustomerInput, CreatePaymentIntentInput,
        CreateRefundInput, CreateSetupIntentInput, LineItemInput,
        PaymentIntentResult, StripeGateway,
    },
    repository::{MemberRepository, PaymentRepository},
    service::{billing_service::BillingService, membership_type_service::MembershipTypeService},
};

pub struct StripeClient {
    /// Trait-based seam over the Stripe API. Production wraps stripe-rs
    /// (`RealStripeGateway`); tests substitute a fake. Webhook signature
    /// verification (`Webhook::construct_event`) is static and lives
    /// outside the trait.
    gateway: Arc<dyn StripeGateway>,
    webhook_secret: String,
    payment_repo: Arc<dyn PaymentRepository>,
    member_repo: Arc<dyn MemberRepository>,
    membership_type_service: Arc<MembershipTypeService>,
    integration_manager: Arc<IntegrationManager>,
    db_pool: SqlitePool,
}

impl StripeClient {
    /// Production constructor: builds a `RealStripeGateway` from the
    /// API key.
    pub fn new(
        api_key: String,
        webhook_secret: String,
        payment_repo: Arc<dyn PaymentRepository>,
        member_repo: Arc<dyn MemberRepository>,
        membership_type_service: Arc<MembershipTypeService>,
        integration_manager: Arc<IntegrationManager>,
        db_pool: SqlitePool,
    ) -> Self {
        let gateway: Arc<dyn StripeGateway> =
            Arc::new(crate::payments::gateway::RealStripeGateway::new(api_key));
        Self::with_gateway(
            gateway, webhook_secret, payment_repo, member_repo,
            membership_type_service, integration_manager, db_pool,
        )
    }

    /// Test seam: build a `StripeClient` with an explicit gateway (e.g.
    /// `FakeStripeGateway`). Used by integration tests to exercise the
    /// application logic without making real Stripe calls.
    pub fn with_gateway(
        gateway: Arc<dyn StripeGateway>,
        webhook_secret: String,
        payment_repo: Arc<dyn PaymentRepository>,
        member_repo: Arc<dyn MemberRepository>,
        membership_type_service: Arc<MembershipTypeService>,
        integration_manager: Arc<IntegrationManager>,
        db_pool: SqlitePool,
    ) -> Self {
        Self {
            gateway,
            webhook_secret,
            payment_repo,
            member_repo,
            membership_type_service,
            integration_manager,
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
        // Metadata: payment_type makes the webhook handler's branching
        // explicit (pairs with the donation flow which sets
        // payment_type=donation); membership_type_slug is what dues
        // extension looks up on the type registry.
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("member_id".to_string(), member_id.to_string());
        metadata.insert("payment_type".to_string(), "membership".to_string());
        metadata.insert("membership_type".to_string(), membership_type_name.to_string());
        metadata.insert("membership_type_slug".to_string(), membership_type_slug.to_string());

        let session = self.gateway.create_checkout_session(CreateCheckoutInput {
            success_url,
            cancel_url,
            line_items: vec![LineItemInput {
                amount_cents,
                product_name: format!("{} Membership", membership_type_name),
                product_description: Some(format!("{} membership dues", membership_type_name)),
            }],
            metadata,
            client_reference_id: Some(member_id.to_string()),
            customer_email: None,
        }).await?;

        // Create pending payment record
        let payment_id = Uuid::new_v4();
        let payment = Payment {
            id: payment_id,
            member_id: Some(member_id),
            amount_cents,
            currency: "USD".to_string(),
            status: PaymentStatus::Pending,
            payment_method: PaymentMethod::Stripe,
            stripe_payment_id: Some(session.session_id),
            description: format!("{} Membership Payment", membership_type_name),
            payment_type: PaymentType::Membership,
            donation_campaign_id: None,
            donor_name: None,
            donor_email: None,
            paid_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.payment_repo.create(payment).await?;

        Ok((session.url, payment_id))
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
        let product_name = if campaign_id.is_some() {
            format!("Donation to {}", campaign_name)
        } else {
            "Donation".to_string()
        };

        let mut metadata = std::collections::HashMap::new();
        metadata.insert("member_id".to_string(), member_id.to_string());
        metadata.insert("payment_type".to_string(), "donation".to_string());
        if let Some(cid) = campaign_id {
            metadata.insert("donation_campaign_id".to_string(), cid.to_string());
        }

        let session = self.gateway.create_checkout_session(CreateCheckoutInput {
            success_url,
            cancel_url,
            line_items: vec![LineItemInput {
                amount_cents,
                product_name: product_name.clone(),
                product_description: Some(product_name.clone()),
            }],
            metadata,
            client_reference_id: Some(member_id.to_string()),
            customer_email: None,
        }).await?;

        let payment_id = Uuid::new_v4();
        let payment = Payment {
            id: payment_id,
            member_id: Some(member_id),
            amount_cents,
            currency: "USD".to_string(),
            status: PaymentStatus::Pending,
            payment_method: PaymentMethod::Stripe,
            stripe_payment_id: Some(session.session_id),
            description: product_name,
            payment_type: PaymentType::Donation,
            donation_campaign_id: campaign_id,
            donor_name: None,
            donor_email: None,
            paid_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        self.payment_repo.create(payment).await?;

        Ok((session.url, payment_id))
    }

    /// Donation Checkout for a public (non-member) donor. The donor's
    /// name and email come from the public form, not from a logged-in
    /// session. Stripe collects card + billing details on the hosted
    /// page; we just stamp the metadata so the webhook knows it's a
    /// public donation and can complete the row without trying to
    /// extend dues.
    ///
    /// Mirrors `create_donation_checkout_session` except no member_id
    /// is involved — the Pending Payment row gets donor_name +
    /// donor_email instead, and the CHECK constraint on the table
    /// keeps the invariant intact.
    pub async fn create_public_donation_checkout_session(
        &self,
        donor_name: &str,
        donor_email: &str,
        campaign_name: &str,
        campaign_id: Option<Uuid>,
        amount_cents: i64,
        success_url: String,
        cancel_url: String,
    ) -> Result<(String, Uuid)> {
        let product_name = if campaign_id.is_some() {
            format!("Donation to {}", campaign_name)
        } else {
            "Donation".to_string()
        };

        let mut metadata = std::collections::HashMap::new();
        metadata.insert("payment_type".to_string(), "donation".to_string());
        metadata.insert("public_donation".to_string(), "1".to_string());
        metadata.insert("donor_name".to_string(), donor_name.to_string());
        metadata.insert("donor_email".to_string(), donor_email.to_string());
        if let Some(cid) = campaign_id {
            metadata.insert("donation_campaign_id".to_string(), cid.to_string());
        }

        let session = self.gateway.create_checkout_session(CreateCheckoutInput {
            success_url,
            cancel_url,
            line_items: vec![LineItemInput {
                amount_cents,
                product_name: product_name.clone(),
                product_description: Some(product_name.clone()),
            }],
            metadata,
            client_reference_id: None,
            // Pre-fill the email on the hosted Checkout page. Stripe also
            // sends the receipt to whatever email the donor confirms.
            customer_email: Some(donor_email.to_string()),
        }).await?;

        let payment_id = Uuid::new_v4();
        let payment = Payment {
            id: payment_id,
            member_id: None,
            amount_cents,
            currency: "USD".to_string(),
            status: PaymentStatus::Pending,
            payment_method: PaymentMethod::Stripe,
            stripe_payment_id: Some(session.session_id),
            description: format!("{} — {}", product_name, donor_name),
            payment_type: PaymentType::Donation,
            donation_campaign_id: campaign_id,
            donor_name: Some(donor_name.to_string()),
            donor_email: Some(donor_email.to_string()),
            paid_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        self.payment_repo.create(payment).await?;

        Ok((session.url, payment_id))
    }

    pub async fn handle_webhook(
        &self,
        payload: &str,
        stripe_signature: &str,
        billing_service: &BillingService,
    ) -> Result<()> {
        // Verify webhook signature and construct event. Specific
        // strings here are pattern-matched by the handler in
        // api/handlers/payments.rs to dispatch AdminAlerts — keep
        // them stable.
        let event = Webhook::construct_event(
            payload,
            stripe_signature,
            &self.webhook_secret,
        )
        .map_err(|e| match e {
            WebhookError::BadSignature => AppError::BadRequest("Invalid signature".to_string()),
            WebhookError::BadTimestamp(skew_secs) => AppError::BadRequest(format!(
                "Webhook timestamp out of tolerance (skew: {}s) — clock drift",
                skew_secs,
            )),
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

        // Dispatch to the right handler. Capture the result so we can
        // roll back the idempotency claim on failure — otherwise a
        // partial-success run (Payment row updated, dues extension
        // failed) would be permanently stuck: the next Stripe retry
        // would hit the claim and skip without re-running the failed
        // step, leaving the member paid-but-not-extended forever.
        let outcome: Result<()> = async {
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

                // Self-heal for saved-card / donation flows that insert
                // a Pending payment row before charging Stripe. If the
                // synchronous response was lost (process crash, network
                // drop) the webhook arrives ~seconds later and flips
                // the row to Completed + does the post-payment work.
                EventType::PaymentIntentSucceeded => {
                    if let EventObject::PaymentIntent(intent) = event.data.object {
                        self.handle_payment_intent_succeeded(intent, billing_service).await?;
                    }
                }

                // Refunds — fired when an admin issues one through Stripe's
                // dashboard (or as the echo from our own admin-button refund).
                EventType::ChargeRefunded => {
                    if let EventObject::Charge(charge) = event.data.object {
                        self.handle_charge_refunded(charge).await?;
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
        .await;

        if let Err(e) = &outcome {
            tracing::error!(
                "Webhook handler for event {} ({}) failed: {}; rolling back idempotency claim so Stripe retry can re-run",
                event_id, event_type, e,
            );
            // Best-effort rollback. If THIS fails, the event stays
            // claimed and the retry will skip — but at that point the
            // DB is in a state where we can't trust further writes
            // anyway, and the original error is the more important
            // signal to surface.
            if let Err(rollback_err) = sqlx::query(
                "DELETE FROM processed_stripe_events WHERE event_id = ?",
            )
            .bind(&event_id)
            .execute(&self.db_pool)
            .await
            {
                tracing::error!(
                    "Idempotency rollback failed for event {}: {}. The event \
                     is permanently claimed; manual intervention may be needed.",
                    event_id, rollback_err,
                );
            }
        }

        outcome
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
        // Also upgrade stripe_payment_id from cs_ → pi_ so future
        // refund webhooks (charge.refunded carries a PaymentIntent ID,
        // not a CheckoutSession ID) can match this row by IN-clause.
        // Fall back to keeping cs_ if Stripe didn't expand the PI on
        // the session (rare, but defensive).
        //
        // Use complete_pending_payment so that if Stripe retries this
        // event (because dispatch failed somewhere below and we rolled
        // back the idempotency claim), the second run sees the row as
        // already Completed and skips the post-work — preventing the
        // double-extension race that the per-payment dues claim also
        // guards against.
        let pi_for_row = session.payment_intent
            .as_ref()
            .map(|exp| exp.id().to_string())
            .unwrap_or_else(|| session_id.clone());
        let won_flip = self.payment_repo
            .complete_pending_payment(payment.id, &pi_for_row)
            .await?;
        if !won_flip {
            tracing::debug!(
                "checkout.session.completed for payment {} that's already Completed; \
                 skipping post-work (likely a Stripe retry after a previous handler error)",
                payment.id,
            );
            return Ok(());
        }

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
                "Donation completed: payment={} member={:?} donor={:?} campaign={:?} amount={}",
                payment.id, payment.member_id, payment.donor_email,
                payment.donation_campaign_id, payment.amount_cents,
            );
            return Ok(());
        }

        // Membership flow. A membership Checkout session was always
        // created with a real member_id (see create_membership_checkout_session)
        // so the Option here is invariably Some. Bail loudly if not —
        // it's a data-integrity violation and we don't want to silently
        // run the rest of the flow with a fabricated UUID.
        let member_id = match payment.member_id {
            Some(id) => id,
            None => {
                tracing::error!(
                    "Membership payment {} has NULL member_id — data integrity violation",
                    payment.id,
                );
                return Err(AppError::Database(
                    "membership payment without member_id".to_string()
                ));
            }
        };

        // Look up the slug from metadata and run the dues-extend +
        // reschedule-if-enrolled chain.
        let membership_type_slug = session.metadata
            .as_ref()
            .and_then(|m| m.get("membership_type_slug"))
            .cloned();

        // Resolve slug: prefer the metadata stamp from
        // create_membership_checkout_session; fall back to the
        // member's current membership_type if the metadata is missing
        // (legacy sessions, or anything created without our helper).
        // Without the fallback, a missing metadata field meant the
        // payment row was flipped Completed but dues were silently
        // not extended — money taken, member expires later. Same
        // self-heal logic as handle_payment_intent_succeeded.
        let resolved_slug = if let Some(s) = membership_type_slug {
            Some(s)
        } else {
            tracing::warn!(
                "No membership_type_slug in session metadata for session {}; \
                 falling back to member's current membership type",
                session_id,
            );
            let member = self.member_repo.find_by_id(member_id).await?;
            let mt_id = member.as_ref().and_then(|m| m.membership_type_id);
            match mt_id {
                Some(id) => self.membership_type_service.get(id).await?.map(|mt| mt.slug),
                None => None,
            }
        };

        if let Some(slug) = &resolved_slug {
            self.extend_member_dues(payment.id, member_id, slug).await?;

            if let Err(e) = billing_service
                .reschedule_after_payment(member_id, slug)
                .await
            {
                tracing::error!(
                    "Member {} paid via Checkout but reschedule failed: {}",
                    member_id, e,
                );
            }
        } else {
            tracing::error!(
                "Couldn't resolve membership type for paid Checkout session {}; \
                 dues NOT extended for member {} — operator must reconcile",
                session_id, member_id,
            );
            self.integration_manager.handle_event(IntegrationEvent::AdminAlert {
                subject: format!(
                    "Checkout paid but dues not extended — member {}",
                    member_id,
                ),
                body: format!(
                    "Checkout session {} (payment {}) was paid by member {} \
                     but no membership type could be resolved (no metadata, \
                     no current type on record). Dues were NOT extended. \
                     Reconcile manually.",
                    session_id, payment.id, member_id,
                ),
            }).await;
        }

        tracing::info!("Payment completed for member: {}", member_id);
        Ok(())
    }

    pub async fn extend_member_dues(
        &self,
        payment_id: Uuid,
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

        let extended = self.payment_repo
            .extend_dues_for_payment_atomic(payment_id, member_id, billing_period)
            .await?;

        if extended {
            tracing::info!(
                "Extended dues for member {} (payment: {}, billing period: {:?})",
                member_id, payment_id, billing_period,
            );
        } else {
            tracing::debug!(
                "Dues already extended for payment {}; skipping",
                payment_id,
            );
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

    /// Self-heal handler for the Pending-first saved-card flow. When
    /// donate_api or charge_saved_card_api inserts a Pending row,
    /// charges Stripe, and crashes (or drops the response) before
    /// completing the row, this webhook arrives ~seconds later and
    /// finishes the work.
    ///
    /// The flip is race-free against the synchronous path:
    /// `complete_pending_payment` only changes a row whose status is
    /// still `Pending`, and returns whether *we* did the flip. Whoever
    /// flips owns the post-work — the loser bails to avoid double-
    /// extending dues.
    ///
    /// PIs that don't carry our `payment_id` metadata (legacy charges,
    /// charges from other systems) are logged and skipped — we have
    /// nothing to do for them. PIs whose Pending row hasn't been
    /// inserted yet (e.g. the billing runner crashed between charge
    /// and row insert) are also a no-op here; the runner's retry will
    /// recover them via Stripe's idempotency key.
    async fn handle_payment_intent_succeeded(
        &self,
        intent: stripe::PaymentIntent,
        billing_service: &BillingService,
    ) -> Result<()> {
        let payment_id_str = match intent.metadata.get("payment_id") {
            Some(s) => s.clone(),
            None => {
                tracing::debug!(
                    "PI {} succeeded with no payment_id metadata; not a Coterie-tracked charge",
                    intent.id,
                );
                return Ok(());
            }
        };
        let payment_id = match Uuid::parse_str(&payment_id_str) {
            Ok(id) => id,
            Err(_) => {
                tracing::warn!(
                    "PI {} has malformed payment_id metadata: '{}'",
                    intent.id, payment_id_str,
                );
                return Ok(());
            }
        };

        let payment = match self.payment_repo.find_by_id(payment_id).await? {
            Some(p) => p,
            None => {
                tracing::warn!(
                    "PI {} succeeded but local payment {} doesn't exist — \
                     either a runner-flow whose row insert hasn't run yet, \
                     or a sync-path crash before the Pending row was created. \
                     Will be reconciled on the next runner tick or via the \
                     Pending-payments review.",
                    intent.id, payment_id,
                );
                return Ok(());
            }
        };

        // Cross-check PI metadata against the local row before
        // mutating. The flip below is conditional on status='Pending',
        // so a Completed row can't be re-flipped — but a Pending row
        // belonging to one member could in theory be stapled to a PI
        // crafted with that row's id but a different member/amount
        // by anyone with Stripe API write access. We refuse if the
        // shape doesn't match: same member, same amount. Public donations
        // (payment.member_id IS NULL) skip the member equality check —
        // the PI metadata for those carries donor_email instead.
        let metadata_member_id = intent.metadata.get("member_id")
            .and_then(|s| Uuid::parse_str(s).ok());
        if payment.member_id.is_some() && metadata_member_id != payment.member_id {
            tracing::warn!(
                "PI {} payment_id metadata points at payment {} (member {:?}), \
                 but PI's member_id metadata is {:?}; refusing to act",
                intent.id, payment_id, payment.member_id, metadata_member_id,
            );
            return Ok(());
        }
        if intent.amount != payment.amount_cents {
            tracing::warn!(
                "PI {} amount {} doesn't match local payment {} amount {}; refusing to act",
                intent.id, intent.amount, payment_id, payment.amount_cents,
            );
            return Ok(());
        }

        let pi_id = intent.id.to_string();
        let won_flip = self.payment_repo
            .complete_pending_payment(payment_id, &pi_id)
            .await?;
        if !won_flip {
            tracing::debug!(
                "PI {} succeeded but payment {} was already completed; sync path won the race",
                intent.id, payment_id,
            );
            return Ok(());
        }

        tracing::info!(
            "Self-healing payment {} via PI.succeeded webhook (member: {:?}, donor: {:?})",
            payment_id, payment.member_id, payment.donor_email,
        );

        // Post-work depends on payment_type. Donations have none —
        // the row flip is the entire job. Membership payments need
        // dues extended and (if auto-renew enrolled) the next renewal
        // rescheduled. We look up the slug from the member's current
        // membership_type since saved-card charges don't carry it on
        // the Payment row.
        if payment.payment_type == PaymentType::Membership {
            // Membership payments must have a member; data integrity
            // violation otherwise (CHECK constraint should prevent it).
            let member_id = match payment.member_id {
                Some(id) => id,
                None => {
                    tracing::error!(
                        "Membership payment {} has NULL member_id — data integrity violation",
                        payment_id,
                    );
                    return Err(AppError::Database(
                        "membership payment without member_id".to_string()
                    ));
                }
            };
            let member = match self.member_repo.find_by_id(member_id).await? {
                Some(m) => m,
                None => {
                    tracing::warn!(
                        "Self-healed payment {} for missing member {}; skipping post-work",
                        payment_id, member_id,
                    );
                    return Ok(());
                }
            };
            let mt_id = match member.membership_type_id {
                Some(id) => id,
                None => {
                    tracing::warn!(
                        "Member {} has no membership_type_id; can't extend dues for self-healed payment {}",
                        member_id, payment_id,
                    );
                    return Ok(());
                }
            };
            let mt = self.membership_type_service.get(mt_id).await?;
            let slug = match mt {
                Some(t) => t.slug,
                None => {
                    tracing::warn!(
                        "Member {}'s membership_type {} not found; can't extend dues for self-healed payment {}",
                        member_id, mt_id, payment_id,
                    );
                    return Ok(());
                }
            };
            self.extend_member_dues(payment_id, member_id, &slug).await?;
            if let Err(e) = billing_service
                .reschedule_after_payment(member_id, &slug)
                .await
            {
                tracing::error!(
                    "Self-healed payment {} but reschedule failed: {}",
                    payment_id, e,
                );
            }
        }

        Ok(())
    }

    // ================================================================
    // Legacy Stripe Subscription Handlers
    // ================================================================

    /// Sync state when a Stripe charge is refunded. Two ways this fires:
    ///
    ///   1. **Admin clicked Refund in Coterie** — our admin handler
    ///      already flipped the Payment row to Refunded and called the
    ///      Stripe API. This is the echo. Idempotent: if the row is
    ///      already Refunded, we no-op.
    ///
    ///   2. **Admin refunded via Stripe's dashboard** — our local row
    ///      is still Completed. Find it (by payment_intent or invoice
    ///      ID, depending on what we stored) and mark Refunded.
    ///
    /// Partial refunds (`amount_refunded < amount`) don't flip the row;
    /// instead we log + dispatch an AdminAlert so an operator can decide
    /// whether to update the local record. Full-refund-only matches the
    /// admin-UI behavior — partial refunds would muddle dues / campaign
    /// totals.
    async fn handle_charge_refunded(&self, charge: stripe::Charge) -> Result<()> {
        let charge_amount = charge.amount;
        let amount_refunded = charge.amount_refunded;
        let is_partial = amount_refunded < charge_amount;

        // Try to find the Payment row. New checkout flows upgrade
        // stripe_payment_id from cs_ → pi_ on completion (see
        // handle_successful_payment), so the IN clause covers them
        // alongside saved-card (pi_) and Stripe-subscription (in_)
        // payments. Legacy rows still keyed by cs_ fall through to
        // the Stripe-API lookup below.
        let pi_id = charge.payment_intent.as_ref().map(|e| e.id().to_string());
        let invoice_id = charge.invoice.as_ref().map(|e| e.id().to_string());

        let mut row: Option<(String, String)> = sqlx::query_as(
            r#"
            SELECT id, status FROM payments
            WHERE stripe_payment_id IS NOT NULL
              AND stripe_payment_id IN (?, ?)
            LIMIT 1
            "#,
        )
        .bind(pi_id.as_deref().unwrap_or(""))
        .bind(invoice_id.as_deref().unwrap_or(""))
        .fetch_optional(&self.db_pool)
        .await
        .map_err(|e| AppError::Internal(format!("Database error: {}", e)))?;

        // Fallback for legacy cs_ rows: ask Stripe which CheckoutSession
        // owns this PaymentIntent, then match. The list API filters
        // server-side, so this is a single round trip.
        if row.is_none() {
            if let Some(pi) = pi_id.as_deref() {
                if let Ok(sessions) = self.gateway.list_checkout_sessions_by_intent(pi).await {
                    if let Some(cs_id) = sessions.first() {
                        row = sqlx::query_as(
                            "SELECT id, status FROM payments WHERE stripe_payment_id = ? LIMIT 1",
                        )
                        .bind(cs_id)
                        .fetch_optional(&self.db_pool)
                        .await
                        .map_err(|e| AppError::Internal(format!("Database error: {}", e)))?;
                    }
                }
            }
        }

        let (payment_id, current_status) = match row {
            Some(r) => r,
            None => {
                tracing::warn!(
                    "charge.refunded for charge {} — no matching local Payment (pi={:?}, invoice={:?}). \
                     This may be a checkout-session payment; mark refunded manually if so.",
                    charge.id, pi_id, invoice_id,
                );
                return Ok(());
            }
        };

        if is_partial {
            tracing::warn!(
                "Partial refund on payment {} (refunded {} of {} cents) — \
                 local row left as-is; flag for an operator.",
                payment_id, amount_refunded, charge_amount,
            );
            // We don't update the row — partial refunds would muddle
            // dues/campaign accounting. Surface to operators so they
            // can decide whether to mark Refunded manually.
            self.integration_manager.handle_event(IntegrationEvent::AdminAlert {
                subject: format!(
                    "Partial Stripe refund — payment {} (${:.2} of ${:.2})",
                    payment_id,
                    amount_refunded as f64 / 100.0,
                    charge_amount as f64 / 100.0,
                ),
                body: format!(
                    "Stripe charge {} was partially refunded (${:.2} of ${:.2}).\n\n\
                     The local Coterie payment row {} is unchanged — partial \
                     refunds aren't supported in our admin UI because they \
                     muddle dues / campaign accounting.\n\n\
                     If this was intentional, mark the payment Refunded \
                     manually in the DB and adjust dues / campaign totals \
                     to match. Otherwise investigate who issued the \
                     partial refund in Stripe's dashboard.",
                    charge.id, amount_refunded as f64 / 100.0,
                    charge_amount as f64 / 100.0, payment_id,
                ),
            }).await;
            return Ok(());
        }

        if current_status == "Refunded" {
            // Echo from our own admin-button refund. Already handled.
            tracing::debug!("charge.refunded echo for already-Refunded payment {}", payment_id);
            return Ok(());
        }

        // Out-of-band full refund. Flip to Refunded.
        sqlx::query(
            "UPDATE payments SET status = 'Refunded', updated_at = ? WHERE id = ?",
        )
        .bind(Utc::now().naive_utc())
        .bind(&payment_id)
        .execute(&self.db_pool)
        .await
        .map_err(|e| AppError::Internal(format!("Database error: {}", e)))?;

        tracing::info!(
            "Synced refund from Stripe dashboard: payment {} marked Refunded (charge {})",
            payment_id, charge.id,
        );

        Ok(())
    }

    /// Handle invoice.paid - extend member dues for subscription payments
    async fn handle_invoice_paid(&self, invoice: stripe::Invoice) -> Result<()> {
        // Only care about subscription invoices
        let subscription_id = match &invoice.subscription {
            Some(sub) => sub.id().to_string(),
            None => return Ok(()),
        };

        // Reject non-USD invoices at the boundary. Coterie treats
        // amount_cents as USD throughout dues math, totals, refund
        // display, and admin UI; a single misconfigured Stripe Price
        // in another currency would silently corrupt all of that.
        // This guard fails loud and dispatches an AdminAlert so an
        // operator can fix the Price config before more invoices land.
        let currency_str = invoice.currency
            .map(|c| c.to_string().to_lowercase())
            .unwrap_or_default();
        if !currency_str.is_empty() && currency_str != "usd" {
            tracing::error!(
                "Invoice {} arrived in non-USD currency '{}'; refusing to process",
                invoice.id, currency_str,
            );
            self.integration_manager.handle_event(IntegrationEvent::AdminAlert {
                subject: format!("Non-USD Stripe invoice received ({})", currency_str.to_uppercase()),
                body: format!(
                    "Invoice {} for subscription {} arrived in '{}' (not USD). \
                     Coterie's payment math assumes USD throughout — the invoice \
                     was NOT recorded locally and dues were NOT extended. \
                     Check the Stripe Price config; once fixed, manually \
                     reconcile this member's dues.",
                    invoice.id, subscription_id, currency_str.to_uppercase(),
                ),
            }).await;
            return Ok(());
        }

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
        let payment_id = Uuid::new_v4();
        let payment = Payment {
            id: payment_id,
            member_id: Some(member_uuid),
            amount_cents,
            currency: invoice.currency.map(|c| c.to_string()).unwrap_or_else(|| "usd".to_string()),
            status: PaymentStatus::Completed,
            payment_method: PaymentMethod::Stripe,
            stripe_payment_id: Some(invoice.id.to_string()),
            description: format!("Subscription payment ({})", subscription_id),
            payment_type: PaymentType::Membership,
            donation_campaign_id: None,
            donor_name: None,
            donor_email: None,
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
            self.extend_member_dues(payment_id, member_uuid, &slug).await?;
        } else {
            // Fallback: extend by 1 month (conservative default for subscriptions
            // we couldn't map to a membership type). Routes through the
            // atomic per-payment claim so a webhook retry won't double-extend.
            self.payment_repo
                .extend_dues_for_payment_atomic(payment_id, member_uuid, BillingPeriod::Monthly)
                .await?;
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
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("member_id".to_string(), member_id.to_string());

        let customer_id = self.gateway.create_customer(CreateCustomerInput {
            email: email.to_string(),
            name: Some(name.to_string()),
            metadata,
        }).await?;

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

        let mut metadata = std::collections::HashMap::new();
        metadata.insert("member_id".to_string(), member_id.to_string());

        let out = self.gateway.create_setup_intent(CreateSetupIntentInput {
            customer_id,
            metadata,
        }).await?;

        Ok(out.client_secret)
    }

    /// Charge a saved payment method (card).
    ///
    /// `idempotency_key` must be stable across retries of the same logical
    /// payment attempt. If the user double-clicks "Pay", both requests should
    /// pass the same key so Stripe returns the cached response and the card
    /// is only charged once. Callers typically generate this at form-render
    /// time (UUID in a hidden field) and thread it through.
    ///
    /// `payment_id` is the Coterie-side Payment row ID (already
    /// inserted as Pending by the caller). It rides on PI metadata so
    /// the `payment_intent.succeeded` webhook can resolve the local
    /// row even if the synchronous response is lost — closing the
    /// "charged but no record" silent-loss case.
    ///
    /// Returns the PaymentIntent ID if successful.
    pub async fn charge_saved_card(
        &self,
        member_id: Uuid,
        stripe_payment_method_id: &str,
        amount_cents: i64,
        description: &str,
        idempotency_key: &str,
        payment_id: Uuid,
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

        let mut metadata = std::collections::HashMap::new();
        metadata.insert("member_id".to_string(), member_id.to_string());
        metadata.insert("payment_id".to_string(), payment_id.to_string());

        let result = self.gateway.create_payment_intent(CreatePaymentIntentInput {
            amount_cents,
            customer_id,
            payment_method_id: stripe_payment_method_id.to_string(),
            description: description.to_string(),
            metadata,
            idempotency_key: idempotency_key.to_string(),
        }).await?;

        match result {
            PaymentIntentResult::Succeeded { id } => {
                tracing::info!("Successfully charged {} for member {}", amount_cents, member_id);
                Ok(id)
            }
            PaymentIntentResult::RequiresAction { .. } => {
                Err(AppError::External("Payment requires additional authentication".to_string()))
            }
            PaymentIntentResult::Other { status, .. } => {
                Err(AppError::External(format!("Payment failed with status: {}", status)))
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
        // Retrieve the customer once so we can identify which PM is the
        // invoice-default; the gateway's list call returns the cards
        // themselves.
        let customer = self.gateway.retrieve_customer(customer_id).await?;
        let default_pm_id = customer.default_payment_method_id;

        let pms = self.gateway.list_payment_methods(customer_id).await?;

        let mut out: Vec<CustomerCardSummary> = pms.into_iter().map(|pm| {
            CustomerCardSummary {
                is_default: default_pm_id.as_deref() == Some(pm.id.as_str()),
                payment_method_id: pm.id,
                last_four: pm.last4,
                brand: pm.brand,
                exp_month: pm.exp_month,
                exp_year: pm.exp_year,
            }
        }).collect();

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

    /// Issue a full refund for a stored Stripe payment ID.
    ///
    /// We store one of three things in `payments.stripe_payment_id`
    /// depending on which flow created the payment:
    ///   - `pi_…`  PaymentIntent (saved-card charges, subscription
    ///             payments processed by Coterie's billing runner)
    ///   - `cs_…`  CheckoutSession (one-time membership/donation
    ///             checkouts)
    ///   - `in_…`  Invoice (Stripe-managed subscription invoices)
    ///
    /// Stripe's Refund API takes a PaymentIntent or Charge; this
    /// method normalizes whichever shape we have to a PaymentIntent.
    /// Full refund only — partial refunds aren't exposed in the
    /// admin UI (would complicate dues / campaign accounting).
    ///
    /// `idempotency_key` must be stable per logical refund attempt
    /// (callers pass the Coterie payment_id). Without it, a double-
    /// click on the admin Refund button or a transient retry could
    /// cause Stripe to issue two refunds.
    pub async fn refund_payment(
        &self,
        stored_stripe_id: &str,
        idempotency_key: &str,
    ) -> Result<String> {
        let payment_intent_id: String = if stored_stripe_id.starts_with("pi_") {
            stored_stripe_id.to_string()
        } else if stored_stripe_id.starts_with("cs_") {
            let session = self.gateway.retrieve_checkout_session(stored_stripe_id).await?;
            session.payment_intent_id.ok_or_else(|| AppError::BadRequest(
                "Checkout session has no PaymentIntent — not a charge that can be refunded".to_string()
            ))?
        } else if stored_stripe_id.starts_with("in_") {
            let invoice = self.gateway.retrieve_invoice(stored_stripe_id).await?;
            invoice.payment_intent_id.ok_or_else(|| AppError::BadRequest(
                "Invoice has no PaymentIntent — not a charge that can be refunded".to_string()
            ))?
        } else {
            return Err(AppError::BadRequest(format!(
                "Unrecognized Stripe ID format '{}': expected pi_, cs_, or in_ prefix",
                stored_stripe_id,
            )));
        };

        let refund = self.gateway.create_refund(CreateRefundInput {
            payment_intent_id: payment_intent_id.clone(),
            idempotency_key: idempotency_key.to_string(),
        }).await?;

        tracing::info!(
            "Refunded PaymentIntent {} → Refund {}",
            payment_intent_id, refund.id,
        );
        Ok(refund.id)
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
        self.gateway.delete_subscription(subscription_id).await?;
        tracing::info!("Cancelled Stripe subscription {}", subscription_id);
        Ok(())
    }

    /// Detach a PaymentMethod from its Stripe Customer. Coterie's
    /// "delete saved card" handlers should call this after removing
    /// the local row so the card doesn't continue to live on Stripe
    /// indefinitely (compliance / trust). Idempotent — Stripe returns
    /// success on an already-detached PM, error mapped to External.
    pub async fn detach_payment_method(&self, payment_method_id: &str) -> Result<()> {
        self.gateway.detach_payment_method(payment_method_id).await
    }

    /// Retrieve card details from a Stripe PaymentMethod, including
    /// the Stripe customer ID it's currently attached to (or `None`
    /// for un-attached PMs). Callers about to persist a `pm_…` ID
    /// against a member MUST verify `customer_id` matches the
    /// member's `stripe_customer_id` — otherwise a member who learns
    /// another's PM ID could attach it to their own saved-cards UI
    /// and learn the brand + last four (info disclosure).
    pub async fn get_payment_method_details(
        &self,
        payment_method_id: &str,
    ) -> Result<CardDetails> {
        let pm = self.gateway.retrieve_payment_method(payment_method_id).await?;
        Ok(CardDetails {
            last_four: pm.last4,
            brand: pm.brand,
            exp_month: pm.exp_month,
            exp_year: pm.exp_year,
            customer_id: pm.customer_id,
        })
    }
}

/// Card details retrieved from Stripe
pub struct CardDetails {
    pub last_four: String,
    pub brand: String,
    pub exp_month: i32,
    pub exp_year: i32,
    /// The Stripe Customer this PM is attached to, if any. Compare
    /// against the local member's `stripe_customer_id` before
    /// persisting to prevent cross-member PM stapling.
    pub customer_id: Option<String>,
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