use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::{
    api::state::AppState,
    error::Result,
    service::audit_service::AuditEntry,
};

pub async fn stats(State(_state): State<AppState>) -> Result<Json<String>> {
    Ok(Json("Stats not implemented".to_string()))
}

#[derive(Debug, Deserialize)]
pub struct AuditLogQuery {
    pub limit: Option<i64>,
}

pub async fn audit_log(
    State(state): State<AppState>,
    Query(query): Query<AuditLogQuery>,
) -> Result<Json<Vec<AuditEntry>>> {
    let entries = state.service_context.audit_service
        .recent(query.limit.unwrap_or(100))
        .await?;
    Ok(Json(entries))
}

#[derive(Serialize)]
pub struct CheckExpiredResponse {
    pub expired_count: u32,
    pub reminders_sent: u32,
}

/// Run the same membership-maintenance cycle the background job runs —
/// expire members past grace period and email dues reminders. Useful
/// for admin "run now" actions and for exercising the flow in tests.
pub async fn check_expired(State(state): State<AppState>) -> Result<Json<CheckExpiredResponse>> {
    let billing = state.service_context.billing_service(
        state.stripe_client.clone(),
        state.settings.server.base_url.clone(),
    );
    let expired_count = billing.check_expired_members().await?;
    let reminders_sent = billing.send_dues_reminders().await?;
    Ok(Json(CheckExpiredResponse { expired_count, reminders_sent }))
}