//! Email verification landing page. Handles the link from the signup
//! verification email: consumes the token and marks the member as
//! email-verified. Shows a success or error page either way.

use askama::Template;
use axum::{
    extract::{Query, State},
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use crate::{
    api::state::AppState,
    auth::EmailTokenService,
    repository::MemberRepository,
    web::templates::HtmlTemplate,
};

#[derive(Debug, Deserialize)]
pub struct VerifyQuery {
    pub token: String,
}

#[derive(Template)]
#[template(path = "auth/verify_result.html")]
pub struct VerifyResultTemplate {
    pub current_user: Option<super::UserInfo>,
    pub is_admin: bool,
    pub success: bool,
    pub message: String,
}

pub async fn verify_handler(
    State(state): State<AppState>,
    Query(query): Query<VerifyQuery>,
) -> Response {
    let service = EmailTokenService::verification(state.service_context.db_pool.clone());

    let (success, message) = match service.consume(&query.token).await {
        Ok(Some(consumed)) => {
            // Mark the member as verified. Any other outstanding
            // verification tokens for this member become moot (the DB
            // state already reflects "verified"), but invalidate them
            // as well for cleanliness.
            if let Err(e) = state.service_context.member_repo
                .mark_email_verified(consumed.member_id).await
            {
                tracing::error!("Failed to mark email verified: {}", e);
                (false, "We couldn't finish verifying your email. Please try again or contact support.".to_string())
            } else {
                if let Err(e) = service.invalidate_for_member(consumed.member_id).await {
                    tracing::warn!(
                        "Verified email for member {} but couldn't invalidate other tokens: {}",
                        consumed.member_id, e
                    );
                }
                (true, "Your email has been verified. An administrator will review your account shortly.".to_string())
            }
        }
        Ok(None) => (false, "This verification link is invalid or has expired.".to_string()),
        Err(e) => {
            tracing::error!("Verification token lookup failed: {}", e);
            (false, "Something went wrong. Please try again later.".to_string())
        }
    };

    HtmlTemplate(VerifyResultTemplate {
        current_user: None,
        is_admin: false,
        success,
        message,
    }).into_response()
}
