use std::sync::Arc;
use crate::{
    config::Settings,
    payments::StripeClient,
    service::ServiceContext,
};

#[derive(Clone)]
pub struct AppState {
    pub service_context: Arc<ServiceContext>,
    pub stripe_client: Option<Arc<StripeClient>>,
    pub settings: Arc<Settings>,
}

impl AppState {
    pub fn new(
        service_context: Arc<ServiceContext>,
        stripe_client: Option<Arc<StripeClient>>,
        settings: Arc<Settings>,
    ) -> Self {
        Self { 
            service_context,
            stripe_client,
            settings,
        }
    }
}