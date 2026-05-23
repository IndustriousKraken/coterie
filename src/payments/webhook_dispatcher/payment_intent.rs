use chrono::Utc;
use uuid::Uuid;

use crate::{
    domain::{PaymentKind, PaymentStatus},
    error::{AppError, Result},
    service::billing_service::BillingService,
};

use super::WebhookDispatcher;

impl WebhookDispatcher {
    pub(super) async fn handle_failed_payment(&self, stripe_payment_id: String) -> Result<()> {
        if let Some(mut payment) = self
            .payment_repo
            .find_by_stripe_id(&stripe_payment_id)
            .await?
        {
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
    pub(super) async fn handle_payment_intent_succeeded(
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
                    intent.id,
                    payment_id_str,
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
                    intent.id,
                    payment_id,
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
        let metadata_member_id = intent
            .metadata
            .get("member_id")
            .and_then(|s| Uuid::parse_str(s).ok());
        let row_member_id = payment.member_id();
        if row_member_id.is_some() && metadata_member_id != row_member_id {
            tracing::warn!(
                "PI {} payment_id metadata points at payment {} (member {:?}), \
                 but PI's member_id metadata is {:?}; refusing to act",
                intent.id,
                payment_id,
                row_member_id,
                metadata_member_id,
            );
            return Ok(());
        }
        if intent.amount != payment.amount_cents {
            tracing::warn!(
                "PI {} amount {} doesn't match local payment {} amount {}; refusing to act",
                intent.id,
                intent.amount,
                payment_id,
                payment.amount_cents,
            );
            return Ok(());
        }

        let pi_id = intent.id.to_string();
        let won_flip = self
            .payment_repo
            .complete_pending_payment(payment_id, &pi_id)
            .await?;
        if !won_flip {
            tracing::debug!(
                "PI {} succeeded but payment {} was already completed; sync path won the race",
                intent.id,
                payment_id,
            );
            return Ok(());
        }

        tracing::info!(
            "Self-healing payment {} via PI.succeeded webhook (payer: {:?})",
            payment_id,
            payment.payer,
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
                    return Err(AppError::Internal(
                        "membership payment without member payer".to_string(),
                    ));
                }
            };
            let member = match self.member_repo.find_by_id(member_id).await? {
                Some(m) => m,
                None => {
                    tracing::warn!(
                        "Self-healed payment {} for missing member {}; skipping post-work",
                        payment_id,
                        member_id,
                    );
                    return Ok(());
                }
            };
            let mt_id = member.membership_type_id;
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
                .auto_renew
                .extend_member_dues_by_slug(payment_id, member_id, &slug)
                .await?;
            if let Err(e) = billing_service
                .auto_renew
                .reschedule_after_payment(member_id, &slug)
                .await
            {
                tracing::error!(
                    "Self-healed payment {} but reschedule failed: {}",
                    payment_id,
                    e,
                );
            }
        }

        Ok(())
    }
}
