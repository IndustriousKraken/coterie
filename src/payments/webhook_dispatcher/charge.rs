use crate::{
    domain::{Payment, PaymentStatus},
    error::Result,
    integrations::IntegrationEvent,
};

use super::WebhookDispatcher;

impl WebhookDispatcher {
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
    pub(super) async fn handle_charge_refunded(&self, charge: stripe::Charge) -> Result<()> {
        let charge_amount = charge.amount;
        let amount_refunded = charge.amount_refunded;
        let is_partial = amount_refunded < charge_amount;

        // Try to find the Payment row. New checkout flows upgrade
        // stripe_payment_id from cs_ → pi_ on completion (see
        // handle_successful_payment), so checking pi then invoice
        // covers saved-card (pi_) and Stripe-subscription (in_)
        // payments. Legacy rows still keyed by cs_ fall through to
        // the Stripe-API lookup below.
        let pi_id = charge.payment_intent.as_ref().map(|e| e.id().to_string());
        let invoice_id = charge.invoice.as_ref().map(|e| e.id().to_string());

        let mut payment: Option<Payment> = None;
        if let Some(ref id) = pi_id {
            payment = self.payment_repo.find_by_stripe_id(id).await?;
        }
        if payment.is_none() {
            if let Some(ref id) = invoice_id {
                payment = self.payment_repo.find_by_stripe_id(id).await?;
            }
        }

        // Fallback for legacy cs_ rows: ask Stripe which CheckoutSession
        // owns this PaymentIntent, then match. The list API filters
        // server-side, so this is a single round trip.
        if payment.is_none() {
            if let Some(pi) = pi_id.as_deref() {
                if let Ok(sessions) = self.gateway.list_checkout_sessions_by_intent(pi).await {
                    if let Some(cs_id) = sessions.first() {
                        payment = self.payment_repo.find_by_stripe_id(cs_id).await?;
                    }
                }
            }
        }

        let payment = match payment {
            Some(p) => p,
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
                payment.id,
                amount_refunded,
                charge_amount,
            );
            // We don't update the row — partial refunds would muddle
            // dues/campaign accounting. Surface to operators so they
            // can decide whether to mark Refunded manually.
            self.integration_manager
                .handle_event(IntegrationEvent::AdminAlert {
                    subject: format!(
                        "Partial Stripe refund — payment {} (${:.2} of ${:.2})",
                        payment.id,
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
                        charge.id,
                        amount_refunded as f64 / 100.0,
                        charge_amount as f64 / 100.0,
                        payment.id,
                    ),
                })
                .await;
            return Ok(());
        }

        if matches!(payment.status, PaymentStatus::Refunded) {
            // Echo from our own admin-button refund. Already handled.
            tracing::debug!(
                "charge.refunded echo for already-Refunded payment {}",
                payment.id
            );
            return Ok(());
        }

        // Out-of-band full refund. Flip to Refunded.
        self.payment_repo.mark_refunded(payment.id).await?;

        tracing::info!(
            "Synced refund from Stripe dashboard: payment {} marked Refunded (charge {})",
            payment.id,
            charge.id,
        );

        Ok(())
    }
}
