//! Inbound Stripe webhook dispatcher.
//!
//! `StripeClient` is responsible for outbound calls to Stripe. This
//! module is responsible for what happens when Stripe calls *us*:
//! signature verification, idempotency claim against
//! `processed_stripe_events`, and dispatch to per-event-type handlers.
//!
//! The split mirrors the gateway seam in `payments::gateway`: by
//! keeping inbound and outbound code in different files, anyone
//! reading either side has a single concern in front of them.
//!
//! Test seams are exposed under `#[cfg(any(test, feature = "test-utils"))]`
//! at the bottom. They wrap the private `handle_*` methods so tests can
//! exercise post-dispatch logic without constructing signed payloads.

use stripe::{CheckoutSession, EventObject, EventType, Webhook, WebhookError};
use chrono::Utc;
use uuid::Uuid;
use std::sync::Arc;
use sqlx::SqlitePool;

use crate::{
    domain::{BillingMode, Payer, Payment, PaymentKind, PaymentMethod, PaymentStatus, StripeRef, configurable_types::BillingPeriod},
    error::{AppError, Result},
    integrations::{IntegrationEvent, IntegrationManager},
    payments::gateway::StripeGateway,
    repository::{MemberRepository, PaymentRepository},
    service::{billing_service::BillingService, membership_type_service::MembershipTypeService},
};

pub struct WebhookDispatcher {
    /// Used by `handle_charge_refunded` to walk back from a PaymentIntent
    /// to the originating CheckoutSession when our local row is keyed by
    /// `cs_` (legacy). Outbound calls live in `StripeClient`; this is
    /// the dispatcher's only outbound dependency.
    gateway: Arc<dyn StripeGateway>,
    webhook_secret: String,
    payment_repo: Arc<dyn PaymentRepository>,
    member_repo: Arc<dyn MemberRepository>,
    membership_type_service: Arc<MembershipTypeService>,
    integration_manager: Arc<IntegrationManager>,
    /// Held for the `processed_stripe_events` idempotency claim and the
    /// legacy charge.refunded UPDATE on the payments table. Both writes
    /// are scoped to processed-events / payments tables that don't yet
    /// have repo methods carved out for them — those would be a small
    /// follow-up to lift the dispatcher fully off `db_pool`.
    db_pool: SqlitePool,
}

impl WebhookDispatcher {
    pub fn new(
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
                        self.handle_invoice_paid(invoice, billing_service).await?;
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

        let payment = match self.payment_repo.find_by_stripe_id(&session_id).await? {
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
            .unwrap_or_else(|| payment.kind.as_str().to_string());

        if payment_type_str == "donation" {
            let (donor_label, campaign_id) = match (&payment.payer, &payment.kind) {
                (Payer::PublicDonor { email, .. }, PaymentKind::Donation { campaign_id }) => {
                    (format!("public:{}", email), *campaign_id)
                }
                (Payer::Member(_), PaymentKind::Donation { campaign_id }) => {
                    ("member".to_string(), *campaign_id)
                }
                _ => ("?".to_string(), payment.kind.campaign_id()),
            };
            tracing::info!(
                "Donation completed: payment={} payer={:?} donor={} campaign={:?} amount={}",
                payment.id, payment.member_id(), donor_label,
                campaign_id, payment.amount_cents,
            );
            return Ok(());
        }

        // Membership flow. A membership Checkout session was always
        // created with a real member_id (see create_membership_checkout_session)
        // so the payer is invariably Member. Bail loudly if not —
        // it's a data-integrity violation and we don't want to silently
        // run the rest of the flow with a fabricated UUID.
        let member_id = match payment.member_id() {
            Some(id) => id,
            None => {
                tracing::error!(
                    "Membership payment {} has no member payer — data integrity violation",
                    payment.id,
                );
                return Err(AppError::Database(
                    "membership payment without member payer".to_string()
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
            billing_service
                .extend_member_dues_by_slug(payment.id, member_id, slug)
                .await?;

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
        // (Payer::PublicDonor) skip the member equality check —
        // the PI metadata for those carries donor_email instead.
        let metadata_member_id = intent.metadata.get("member_id")
            .and_then(|s| Uuid::parse_str(s).ok());
        let row_member_id = payment.member_id();
        if row_member_id.is_some() && metadata_member_id != row_member_id {
            tracing::warn!(
                "PI {} payment_id metadata points at payment {} (member {:?}), \
                 but PI's member_id metadata is {:?}; refusing to act",
                intent.id, payment_id, row_member_id, metadata_member_id,
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
            "Self-healing payment {} via PI.succeeded webhook (payer: {:?})",
            payment_id, payment.payer,
        );

        // Post-work depends on payment kind. Donations have none —
        // the row flip is the entire job. Membership payments need
        // dues extended and (if auto-renew enrolled) the next renewal
        // rescheduled. We look up the slug from the member's current
        // membership_type since saved-card charges don't carry it on
        // the Payment row.
        if matches!(payment.kind, PaymentKind::Membership) {
            // Membership payments must have a member; data integrity
            // violation otherwise (CHECK constraint should prevent it).
            let member_id = match payment.member_id() {
                Some(id) => id,
                None => {
                    tracing::error!(
                        "Membership payment {} has no member payer — data integrity violation",
                        payment_id,
                    );
                    return Err(AppError::Database(
                        "membership payment without member payer".to_string()
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
            billing_service
                .extend_member_dues_by_slug(payment_id, member_id, &slug)
                .await?;
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
    async fn handle_invoice_paid(
        &self,
        invoice: stripe::Invoice,
        billing_service: &BillingService,
    ) -> Result<()> {
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
        let member = match self.member_repo.find_by_stripe_customer_id(&customer_id).await? {
            Some(m) => m,
            None => {
                tracing::debug!("No member found for Stripe customer {}", customer_id);
                return Ok(());
            }
        };
        let member_uuid = member.id;

        let amount_cents = invoice.amount_paid.unwrap_or(0);

        // Create payment record
        let payment_id = Uuid::new_v4();
        let payment = Payment {
            id: payment_id,
            payer: Payer::Member(member_uuid),
            amount_cents,
            currency: invoice.currency.map(|c| c.to_string()).unwrap_or_else(|| "usd".to_string()),
            status: PaymentStatus::Completed,
            payment_method: PaymentMethod::Stripe,
            external_id: Some(StripeRef::Invoice(invoice.id.to_string())),
            description: format!("Subscription payment ({})", subscription_id),
            kind: PaymentKind::Membership,
            paid_at: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.payment_repo.create(payment).await?;

        // Extend dues - look up membership type from member's current type
        let membership_type_slug = match member.membership_type_id {
            Some(mt_id) => self.membership_type_service.get(mt_id).await?
                .map(|mt| mt.slug),
            None => None,
        };

        if let Some(slug) = membership_type_slug {
            billing_service
                .extend_member_dues_by_slug(payment_id, member_uuid, &slug)
                .await?;
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
            member_uuid, subscription_id
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
        let member_id = match self.member_repo.find_by_stripe_customer_id(&customer_id).await? {
            Some(m) => m.id,
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
        let member = match self.member_repo.find_by_stripe_customer_id(&customer_id).await? {
            Some(m) => m,
            None => {
                tracing::debug!(
                    "subscription.deleted for customer {} — no matching member",
                    customer_id,
                );
                return Ok(());
            }
        };

        if member.billing_mode != BillingMode::StripeSubscription {
            // Echo from our own migration; nothing to do.
            tracing::debug!(
                "subscription.deleted echo for migrated customer {} (mode={:?}); ignoring",
                customer_id, member.billing_mode,
            );
            return Ok(());
        }

        // Real out-of-band cancellation. Flip + notify.
        self.member_repo
            .set_billing_mode(member.id, BillingMode::Manual, None)
            .await?;

        tracing::info!(
            "Subscription cancelled out-of-band for customer {}; switched member to manual",
            customer_id,
        );

        if let Err(e) = billing_service
            .notify_subscription_cancelled(member.id)
            .await
        {
            tracing::error!(
                "Switched member {} to manual but notification failed: {}",
                member.id, e,
            );
        }

        Ok(())
    }

    /// Handle customer.subscription.updated - update cached subscription info
    async fn handle_subscription_updated(&self, subscription: stripe::Subscription) -> Result<()> {
        let customer_id = subscription.customer.id().to_string();
        let subscription_id = subscription.id.to_string();
        let status = format!("{:?}", subscription.status);

        // Update the subscription ID in case it changed. No-op if the
        // customer doesn't map to a Coterie member (we just don't track them).
        if let Some(member) = self.member_repo
            .find_by_stripe_customer_id(&customer_id).await?
        {
            self.member_repo
                .set_billing_mode(member.id, member.billing_mode, Some(&subscription_id))
                .await?;
        }

        tracing::debug!(
            "Subscription {} updated for customer {} (status: {})",
            subscription_id, customer_id, status
        );

        Ok(())
    }
}

/// Test-only access to the per-event handlers that `handle_webhook`
/// dispatches to. Production code must go through `handle_webhook` so
/// that signature verification and the event-id idempotency claim
/// happen first; tests target the handlers directly to focus on the
/// post-dispatch logic without having to construct signed payloads.
#[cfg(any(test, feature = "test-utils"))]
impl WebhookDispatcher {
    pub async fn dispatch_payment_intent_succeeded(
        &self,
        intent: stripe::PaymentIntent,
        billing_service: &BillingService,
    ) -> Result<()> {
        self.handle_payment_intent_succeeded(intent, billing_service).await
    }

    pub async fn dispatch_charge_refunded(&self, charge: stripe::Charge) -> Result<()> {
        self.handle_charge_refunded(charge).await
    }

    pub async fn dispatch_subscription_deleted(
        &self,
        subscription: stripe::Subscription,
        billing_service: &BillingService,
    ) -> Result<()> {
        self.handle_subscription_deleted(subscription, billing_service).await
    }

    pub async fn dispatch_checkout_session_completed(
        &self,
        session: CheckoutSession,
        billing_service: &BillingService,
    ) -> Result<()> {
        self.handle_successful_payment(session, billing_service).await
    }
}
