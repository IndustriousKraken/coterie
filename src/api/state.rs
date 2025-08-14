use std::sync::Arc;
use crate::service::ServiceContext;

#[derive(Clone)]
pub struct AppState {
    pub service_context: Arc<ServiceContext>,
}

impl AppState {
    pub fn new(service_context: Arc<ServiceContext>) -> Self {
        Self { service_context }
    }
}