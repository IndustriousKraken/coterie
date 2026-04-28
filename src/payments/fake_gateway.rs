//! In-process Stripe gateway for tests.
//!
//! Records every call and returns canned responses queued by the test
//! body. Default responses are sensible — `create_payment_intent`
//! returns a Succeeded result with a generated `pi_test_*` ID, etc. —
//! so simple happy-path tests don't need to queue anything; tests that
//! exercise error paths or specific IDs override per call.
//!
//! Usage pattern:
//!
//! ```ignore
//! let gw = Arc::new(FakeStripeGateway::new());
//! let stripe = StripeClient::with_gateway(stripe::Client::new("dummy"), gw.clone(), ...);
//!
//! // Optional — queue a custom response for the next create_payment_intent
//! gw.next_payment_intent(PaymentIntentResult::Succeeded { id: "pi_known".into() });
//!
//! stripe.charge_saved_card(...).await?;
//!
//! // Inspect what happened
//! let calls = gw.calls();
//! assert!(matches!(calls[0], FakeCall::CreatePaymentIntent { .. }));
//! ```

use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::Mutex;

use crate::error::{AppError, Result};
use super::gateway::{
    CheckoutOutput, CreateCheckoutInput, CreateCustomerInput,
    CreatePaymentIntentInput, CreateRefundInput, CreateSetupIntentInput,
    PaymentIntentResult, PaymentMethodDetails, PaymentMethodSummary,
    RefundOutput, RetrievedCheckoutSession, RetrievedCustomer,
    RetrievedInvoice, SetupIntentOutput, StripeGateway,
};

/// Recorded gateway invocation. Tests assert on a `Vec<FakeCall>` to
/// verify what the production code dispatched.
#[derive(Debug, Clone)]
pub enum FakeCall {
    CreateCheckoutSession(CreateCheckoutInput),
    ListCheckoutSessionsByIntent { payment_intent_id: String },
    RetrieveCheckoutSession { session_id: String },
    CreateCustomer(CreateCustomerInput),
    RetrieveCustomer { customer_id: String },
    CreateSetupIntent(CreateSetupIntentInput),
    CreatePaymentIntent(CreatePaymentIntentInput),
    ListPaymentMethods { customer_id: String },
    RetrievePaymentMethod { payment_method_id: String },
    DetachPaymentMethod { payment_method_id: String },
    CreateRefund(CreateRefundInput),
    DeleteSubscription { subscription_id: String },
    RetrieveInvoice { invoice_id: String },
}

/// What to return for the next call to a given gateway method. Pre-
/// queued responses are popped FIFO; an empty queue yields a sensible
/// default. `Err(_)` simulates Stripe failures.
#[derive(Default)]
struct ResponseQueues {
    create_checkout: VecDeque<Result<CheckoutOutput>>,
    list_sessions: VecDeque<Result<Vec<String>>>,
    retrieve_session: VecDeque<Result<RetrievedCheckoutSession>>,
    create_customer: VecDeque<Result<String>>,
    retrieve_customer: VecDeque<Result<RetrievedCustomer>>,
    setup_intent: VecDeque<Result<SetupIntentOutput>>,
    payment_intent: VecDeque<Result<PaymentIntentResult>>,
    list_pms: VecDeque<Result<Vec<PaymentMethodSummary>>>,
    retrieve_pm: VecDeque<Result<PaymentMethodDetails>>,
    detach_pm: VecDeque<Result<()>>,
    refund: VecDeque<Result<RefundOutput>>,
    delete_sub: VecDeque<Result<()>>,
    retrieve_invoice: VecDeque<Result<RetrievedInvoice>>,
}

pub struct FakeStripeGateway {
    calls: Mutex<Vec<FakeCall>>,
    queues: Mutex<ResponseQueues>,
    /// Auto-incrementing counter for generated IDs (so each call gets
    /// a unique `pi_test_1`, `pi_test_2`, etc. when no explicit
    /// response was queued).
    next_id: Mutex<u64>,
}

impl FakeStripeGateway {
    pub fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            queues: Mutex::new(ResponseQueues::default()),
            next_id: Mutex::new(1),
        }
    }

    /// Snapshot of every call made so far, in order.
    pub fn calls(&self) -> Vec<FakeCall> {
        self.calls.lock().unwrap().clone()
    }

    /// Convenience: count calls of a particular shape via a predicate.
    pub fn count_where<F: Fn(&FakeCall) -> bool>(&self, pred: F) -> usize {
        self.calls.lock().unwrap().iter().filter(|c| pred(c)).count()
    }

    fn record(&self, call: FakeCall) {
        self.calls.lock().unwrap().push(call);
    }

    fn gen_id(&self, prefix: &str) -> String {
        let mut n = self.next_id.lock().unwrap();
        let id = format!("{}_test_{}", prefix, *n);
        *n += 1;
        id
    }

    // --- Response setters -------------------------------------------

    pub fn next_payment_intent(&self, result: PaymentIntentResult) {
        self.queues.lock().unwrap().payment_intent.push_back(Ok(result));
    }

    pub fn next_payment_intent_err(&self, e: AppError) {
        self.queues.lock().unwrap().payment_intent.push_back(Err(e));
    }

    pub fn next_refund(&self, refund: RefundOutput) {
        self.queues.lock().unwrap().refund.push_back(Ok(refund));
    }

    pub fn next_refund_err(&self, e: AppError) {
        self.queues.lock().unwrap().refund.push_back(Err(e));
    }

    pub fn next_checkout_session(&self, output: CheckoutOutput) {
        self.queues.lock().unwrap().create_checkout.push_back(Ok(output));
    }

    pub fn next_retrieve_checkout_session(&self, retrieved: RetrievedCheckoutSession) {
        self.queues.lock().unwrap().retrieve_session.push_back(Ok(retrieved));
    }

    pub fn next_retrieve_invoice(&self, retrieved: RetrievedInvoice) {
        self.queues.lock().unwrap().retrieve_invoice.push_back(Ok(retrieved));
    }
}

impl Default for FakeStripeGateway {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl StripeGateway for FakeStripeGateway {
    async fn create_checkout_session(
        &self,
        input: CreateCheckoutInput,
    ) -> Result<CheckoutOutput> {
        self.record(FakeCall::CreateCheckoutSession(input.clone()));
        if let Some(r) = self.queues.lock().unwrap().create_checkout.pop_front() {
            return r;
        }
        let id = self.gen_id("cs");
        Ok(CheckoutOutput {
            url: format!("https://checkout.stripe.test/{}", id),
            session_id: id,
        })
    }

    async fn list_checkout_sessions_by_intent(
        &self,
        payment_intent_id: &str,
    ) -> Result<Vec<String>> {
        self.record(FakeCall::ListCheckoutSessionsByIntent {
            payment_intent_id: payment_intent_id.to_string(),
        });
        if let Some(r) = self.queues.lock().unwrap().list_sessions.pop_front() {
            return r;
        }
        Ok(Vec::new())
    }

    async fn retrieve_checkout_session(
        &self,
        session_id: &str,
    ) -> Result<RetrievedCheckoutSession> {
        self.record(FakeCall::RetrieveCheckoutSession {
            session_id: session_id.to_string(),
        });
        if let Some(r) = self.queues.lock().unwrap().retrieve_session.pop_front() {
            return r;
        }
        Ok(RetrievedCheckoutSession { payment_intent_id: None })
    }

    async fn create_customer(&self, input: CreateCustomerInput) -> Result<String> {
        self.record(FakeCall::CreateCustomer(input));
        if let Some(r) = self.queues.lock().unwrap().create_customer.pop_front() {
            return r;
        }
        Ok(self.gen_id("cus"))
    }

    async fn retrieve_customer(&self, customer_id: &str) -> Result<RetrievedCustomer> {
        self.record(FakeCall::RetrieveCustomer {
            customer_id: customer_id.to_string(),
        });
        if let Some(r) = self.queues.lock().unwrap().retrieve_customer.pop_front() {
            return r;
        }
        Ok(RetrievedCustomer {
            id: customer_id.to_string(),
            email: None,
            default_payment_method_id: None,
        })
    }

    async fn create_setup_intent(
        &self,
        input: CreateSetupIntentInput,
    ) -> Result<SetupIntentOutput> {
        self.record(FakeCall::CreateSetupIntent(input));
        if let Some(r) = self.queues.lock().unwrap().setup_intent.pop_front() {
            return r;
        }
        let id = self.gen_id("seti");
        Ok(SetupIntentOutput {
            client_secret: format!("{}_secret_test", id),
            id,
        })
    }

    async fn create_payment_intent(
        &self,
        input: CreatePaymentIntentInput,
    ) -> Result<PaymentIntentResult> {
        self.record(FakeCall::CreatePaymentIntent(input.clone()));
        if let Some(r) = self.queues.lock().unwrap().payment_intent.pop_front() {
            return r;
        }
        Ok(PaymentIntentResult::Succeeded { id: self.gen_id("pi") })
    }

    async fn list_payment_methods(
        &self,
        customer_id: &str,
    ) -> Result<Vec<PaymentMethodSummary>> {
        self.record(FakeCall::ListPaymentMethods {
            customer_id: customer_id.to_string(),
        });
        if let Some(r) = self.queues.lock().unwrap().list_pms.pop_front() {
            return r;
        }
        Ok(Vec::new())
    }

    async fn retrieve_payment_method(
        &self,
        payment_method_id: &str,
    ) -> Result<PaymentMethodDetails> {
        self.record(FakeCall::RetrievePaymentMethod {
            payment_method_id: payment_method_id.to_string(),
        });
        if let Some(r) = self.queues.lock().unwrap().retrieve_pm.pop_front() {
            return r;
        }
        Ok(PaymentMethodDetails {
            id: payment_method_id.to_string(),
            customer_id: None,
            brand: "visa".to_string(),
            last4: "4242".to_string(),
            exp_month: 12,
            exp_year: 2030,
        })
    }

    async fn detach_payment_method(&self, payment_method_id: &str) -> Result<()> {
        self.record(FakeCall::DetachPaymentMethod {
            payment_method_id: payment_method_id.to_string(),
        });
        self.queues.lock().unwrap().detach_pm.pop_front().unwrap_or(Ok(()))
    }

    async fn create_refund(&self, input: CreateRefundInput) -> Result<RefundOutput> {
        self.record(FakeCall::CreateRefund(input.clone()));
        if let Some(r) = self.queues.lock().unwrap().refund.pop_front() {
            return r;
        }
        Ok(RefundOutput { id: self.gen_id("re") })
    }

    async fn delete_subscription(&self, subscription_id: &str) -> Result<()> {
        self.record(FakeCall::DeleteSubscription {
            subscription_id: subscription_id.to_string(),
        });
        self.queues.lock().unwrap().delete_sub.pop_front().unwrap_or(Ok(()))
    }

    async fn retrieve_invoice(&self, invoice_id: &str) -> Result<RetrievedInvoice> {
        self.record(FakeCall::RetrieveInvoice {
            invoice_id: invoice_id.to_string(),
        });
        if let Some(r) = self.queues.lock().unwrap().retrieve_invoice.pop_front() {
            return r;
        }
        Ok(RetrievedInvoice { payment_intent_id: None })
    }
}
