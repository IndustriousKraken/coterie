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

mod charge;
mod checkout;
mod invoice;
mod payment_intent;
mod subscription;

use std::sync::Arc;
use stripe::{CheckoutSession, EventObject, EventType, Webhook, WebhookError};

use crate::{
    error::{AppError, Result},
    integrations::IntegrationManager,
    payments::gateway::StripeGateway,
    repository::{MemberRepository, PaymentRepository, ProcessedEventsRepository},
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
    processed_events_repo: Arc<dyn ProcessedEventsRepository>,
    membership_type_service: Arc<MembershipTypeService>,
    integration_manager: Arc<IntegrationManager>,
}

impl WebhookDispatcher {
    pub fn new(
        gateway: Arc<dyn StripeGateway>,
        webhook_secret: String,
        payment_repo: Arc<dyn PaymentRepository>,
        member_repo: Arc<dyn MemberRepository>,
        processed_events_repo: Arc<dyn ProcessedEventsRepository>,
        membership_type_service: Arc<MembershipTypeService>,
        integration_manager: Arc<IntegrationManager>,
    ) -> Self {
        Self {
            gateway,
            webhook_secret,
            payment_repo,
            member_repo,
            processed_events_repo,
            membership_type_service,
            integration_manager,
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
        let event = Webhook::construct_event(payload, stripe_signature, &self.webhook_secret)
            .map_err(|e| match e {
                WebhookError::BadSignature => AppError::BadRequest("Invalid signature".to_string()),
                WebhookError::BadTimestamp(skew_secs) => AppError::BadRequest(format!(
                    "Webhook timestamp out of tolerance (skew: {}s) — clock drift",
                    skew_secs,
                )),
                _ => AppError::External(format!("Webhook error: {}", e)),
            })?;

        // Idempotency: claim the event ID atomically. If another worker
        // or a retry already processed this event, `claim` returns
        // false and we bail early. Without this, Stripe's "at-least-
        // once" delivery would let retries double-extend dues.
        let event_id = event.id.to_string();
        let event_type = format!("{:?}", event.type_);
        let claimed = self
            .processed_events_repo
            .claim(&event_id, &event_type)
            .await?;
        if !claimed {
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
                        self.handle_successful_payment(session, billing_service)
                            .await?;
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
                        self.handle_payment_intent_succeeded(intent, billing_service)
                            .await?;
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
                        self.handle_invoice_payment_failed(invoice, billing_service)
                            .await?;
                    }
                }
                EventType::CustomerSubscriptionDeleted => {
                    if let EventObject::Subscription(subscription) = event.data.object {
                        self.handle_subscription_deleted(subscription, billing_service)
                            .await?;
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
            if let Err(rollback_err) = self.processed_events_repo.release(&event_id).await {
                tracing::error!(
                    "Idempotency rollback failed for event {}: {}. The event \
                     is permanently claimed; manual intervention may be needed.",
                    event_id,
                    rollback_err,
                );
            }
        }

        outcome
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
        self.handle_payment_intent_succeeded(intent, billing_service)
            .await
    }

    pub async fn dispatch_charge_refunded(&self, charge: stripe::Charge) -> Result<()> {
        self.handle_charge_refunded(charge).await
    }

    pub async fn dispatch_subscription_deleted(
        &self,
        subscription: stripe::Subscription,
        billing_service: &BillingService,
    ) -> Result<()> {
        self.handle_subscription_deleted(subscription, billing_service)
            .await
    }

    pub async fn dispatch_checkout_session_completed(
        &self,
        session: CheckoutSession,
        billing_service: &BillingService,
    ) -> Result<()> {
        self.handle_successful_payment(session, billing_service)
            .await
    }

    pub async fn dispatch_invoice_paid(
        &self,
        invoice: stripe::Invoice,
        billing_service: &BillingService,
    ) -> Result<()> {
        self.handle_invoice_paid(invoice, billing_service).await
    }

    pub async fn dispatch_invoice_payment_failed(
        &self,
        invoice: stripe::Invoice,
        billing_service: &BillingService,
    ) -> Result<()> {
        self.handle_invoice_payment_failed(invoice, billing_service)
            .await
    }
}
