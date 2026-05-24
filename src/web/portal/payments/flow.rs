use std::sync::Arc;

use askama::Template;
use axum::{extract::State, response::IntoResponse, Extension};

use crate::{
    api::middleware::auth::{CurrentUser, SessionInfo},
    auth::CsrfService,
    config::Settings,
    payments::StripeClient,
    repository::SavedCardRepository,
    service::membership_type_service::MembershipTypeService,
    web::templates::{BaseContext, HtmlTemplate},
};

#[derive(Template)]
#[template(path = "portal/payment_new.html")]
pub struct PaymentNewTemplate {
    pub base: BaseContext,
    pub stripe_enabled: bool,
    pub membership_types: Vec<MembershipTypeDisplay>,
    pub saved_cards: Vec<SavedCardDisplay>,
    pub stripe_publishable_key: String,
    /// True when the member is already on Coterie-managed auto-renew.
    /// We hide the "Enable auto-renew" checkbox in that case and show
    /// an "already enrolled" badge instead — the saved-card payment
    /// will keep them enrolled and refresh the schedule automatically.
    pub is_auto_renew: bool,
}

pub struct SavedCardDisplay {
    pub id: String,
    pub display_name: String,
    pub exp_display: String,
    pub is_default: bool,
}

pub struct MembershipTypeDisplay {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub color: Option<String>,
    pub fee_display: String,
    pub billing_period: String,
}

#[derive(Template)]
#[template(path = "portal/payment_success.html")]
pub struct PaymentSuccessTemplate {
    pub base: BaseContext,
}

#[derive(Template)]
#[template(path = "portal/payment_cancel.html")]
pub struct PaymentCancelTemplate {
    pub base: BaseContext,
}

pub async fn payment_success_page(
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session): Extension<SessionInfo>,
) -> impl IntoResponse {
    let template = PaymentSuccessTemplate {
        base: BaseContext::for_member(&csrf_service, &current_user, &session).await,
    };

    HtmlTemplate(template)
}

pub async fn payment_cancel_page(
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session): Extension<SessionInfo>,
) -> impl IntoResponse {
    let template = PaymentCancelTemplate {
        base: BaseContext::for_member(&csrf_service, &current_user, &session).await,
    };

    HtmlTemplate(template)
}

pub async fn payment_new_page(
    State(csrf_service): State<Arc<CsrfService>>,
    State(settings): State<Arc<Settings>>,
    State(stripe_client): State<Option<Arc<StripeClient>>>,
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    State(saved_card_repo): State<Arc<dyn SavedCardRepository>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> impl IntoResponse {
    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;

    let stripe_enabled = stripe_client.is_some();
    let stripe_publishable_key = settings.stripe.publishable_key.clone().unwrap_or_default();

    let membership_types = membership_type_service
        .list(false)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|mt| MembershipTypeDisplay {
            name: mt.name,
            slug: mt.slug,
            description: mt.description,
            color: mt.color,
            fee_display: format!("{:.2}", mt.fee_cents as f64 / 100.0),
            billing_period: mt.billing_period,
        })
        .collect();

    let saved_cards = saved_card_repo
        .find_by_member(current_user.member.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|c| SavedCardDisplay {
            id: c.id.to_string(),
            display_name: c.display_name(),
            exp_display: c.exp_display(),
            is_default: c.is_default,
        })
        .collect();

    let is_auto_renew =
        current_user.member.billing_mode == crate::domain::BillingMode::CoterieManaged;

    let template = PaymentNewTemplate {
        base,
        stripe_enabled,
        membership_types,
        saved_cards,
        stripe_publishable_key,
        is_auto_renew,
    };

    HtmlTemplate(template)
}
