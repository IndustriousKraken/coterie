use chrono::Utc;
use uuid::Uuid;
use std::sync::Arc;

use crate::{
    domain::{Payer, Payment, PaymentKind, PaymentMethod, PaymentStatus, StripeRef},
    error::{AppError, Result},
    payments::gateway::{
        CreateCheckoutInput, CreateCustomerInput, CreatePaymentIntentInput,
        CreateRefundInput, CreateSetupIntentInput, LineItemInput,
        PaymentIntentResult, StripeGateway,
    },
    repository::{MemberRepository, PaymentRepository},
};

/// Outbound Stripe API operations. Inbound webhook handling lives in
/// [`crate::payments::webhook_dispatcher::WebhookDispatcher`]; the two
/// halves used to share this struct but were split so each file has a
/// single concern.
pub struct StripeClient {
    /// Trait-based seam over the Stripe API. Production wraps stripe-rs
    /// (`RealStripeGateway`); tests substitute a fake.
    gateway: Arc<dyn StripeGateway>,
    payment_repo: Arc<dyn PaymentRepository>,
    member_repo: Arc<dyn MemberRepository>,
}

impl StripeClient {
    /// Production constructor: builds a `RealStripeGateway` from the
    /// API key.
    pub fn new(
        api_key: String,
        payment_repo: Arc<dyn PaymentRepository>,
        member_repo: Arc<dyn MemberRepository>,
    ) -> Self {
        let gateway: Arc<dyn StripeGateway> =
            Arc::new(crate::payments::gateway::RealStripeGateway::new(api_key));
        Self::with_gateway(gateway, payment_repo, member_repo)
    }

    /// Test seam: build a `StripeClient` with an explicit gateway (e.g.
    /// `FakeStripeGateway`). Used by integration tests to exercise the
    /// application logic without making real Stripe calls.
    pub fn with_gateway(
        gateway: Arc<dyn StripeGateway>,
        payment_repo: Arc<dyn PaymentRepository>,
        member_repo: Arc<dyn MemberRepository>,
    ) -> Self {
        Self { gateway, payment_repo, member_repo }
    }

    /// Borrow the underlying gateway. Used to share the same gateway
    /// instance with the `WebhookDispatcher` (so production and test
    /// builds wire one trait object through both halves).
    pub fn gateway(&self) -> Arc<dyn StripeGateway> {
        self.gateway.clone()
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
            payer: Payer::Member(member_id),
            amount_cents,
            currency: "USD".to_string(),
            status: PaymentStatus::Pending,
            payment_method: PaymentMethod::Stripe,
            external_id: Some(StripeRef::CheckoutSession(session.session_id)),
            description: format!("{} Membership Payment", membership_type_name),
            kind: PaymentKind::Membership,
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
    /// recorded with `kind=Donation { campaign_id }` so totals
    /// attribute correctly even before the webhook flips the row to
    /// Completed.
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
            payer: Payer::Member(member_id),
            amount_cents,
            currency: "USD".to_string(),
            status: PaymentStatus::Pending,
            payment_method: PaymentMethod::Stripe,
            external_id: Some(StripeRef::CheckoutSession(session.session_id)),
            description: product_name,
            kind: PaymentKind::Donation { campaign_id },
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
            payer: Payer::PublicDonor {
                name: donor_name.to_string(),
                email: donor_email.to_string(),
            },
            amount_cents,
            currency: "USD".to_string(),
            status: PaymentStatus::Pending,
            payment_method: PaymentMethod::Stripe,
            external_id: Some(StripeRef::CheckoutSession(session.session_id)),
            description: format!("{} — {}", product_name, donor_name),
            kind: PaymentKind::Donation { campaign_id },
            paid_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        self.payment_repo.create(payment).await?;

        Ok((session.url, payment_id))
    }

    /// Get or create a Stripe Customer for a member
    pub async fn get_or_create_customer(
        &self,
        member_id: Uuid,
        email: &str,
        name: &str,
    ) -> Result<String> {
        // Check if member already has a stripe_customer_id
        if let Some(member) = self.member_repo.find_by_id(member_id).await? {
            if let Some(customer_id) = member.stripe_customer_id {
                return Ok(customer_id);
            }
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
        self.member_repo
            .set_stripe_customer_id(member_id, &customer_id)
            .await?;

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
        let customer_id = self.member_repo.find_by_id(member_id).await?
            .and_then(|m| m.stripe_customer_id)
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

    /// Issue a full refund for a stored Stripe payment reference.
    ///
    /// Stripe's Refund API takes a PaymentIntent or Charge; this
    /// method normalizes the three reference shapes Coterie holds
    /// (see [`StripeRef`]) to a PaymentIntent before calling Stripe.
    /// Full refund only — partial refunds aren't exposed in the
    /// admin UI (would complicate dues / campaign accounting).
    ///
    /// `idempotency_key` must be stable per logical refund attempt
    /// (callers pass the Coterie payment_id). Without it, a double-
    /// click on the admin Refund button or a transient retry could
    /// cause Stripe to issue two refunds.
    pub async fn refund_payment(
        &self,
        stripe_ref: &StripeRef,
        idempotency_key: &str,
    ) -> Result<String> {
        let payment_intent_id = match stripe_ref {
            StripeRef::PaymentIntent(id) => id.clone(),
            StripeRef::CheckoutSession(id) => {
                let session = self.gateway.retrieve_checkout_session(id).await?;
                session.payment_intent_id.ok_or_else(|| AppError::BadRequest(
                    "Checkout session has no PaymentIntent — not a charge that can be refunded".to_string()
                ))?
            }
            StripeRef::Invoice(id) => {
                let invoice = self.gateway.retrieve_invoice(id).await?;
                invoice.payment_intent_id.ok_or_else(|| AppError::BadRequest(
                    "Invoice has no PaymentIntent — not a charge that can be refunded".to_string()
                ))?
            }
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