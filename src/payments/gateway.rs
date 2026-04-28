//! Abstraction over the stripe-rs SDK.
//!
//! `StripeClient` owns application logic (Pending row creation, webhook
//! dispatch, dues extension, idempotency tracking) layered on top of
//! Stripe's REST API. To test that logic without making real network
//! calls, the SDK calls are routed through `StripeGateway` — a thin
//! trait whose only job is to do I/O against Stripe.
//!
//! Two implementations:
//!   - [`RealStripeGateway`] — production. Wraps a `stripe::Client`.
//!   - [`FakeStripeGateway`](crate::payments::fake_gateway) — tests.
//!     Records calls, returns canned responses.
//!
//! The trait deliberately uses Coterie-side input/output structs rather
//! than stripe-rs types directly, so:
//!   - Tests don't have to construct stripe-rs `CreatePaymentIntent`
//!     with its lifetime-laden parameter trees.
//!   - The application code's call sites are easier to read (each
//!     method describes intent — "charge this card for this amount" —
//!     rather than dialect-specific Stripe parameters).
//!
//! Webhook signature verification is intentionally NOT in the trait.
//! Tests construct `stripe::Event` instances directly and call the
//! relevant inner handlers on `StripeClient`. Putting `construct_event`
//! behind the gateway would just trade one untestable thing
//! (signature verification) for another (a fake that approves anything).

use async_trait::async_trait;
use std::collections::HashMap;
use std::time::Duration;

use stripe::{
    Client, CheckoutSession, CheckoutSessionId, CheckoutSessionMode,
    CreateCheckoutSession, CreateCheckoutSessionLineItems, CreateCustomer,
    CreatePaymentIntent, CreateRefund, CreateSetupIntent, Currency, Customer,
    CustomerId, Invoice, InvoiceId, ListPaymentMethods, PaymentIntent,
    PaymentIntentConfirmationMethod, PaymentIntentId, PaymentIntentOffSession,
    PaymentIntentStatus, PaymentMethod, PaymentMethodId, PaymentMethodTypeFilter,
    Refund, RequestStrategy, SetupIntent, Subscription, SubscriptionId,
};

use crate::error::{AppError, Result};

/// 30s ceiling on every Stripe call. async-stripe 0.39 doesn't expose
/// per-request timeouts on its Client, and a hung response would tie up
/// an Axum handler indefinitely. Worst case before this: a webhook
/// handler that never returns, blocking Stripe's retry from making
/// forward progress.
const STRIPE_TIMEOUT: Duration = Duration::from_secs(30);

/// Apply STRIPE_TIMEOUT to any stripe-rs future and translate both the
/// timeout and SDK errors into `AppError::External`.
async fn timed<T, F>(fut: F) -> Result<T>
where
    F: std::future::Future<Output = std::result::Result<T, stripe::StripeError>>,
{
    match tokio::time::timeout(STRIPE_TIMEOUT, fut).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(AppError::External(format!("Stripe error: {}", e))),
        Err(_) => Err(AppError::External(format!(
            "Stripe API timed out after {}s",
            STRIPE_TIMEOUT.as_secs(),
        ))),
    }
}

// ---------------------------------------------------------------------
// Trait inputs/outputs
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct LineItemInput {
    /// Stripe expects amount_cents per unit; quantity is fixed at 1
    /// in every Coterie code path (we don't sell batches).
    pub amount_cents: i64,
    pub product_name: String,
    pub product_description: Option<String>,
}

/// Everything we need to spin up a Stripe-hosted Checkout session.
/// `metadata` rides along with the session and is mirrored onto the
/// resulting PaymentIntent — that's how the webhook dispatcher
/// resolves the local Payment row.
#[derive(Debug, Clone)]
pub struct CreateCheckoutInput {
    pub success_url: String,
    pub cancel_url: String,
    pub line_items: Vec<LineItemInput>,
    pub metadata: HashMap<String, String>,
    /// Stripe's `client_reference_id` field. Coterie uses it to stash
    /// the member_id (where applicable) for cross-checking later.
    pub client_reference_id: Option<String>,
    /// Pre-fill the donor email on the hosted page (public donations).
    pub customer_email: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CheckoutOutput {
    pub session_id: String,
    /// The hosted Stripe Checkout URL. Coterie redirects the browser
    /// here.
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct CreatePaymentIntentInput {
    pub amount_cents: i64,
    pub customer_id: String,
    pub payment_method_id: String,
    pub description: String,
    pub metadata: HashMap<String, String>,
    /// Stripe-side idempotency key. Pass the Coterie payment_id (or
    /// equivalent) so a double-submit doesn't double-charge.
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaymentIntentResult {
    Succeeded { id: String },
    RequiresAction { id: String },
    /// Anything else from Stripe — the variant carries the raw status
    /// string (Stripe occasionally adds new statuses).
    Other { id: String, status: String },
}

#[derive(Debug, Clone)]
pub struct CreateRefundInput {
    pub payment_intent_id: String,
    pub idempotency_key: String,
}

#[derive(Debug, Clone)]
pub struct CreateCustomerInput {
    pub email: String,
    pub name: Option<String>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct CreateSetupIntentInput {
    pub customer_id: String,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct RefundOutput {
    pub id: String,
}

#[derive(Debug, Clone)]
pub struct RetrievedCheckoutSession {
    pub payment_intent_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RetrievedInvoice {
    pub payment_intent_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RetrievedCustomer {
    pub id: String,
    pub email: Option<String>,
    /// `invoice_settings.default_payment_method`. Coterie tags the
    /// matching card as default in the Manage Cards UI.
    pub default_payment_method_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PaymentMethodSummary {
    pub id: String,
    pub brand: String,
    pub last4: String,
    pub exp_month: i32,
    pub exp_year: i32,
}

#[derive(Debug, Clone)]
pub struct PaymentMethodDetails {
    pub id: String,
    /// The Stripe customer this PM is attached to. Coterie verifies
    /// this matches the requesting member's customer_id before saving
    /// — defends against a forged payment_method_id from a different
    /// account.
    pub customer_id: Option<String>,
    pub brand: String,
    pub last4: String,
    pub exp_month: i32,
    pub exp_year: i32,
}

#[derive(Debug, Clone)]
pub struct SetupIntentOutput {
    pub id: String,
    pub client_secret: String,
}

// ---------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------

#[async_trait]
pub trait StripeGateway: Send + Sync {
    async fn create_checkout_session(
        &self,
        input: CreateCheckoutInput,
    ) -> Result<CheckoutOutput>;

    /// Look up checkout sessions whose `payment_intent` matches the
    /// given PI ID. Used by the refund flow to map cs_ → pi_ when the
    /// local row hasn't been upgraded.
    async fn list_checkout_sessions_by_intent(
        &self,
        payment_intent_id: &str,
    ) -> Result<Vec<String>>;

    async fn retrieve_checkout_session(
        &self,
        session_id: &str,
    ) -> Result<RetrievedCheckoutSession>;

    async fn create_customer(&self, input: CreateCustomerInput) -> Result<String>;

    async fn retrieve_customer(
        &self,
        customer_id: &str,
    ) -> Result<RetrievedCustomer>;

    async fn create_setup_intent(
        &self,
        input: CreateSetupIntentInput,
    ) -> Result<SetupIntentOutput>;

    async fn create_payment_intent(
        &self,
        input: CreatePaymentIntentInput,
    ) -> Result<PaymentIntentResult>;

    async fn list_payment_methods(
        &self,
        customer_id: &str,
    ) -> Result<Vec<PaymentMethodSummary>>;

    async fn retrieve_payment_method(
        &self,
        payment_method_id: &str,
    ) -> Result<PaymentMethodDetails>;

    async fn detach_payment_method(&self, payment_method_id: &str) -> Result<()>;

    async fn create_refund(&self, input: CreateRefundInput) -> Result<RefundOutput>;

    async fn delete_subscription(&self, subscription_id: &str) -> Result<()>;

    async fn retrieve_invoice(&self, invoice_id: &str) -> Result<RetrievedInvoice>;
}

// ---------------------------------------------------------------------
// Real implementation — delegates to stripe-rs
// ---------------------------------------------------------------------

pub struct RealStripeGateway {
    client: Client,
}

impl RealStripeGateway {
    pub fn new(api_key: String) -> Self {
        Self { client: Client::new(api_key) }
    }

    /// Test/seam access to the underlying stripe-rs client. Used during
    /// the gateway-extraction migration where some StripeClient methods
    /// still call stripe-rs directly. Will go away once every method is
    /// behind the trait.
    pub fn raw_client(&self) -> &Client {
        &self.client
    }
}

#[async_trait]
impl StripeGateway for RealStripeGateway {
    async fn create_checkout_session(
        &self,
        input: CreateCheckoutInput,
    ) -> Result<CheckoutOutput> {
        let mut params = CreateCheckoutSession::new();
        params.mode = Some(CheckoutSessionMode::Payment);
        params.success_url = Some(&input.success_url);
        params.cancel_url = Some(&input.cancel_url);
        if let Some(email) = input.customer_email.as_deref() {
            params.customer_email = Some(email);
        }

        let line_items: Vec<CreateCheckoutSessionLineItems> = input.line_items
            .iter()
            .map(|li| CreateCheckoutSessionLineItems {
                price_data: Some(stripe::CreateCheckoutSessionLineItemsPriceData {
                    currency: Currency::USD,
                    unit_amount: Some(li.amount_cents),
                    product_data: Some(stripe::CreateCheckoutSessionLineItemsPriceDataProductData {
                        name: li.product_name.clone(),
                        description: li.product_description.clone(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                quantity: Some(1),
                ..Default::default()
            })
            .collect();
        params.line_items = Some(line_items);

        if !input.metadata.is_empty() {
            params.metadata = Some(input.metadata.clone());
        }
        if let Some(ref_id) = input.client_reference_id.as_deref() {
            params.client_reference_id = Some(ref_id);
        }

        let session = timed(CheckoutSession::create(&self.client, params)).await?;
        let url = session.url
            .ok_or_else(|| AppError::External("No checkout URL returned".to_string()))?;
        Ok(CheckoutOutput {
            session_id: session.id.to_string(),
            url,
        })
    }

    async fn list_checkout_sessions_by_intent(
        &self,
        payment_intent_id: &str,
    ) -> Result<Vec<String>> {
        let pi_id: PaymentIntentId = payment_intent_id.parse().map_err(|_| {
            AppError::BadRequest(format!("Invalid PaymentIntent ID: {}", payment_intent_id))
        })?;
        let mut params = stripe::ListCheckoutSessions::new();
        params.payment_intent = Some(pi_id);
        let list = timed(CheckoutSession::list(&self.client, &params)).await?;
        Ok(list.data.into_iter().map(|s| s.id.to_string()).collect())
    }

    async fn retrieve_checkout_session(
        &self,
        session_id: &str,
    ) -> Result<RetrievedCheckoutSession> {
        let cs_id: CheckoutSessionId = session_id.parse().map_err(|_| {
            AppError::BadRequest(format!("Invalid CheckoutSession ID: {}", session_id))
        })?;
        let session = timed(CheckoutSession::retrieve(&self.client, &cs_id, &[])).await?;
        Ok(RetrievedCheckoutSession {
            payment_intent_id: session.payment_intent.map(|exp| exp.id().to_string()),
        })
    }

    async fn create_customer(&self, input: CreateCustomerInput) -> Result<String> {
        let mut params = CreateCustomer::new();
        params.email = Some(&input.email);
        if let Some(n) = input.name.as_deref() {
            params.name = Some(n);
        }
        if !input.metadata.is_empty() {
            params.metadata = Some(input.metadata.clone());
        }
        let customer = timed(Customer::create(&self.client, params)).await?;
        Ok(customer.id.to_string())
    }

    async fn retrieve_customer(&self, customer_id: &str) -> Result<RetrievedCustomer> {
        let cid: CustomerId = customer_id.parse().map_err(|_| {
            AppError::BadRequest(format!("Invalid customer ID: {}", customer_id))
        })?;
        let customer = timed(Customer::retrieve(&self.client, &cid, &[])).await?;
        let default_pm = customer.invoice_settings
            .as_ref()
            .and_then(|s| s.default_payment_method.as_ref())
            .map(|exp| exp.id().to_string());
        Ok(RetrievedCustomer {
            id: customer.id.to_string(),
            email: customer.email,
            default_payment_method_id: default_pm,
        })
    }

    async fn create_setup_intent(
        &self,
        input: CreateSetupIntentInput,
    ) -> Result<SetupIntentOutput> {
        let cid: CustomerId = input.customer_id.parse().map_err(|_| {
            AppError::BadRequest(format!("Invalid customer ID: {}", input.customer_id))
        })?;
        let mut params = CreateSetupIntent::new();
        params.customer = Some(cid);
        params.payment_method_types = Some(vec!["card".to_string()]);
        if !input.metadata.is_empty() {
            params.metadata = Some(input.metadata.clone());
        }
        let setup_intent = timed(SetupIntent::create(&self.client, params)).await?;
        let client_secret = setup_intent.client_secret
            .ok_or_else(|| AppError::External("SetupIntent missing client_secret".to_string()))?;
        Ok(SetupIntentOutput {
            id: setup_intent.id.to_string(),
            client_secret,
        })
    }

    async fn create_payment_intent(
        &self,
        input: CreatePaymentIntentInput,
    ) -> Result<PaymentIntentResult> {
        let cid: CustomerId = input.customer_id.parse().map_err(|_| {
            AppError::Internal("Invalid customer ID".to_string())
        })?;
        let pmid: PaymentMethodId = input.payment_method_id.parse().map_err(|_| {
            AppError::Internal("Invalid payment method ID".to_string())
        })?;

        let mut params = CreatePaymentIntent::new(input.amount_cents, Currency::USD);
        params.customer = Some(cid);
        params.payment_method = Some(pmid);
        params.confirm = Some(true);
        params.confirmation_method = Some(PaymentIntentConfirmationMethod::Automatic);
        params.description = Some(&input.description);
        params.off_session = Some(PaymentIntentOffSession::exists(true));
        if !input.metadata.is_empty() {
            params.metadata = Some(input.metadata.clone());
        }

        let idempotent_client = self.client.clone().with_strategy(
            RequestStrategy::Idempotent(input.idempotency_key.clone())
        );
        let intent = timed(PaymentIntent::create(&idempotent_client, params)).await
            .map_err(|e| match e {
                AppError::External(msg) => AppError::External(format!("Stripe charge failed: {}", msg)),
                other => other,
            })?;

        let id = intent.id.to_string();
        Ok(match intent.status {
            PaymentIntentStatus::Succeeded => PaymentIntentResult::Succeeded { id },
            PaymentIntentStatus::RequiresAction => PaymentIntentResult::RequiresAction { id },
            other => PaymentIntentResult::Other { id, status: format!("{:?}", other) },
        })
    }

    async fn list_payment_methods(
        &self,
        customer_id: &str,
    ) -> Result<Vec<PaymentMethodSummary>> {
        let cid: CustomerId = customer_id.parse().map_err(|_| {
            AppError::BadRequest(format!("Invalid customer ID: {}", customer_id))
        })?;
        let mut params = ListPaymentMethods::new();
        params.customer = Some(cid);
        params.type_ = Some(PaymentMethodTypeFilter::Card);
        let list = timed(PaymentMethod::list(&self.client, &params)).await?;
        Ok(list.data.into_iter().filter_map(|pm| {
            let card = pm.card?;
            Some(PaymentMethodSummary {
                id: pm.id.to_string(),
                brand: format!("{:?}", card.brand).to_lowercase(),
                last4: card.last4,
                exp_month: card.exp_month as i32,
                exp_year: card.exp_year as i32,
            })
        }).collect())
    }

    async fn retrieve_payment_method(
        &self,
        payment_method_id: &str,
    ) -> Result<PaymentMethodDetails> {
        let pm_id: PaymentMethodId = payment_method_id.parse().map_err(|_| {
            AppError::BadRequest(format!("Invalid PaymentMethod ID: {}", payment_method_id))
        })?;
        let pm = timed(PaymentMethod::retrieve(&self.client, &pm_id, &[])).await?;
        let (brand, last4, exp_month, exp_year) = pm.card
            .as_ref()
            .map(|c| (
                format!("{:?}", c.brand).to_lowercase(),
                c.last4.clone(),
                c.exp_month as i32,
                c.exp_year as i32,
            ))
            .unwrap_or_else(|| ("unknown".to_string(), String::new(), 0, 0));
        Ok(PaymentMethodDetails {
            id: pm.id.to_string(),
            customer_id: pm.customer.map(|exp| exp.id().to_string()),
            brand,
            last4,
            exp_month,
            exp_year,
        })
    }

    async fn detach_payment_method(&self, payment_method_id: &str) -> Result<()> {
        let pm_id: PaymentMethodId = payment_method_id.parse().map_err(|_| {
            AppError::BadRequest(format!("Invalid PaymentMethod ID: {}", payment_method_id))
        })?;
        timed(PaymentMethod::detach(&self.client, &pm_id)).await?;
        Ok(())
    }

    async fn create_refund(&self, input: CreateRefundInput) -> Result<RefundOutput> {
        let pi_id: PaymentIntentId = input.payment_intent_id.parse().map_err(|_| {
            AppError::BadRequest(format!("Invalid PaymentIntent ID: {}", input.payment_intent_id))
        })?;
        let mut params = CreateRefund::new();
        params.payment_intent = Some(pi_id);

        let idempotent_client = self.client.clone().with_strategy(
            RequestStrategy::Idempotent(input.idempotency_key.clone())
        );
        let refund = timed(Refund::create(&idempotent_client, params)).await
            .map_err(|e| match e {
                AppError::External(msg) => AppError::External(format!("Stripe refund failed: {}", msg)),
                other => other,
            })?;
        Ok(RefundOutput { id: refund.id.to_string() })
    }

    async fn delete_subscription(&self, subscription_id: &str) -> Result<()> {
        let sub_id: SubscriptionId = subscription_id.parse().map_err(|_| {
            AppError::BadRequest(format!("Invalid subscription ID: {}", subscription_id))
        })?;
        timed(Subscription::delete(&self.client, &sub_id)).await
            .map_err(|e| match e {
                AppError::External(msg) => AppError::External(format!("Stripe cancel failed: {}", msg)),
                other => other,
            })?;
        Ok(())
    }

    async fn retrieve_invoice(&self, invoice_id: &str) -> Result<RetrievedInvoice> {
        let inv_id: InvoiceId = invoice_id.parse().map_err(|_| {
            AppError::BadRequest(format!("Invalid invoice ID: {}", invoice_id))
        })?;
        let invoice = timed(Invoice::retrieve(&self.client, &inv_id, &[])).await?;
        Ok(RetrievedInvoice {
            payment_intent_id: invoice.payment_intent.map(|exp| exp.id().to_string()),
        })
    }
}
