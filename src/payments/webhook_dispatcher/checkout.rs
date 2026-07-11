use chrono::Utc;
use stripe::CheckoutSession;

use crate::{
    domain::{Payer, PaymentKind, PaymentStatus},
    error::{AppError, Result},
    integrations::IntegrationEvent,
    service::billing_service::BillingService,
};

use super::WebhookDispatcher;

impl WebhookDispatcher {
    pub(super) async fn handle_successful_payment(
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

        // Upgrade stripe_payment_id from cs_ → pi_ on the flip so future
        // refund webhooks (charge.refunded carries a PaymentIntent ID,
        // not a CheckoutSession ID) can match this row by IN-clause.
        // Fall back to keeping cs_ if Stripe didn't expand the PI on
        // the session (rare, but defensive).
        let pi_for_row = session
            .payment_intent
            .as_ref()
            .map(|exp| exp.id().to_string())
            .unwrap_or_else(|| session_id.clone());

        // Branch on payment type. Donations don't extend dues and
        // don't refresh auto-renew schedules — they're a separate
        // bucket from membership renewal. Reading from session
        // metadata avoids a second DB lookup; falling back to the
        // payment row's stored type covers older sessions that
        // didn't write the metadata key (i.e. created before this
        // code shipped). Resolved BEFORE any flip — it only reads
        // session.metadata + payment.kind, so it doesn't depend on it.
        let payment_type_str = session
            .metadata
            .as_ref()
            .and_then(|m| m.get("payment_type"))
            .cloned()
            .unwrap_or_else(|| payment.kind.as_str().to_string());

        if payment_type_str == "donation" {
            // Donations have no dues post-work, so flip-first is correct
            // and cannot strand anything. complete_pending_payment only
            // flips a Pending row and reports whether *we* did it; a
            // retry/loser of the sync race sees false and bails.
            let won_flip = self
                .payment_repo
                .complete_pending_payment(payment.id, &pi_for_row)
                .await?;
            if !won_flip {
                tracing::debug!(
                    "checkout.session.completed donation for payment {} already Completed; \
                     skipping (Stripe retry or sync path won the race)",
                    payment.id,
                );
                return Ok(());
            }

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
                payment.id,
                payment.member_id(),
                donor_label,
                campaign_id,
                payment.amount_cents,
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
                return Err(AppError::Internal(
                    "membership payment without member payer".to_string(),
                ));
            }
        };

        // Look up the slug from metadata. Resolved BEFORE any flip so
        // the dues-extend below runs first.
        let membership_type_slug = session
            .metadata
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
            match member {
                Some(m) => self
                    .membership_type_service
                    .get(m.membership_type_id)
                    .await?
                    .map(|mt| mt.slug),
                None => None,
            }
        };

        if let Some(slug) = &resolved_slug {
            // Extend dues FIRST. extend_member_dues_by_slug is an
            // idempotent, atomic per-payment claim (dues_extended_at),
            // so it's safe to run before the flip and to let Stripe's
            // retry re-run it. If it errors transiently, the `?` returns
            // Err BEFORE the flip below — leaving the row Pending so the
            // dispatcher's claim release lets the next retry recover it.
            // The irreversible Completed flip must be the LAST
            // must-succeed step, or a transient extend failure strands
            // a paid-but-unextended member that the retry can't fix.
            billing_service
                .auto_renew
                .extend_member_dues_by_slug(payment.id, member_id, slug)
                .await?;

            // Flip only after a successful extend.
            let won_flip = self
                .payment_repo
                .complete_pending_payment(payment.id, &pi_for_row)
                .await?;

            // Reschedule only when we actually flipped — so only the
            // caller that won the sync/webhook race reschedules, avoiding
            // double cancel/queue churn. Soft-failed (log, don't
            // propagate): a reschedule failure must not roll back a
            // completed payment.
            if won_flip {
                if let Err(e) = billing_service
                    .auto_renew
                    .reschedule_after_payment(member_id, slug)
                    .await
                {
                    tracing::error!(
                        "Member {} paid via Checkout but reschedule failed: {}",
                        member_id,
                        e,
                    );
                }

                // Pay-at-signup auto-renew enrollment: the session was
                // created with save_card=true metadata (setting on at
                // creation time), so save the paying card and enable
                // Coterie-managed renewal. Soft-failed: the member is
                // already Active with dues extended — enrollment
                // problems are logged, never fail the webhook.
                let save_card = session
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("save_card"))
                    .map(|v| v == "true")
                    .unwrap_or(false);
                if save_card {
                    match session.customer.as_ref().map(|c| c.id().to_string()) {
                        Some(customer_id) => {
                            if let Err(e) = billing_service
                                .auto_renew
                                .enroll_after_signup_payment(member_id, slug, &customer_id)
                                .await
                            {
                                tracing::error!(
                                    "Signup auto-renew enrollment failed for member {} \
                                     (payment stands, member is Active): {}",
                                    member_id,
                                    e,
                                );
                            }
                        }
                        None => tracing::error!(
                            "save_card session {} carries no customer; cannot enroll \
                             member {} in auto-renew",
                            session_id,
                            member_id,
                        ),
                    }
                }
            }
        } else {
            // Slug unresolvable: this path deliberately gives up on
            // automatic extension and alerts an operator, so the flip
            // stays terminal here. complete_pending_payment is
            // conditional on Pending, so a retry that finds the row
            // already Completed skips re-alerting.
            let won_flip = self
                .payment_repo
                .complete_pending_payment(payment.id, &pi_for_row)
                .await?;
            if !won_flip {
                tracing::debug!(
                    "checkout.session.completed for payment {} already Completed; \
                     skipping AdminAlert re-dispatch (likely a Stripe retry)",
                    payment.id,
                );
                return Ok(());
            }
            tracing::error!(
                "Couldn't resolve membership type for paid Checkout session {}; \
                 dues NOT extended for member {} — operator must reconcile",
                session_id,
                member_id,
            );
            self.integration_manager
                .handle_event(IntegrationEvent::AdminAlert {
                    subject: format!("Checkout paid but dues not extended — member {}", member_id,),
                    body: format!(
                        "Checkout session {} (payment {}) was paid by member {} \
                     but no membership type could be resolved (no metadata, \
                     no current type on record). Dues were NOT extended. \
                     Reconcile manually.",
                        session_id, payment.id, member_id,
                    ),
                })
                .await;
        }

        tracing::info!("Payment completed for member: {}", member_id);
        Ok(())
    }

    pub(super) async fn handle_expired_session(&self, session: CheckoutSession) -> Result<()> {
        let session_id = session.id.to_string();

        if let Some(mut payment) = self.payment_repo.find_by_stripe_id(&session_id).await? {
            payment.status = PaymentStatus::Failed;
            payment.updated_at = Utc::now();

            self.payment_repo.update(payment.id, payment).await?;

            tracing::info!("Checkout session expired: {}", session_id);
        }

        Ok(())
    }
}
