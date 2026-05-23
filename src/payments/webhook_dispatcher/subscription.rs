use crate::{domain::BillingMode, error::Result, service::billing_service::BillingService};

use super::WebhookDispatcher;

impl WebhookDispatcher {
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
    pub(super) async fn handle_subscription_deleted(
        &self,
        subscription: stripe::Subscription,
        billing_service: &BillingService,
    ) -> Result<()> {
        let customer_id = subscription.customer.id().to_string();

        // Look up state BEFORE writing — we need to distinguish the
        // two cases above by reading billing_mode.
        let member = match self
            .member_repo
            .find_by_stripe_customer_id(&customer_id)
            .await?
        {
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
                customer_id,
                member.billing_mode,
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
            .notifications
            .notify_subscription_cancelled(member.id)
            .await
        {
            tracing::error!(
                "Switched member {} to manual but notification failed: {}",
                member.id,
                e,
            );
        }

        Ok(())
    }

    /// Handle customer.subscription.updated - update cached subscription info
    pub(super) async fn handle_subscription_updated(
        &self,
        subscription: stripe::Subscription,
    ) -> Result<()> {
        let customer_id = subscription.customer.id().to_string();
        let subscription_id = subscription.id.to_string();
        let status = format!("{:?}", subscription.status);

        // Update the subscription ID in case it changed. No-op if the
        // customer doesn't map to a Coterie member (we just don't track them).
        if let Some(member) = self
            .member_repo
            .find_by_stripe_customer_id(&customer_id)
            .await?
        {
            self.member_repo
                .set_billing_mode(member.id, member.billing_mode, Some(&subscription_id))
                .await?;
        }

        tracing::debug!(
            "Subscription {} updated for customer {} (status: {})",
            subscription_id,
            customer_id,
            status
        );

        Ok(())
    }
}
