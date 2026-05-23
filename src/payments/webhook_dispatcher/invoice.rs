use chrono::Utc;
use uuid::Uuid;

use crate::{
    domain::{
        configurable_types::BillingPeriod, Payer, Payment, PaymentKind, PaymentMethod,
        PaymentStatus, StripeRef,
    },
    error::Result,
    integrations::IntegrationEvent,
    service::billing_service::BillingService,
};

use super::WebhookDispatcher;

impl WebhookDispatcher {
    /// Handle invoice.paid - extend member dues for subscription payments
    pub(super) async fn handle_invoice_paid(
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
        let currency_str = invoice
            .currency
            .map(|c| c.to_string().to_lowercase())
            .unwrap_or_default();
        if !currency_str.is_empty() && currency_str != "usd" {
            tracing::error!(
                "Invoice {} arrived in non-USD currency '{}'; refusing to process",
                invoice.id,
                currency_str,
            );
            self.integration_manager
                .handle_event(IntegrationEvent::AdminAlert {
                    subject: format!(
                        "Non-USD Stripe invoice received ({})",
                        currency_str.to_uppercase()
                    ),
                    body: format!(
                        "Invoice {} for subscription {} arrived in '{}' (not USD). \
                     Coterie's payment math assumes USD throughout — the invoice \
                     was NOT recorded locally and dues were NOT extended. \
                     Check the Stripe Price config; once fixed, manually \
                     reconcile this member's dues.",
                        invoice.id,
                        subscription_id,
                        currency_str.to_uppercase(),
                    ),
                })
                .await;
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
        let member = match self
            .member_repo
            .find_by_stripe_customer_id(&customer_id)
            .await?
        {
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
            currency: invoice
                .currency
                .map(|c| c.to_string())
                .unwrap_or_else(|| "usd".to_string()),
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
        let membership_type_slug = self
            .membership_type_service
            .get(member.membership_type_id)
            .await?
            .map(|mt| mt.slug);

        if let Some(slug) = membership_type_slug {
            billing_service
                .auto_renew
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
            member_uuid,
            subscription_id
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
    pub(super) async fn handle_invoice_payment_failed(
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
        let subscription_id = invoice
            .subscription
            .as_ref()
            .map(|s| s.id().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        // Find the member behind this Stripe customer.
        let member_id = match self
            .member_repo
            .find_by_stripe_customer_id(&customer_id)
            .await?
        {
            Some(m) => m.id,
            None => {
                tracing::warn!(
                    "invoice.payment_failed for unknown Stripe customer {} (subscription: {})",
                    customer_id,
                    subscription_id,
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
        let amount_cents = invoice.amount_due.or(invoice.amount_remaining).unwrap_or(0);
        let amount_display = if amount_cents > 0 {
            Some(format!("${:.2}", amount_cents as f64 / 100.0))
        } else {
            None
        };

        if let Err(e) = billing_service
            .notifications
            .notify_subscription_payment_failed(member_id, amount_display, is_final)
            .await
        {
            tracing::error!(
                "Couldn't notify member {} of failed subscription charge: {}",
                member_id,
                e,
            );
        }

        Ok(())
    }
}
