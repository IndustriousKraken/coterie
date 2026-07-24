//! Email verification landing page. Handles the link from the signup
//! verification email: consumes the token and marks the member as
//! email-verified. Shows a success or error page either way.

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Query, State},
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    auth,
    config::Settings,
    payments::StripeClient,
    repository::{MemberRepository, PaymentRepository},
    service::{membership_type_service::MembershipTypeService, settings_service::SettingsService},
    web::templates::{BaseContext, HtmlTemplate},
};

#[derive(Debug, Deserialize)]
pub struct VerifyQuery {
    pub token: String,
}

#[derive(Template)]
#[template(path = "auth/verify_result.html")]
pub struct VerifyResultTemplate {
    pub base: BaseContext,
    pub success: bool,
    pub message: String,
}

#[allow(clippy::too_many_arguments)]
pub async fn verify_handler(
    State(db_pool): State<SqlitePool>,
    State(member_repo): State<Arc<dyn MemberRepository>>,
    State(payment_repo): State<Arc<dyn PaymentRepository>>,
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    State(stripe_client): State<Option<Arc<StripeClient>>>,
    State(settings_service): State<Arc<SettingsService>>,
    State(settings): State<Arc<Settings>>,
    Query(query): Query<VerifyQuery>,
) -> Response {
    let (success, message) = match auth::email_tokens::consume_verification_token(
        &db_pool,
        &query.token,
    )
    .await
    {
        Ok(Some(consumed)) => {
            // Mark the member as verified. Any other outstanding
            // verification tokens for this member become moot (the DB
            // state already reflects "verified"), but invalidate them
            // as well for cleanliness.
            if let Err(e) = member_repo.mark_email_verified(consumed.member_id).await {
                tracing::error!("Failed to mark email verified: {}", e);
                (
                    false,
                    "We couldn't finish verifying your email. Please try again or contact support."
                        .to_string(),
                )
            } else {
                if let Err(e) = auth::email_tokens::invalidate_verification_tokens_for_member(
                    &db_pool,
                    consumed.member_id,
                )
                .await
                {
                    tracing::warn!(
                        "Verified email for member {} but couldn't invalidate other tokens: {}",
                        consumed.member_id,
                        e
                    );
                }

                // Verification is the point at which a payment-mode signup
                // reaches Stripe: for a Pending, unpaid, payment-mode member
                // with a paid type, initiate the Checkout session now and
                // redirect them to it. Every other case (approval mode,
                // fee-0 type, already-paid) keeps the "verified; awaiting
                // review" result. A checkout-creation failure is soft — the
                // email is verified regardless; the retry path can re-mint a
                // session — so we fall back to a message rather than 500.
                match member_repo.find_by_id(consumed.member_id).await {
                    Ok(Some(member)) => {
                        match crate::api::handlers::public::initiate_checkout_on_verify(
                            payment_repo.as_ref(),
                            &membership_type_service,
                            stripe_client.as_deref(),
                            &settings_service,
                            &settings,
                            &member,
                        )
                        .await
                        {
                            Ok(Some(url)) => return Redirect::to(&url).into_response(),
                            Ok(None) => (true, "Your email has been verified. An administrator will review your account shortly.".to_string()),
                            Err(e) => {
                                tracing::error!(
                                    "Verified email for member {} but checkout initiation failed: {}",
                                    consumed.member_id,
                                    e,
                                );
                                (true, "Your email is verified. We couldn't start your payment just yet — please return to the join form to continue to payment.".to_string())
                            }
                        }
                    }
                    Ok(None) => {
                        // Token consumed but the member vanished — nothing to
                        // bill. Treat as verified; nothing more to do.
                        (true, "Your email has been verified.".to_string())
                    }
                    Err(e) => {
                        tracing::error!(
                            "Verified email for member {} but member lookup failed: {}",
                            consumed.member_id,
                            e,
                        );
                        (true, "Your email has been verified. An administrator will review your account shortly.".to_string())
                    }
                }
            }
        }
        Ok(None) => (
            false,
            "This verification link is invalid or has expired.".to_string(),
        ),
        Err(e) => {
            tracing::error!("Verification token lookup failed: {}", e);
            (
                false,
                "Something went wrong. Please try again later.".to_string(),
            )
        }
    };

    HtmlTemplate(VerifyResultTemplate {
        base: BaseContext::for_anon(),
        success,
        message,
    })
    .into_response()
}
