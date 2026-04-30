pub mod gateway;
pub mod stripe_client;
pub mod webhook_dispatcher;

#[cfg(any(test, feature = "test-utils"))]
pub mod fake_gateway;

pub use gateway::{
    StripeGateway, RealStripeGateway,
    CreateCheckoutInput, CheckoutOutput, LineItemInput,
    CreatePaymentIntentInput, PaymentIntentResult,
    CreateRefundInput, RefundOutput,
    RetrievedCheckoutSession, RetrievedInvoice, RetrievedCustomer,
    PaymentMethodSummary, PaymentMethodDetails,
    SetupIntentOutput,
};
pub use stripe_client::StripeClient;
pub use webhook_dispatcher::WebhookDispatcher;