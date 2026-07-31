//! Public event registration (guests) and class enrollment — the two
//! unauthenticated, money-moving registration endpoints. Both keep the
//! same fixed protection order documented on [`register_for_event`].

use std::sync::Arc;

use axum::{
    extract::State,
    http::{header, HeaderMap},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    api::{middleware::bot_challenge::BotChallengeVerifier, state::MoneyLimiter},
    config::Settings,
    email::EmailSender,
    error::{AppError, Result},
    repository::{EventRepository, EventSeriesRepository},
    service::{
        event_registration_service::{EventRegistrationService, RegistrationOutcome},
        series_enrollment_service::SeriesEnrollmentService,
        settings_service::SettingsService,
    },
};

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct PublicEventRegisterRequest {
    /// The guest's name, as they typed it. Bounded and validated at the
    /// service boundary; never matched against the member directory.
    pub name: String,
    pub email: String,
    /// Bot-challenge token from the registration form's CAPTCHA widget.
    /// Required when the org has configured a provider; ignored when
    /// `bot_challenge.provider = "disabled"`. Same contract as
    /// `/public/signup` and `/public/donate`.
    #[serde(default)]
    pub captcha_token: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PublicEventRegisterResponse {
    /// Stripe-hosted Checkout URL when the event has a guest price —
    /// redirect the guest here. `null` for a free registration, whose
    /// seat is already confirmed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkout_url: Option<String>,
    /// `"registered"` when the seat is confirmed (free event, or an
    /// already-paid guest), `"checkout"` when payment is still owed.
    pub status: &'static str,
    pub message: String,
}

/// `POST /public/events/:id/register` — an anonymous visitor registers
/// for a publicly-registerable event.
///
/// This is an unauthenticated, money-moving, publicly-reachable endpoint
/// — the same shape as `/public/signup` and `/public/donate`, which are
/// exactly the endpoints card-testing abuse hit. The protections are
/// therefore in a fixed order and are not optional:
///
///   1. CORS allowlist (the router's global layer),
///   2. the per-IP `money_limiter` — BEFORE the provider, so a bursting
///      IP can't burn the org's Turnstile quota,
///   3. bot-challenge verification, fail-closed,
///   4. the registerability check (404, never 403 — a 403 on a
///      members-only id would confirm the event exists),
///   5. then, and only then, a seat and a charge.
///
/// Nothing before step 5 writes state, so a rate-limited or
/// challenge-failed request claims no seat and creates no payment row.
///
/// Accepts JSON (the marketing site's `fetch`) or form-urlencoded (the
/// Coterie-hosted page's plain `<form>`, which works with no JavaScript);
/// a form post gets a redirect, a JSON post gets JSON.
#[utoipa::path(
    post,
    path = "/public/events/{id}/register",
    tag = "public",
    request_body = PublicEventRegisterRequest,
    responses(
        (status = 200, description = "Seat held; redirect the guest to `checkout_url`, or the \
            seat is confirmed for a free event", body = PublicEventRegisterResponse),
        (status = 400, description = "Invalid name or email, or the event is full"),
        (status = 403, description = "Bot-challenge verification failed (fail-closed)"),
        (status = 404, description = "No publicly registerable event with this id — the same \
            response a members-only event and a nonexistent id both get"),
        (status = 429, description = "Rate limited (per-IP money limiter)"),
        (status = 503, description = "Payment processing not configured"),
    ),
)]
pub async fn register_for_event(
    State(settings): State<Arc<Settings>>,
    State(money_limiter): State<MoneyLimiter>,
    State(bot_challenge_verifier): State<Arc<dyn BotChallengeVerifier>>,
    State(event_repo): State<Arc<dyn EventRepository>>,
    State(registration_service): State<Arc<EventRegistrationService>>,
    State(email_sender): State<Arc<dyn EmailSender>>,
    State(settings_service): State<Arc<SettingsService>>,
    axum::extract::Path(event_id): axum::extract::Path<Uuid>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response> {
    let wants_html = is_form_encoded(&headers);
    let request: PublicEventRegisterRequest = parse_body(&headers, &body)?;

    // Rate limit FIRST, before the bot-challenge provider is consulted:
    // a bursting IP must not be able to spend the org's provider quota.
    let ip = crate::api::state::client_ip(&headers, settings.server.trust_forwarded_for());
    if !money_limiter
        .0
        .check_and_record(ip, "public.event_register")
    {
        return Err(AppError::TooManyRequests);
    }

    // Fail closed: when the org has configured a provider, every request
    // must carry a token the provider verifies. A free registration gets
    // no relaxation — with no card in front of it, this and the limiter
    // are the only controls it has.
    if bot_challenge_verifier
        .verify(
            "public/events/register",
            request.captcha_token.as_deref(),
            Some(ip),
        )
        .await
        .is_err()
    {
        return Err(AppError::Forbidden);
    }

    // 404, not 403: a members-only event id and a nonexistent one must be
    // indistinguishable, or the response leaks which private events exist.
    let event = event_repo
        .find_by_id(event_id)
        .await?
        .filter(|e| e.publicly_registerable())
        .ok_or_else(|| AppError::NotFound("Event not found".to_string()))?;

    let outcome = registration_service
        .register_guest(&event, &request.name, &request.email)
        .await?;

    // Free registrations are confirmed right here, so this is where their
    // confirmation email goes out. A paid one is confirmed by the
    // completion webhook, which sends it there — the seat and the email
    // stay together in both cases.
    if let RegistrationOutcome::Registered = outcome {
        if !event.is_paid_for_guests() {
            crate::service::billing_service::notifications::dispatch_guest_event_confirmation(
                &email_sender,
                &settings_service,
                &request.email,
                &request.name,
                &event,
                None,
            )
            .await;
        }
    }

    let (checkout_url, status, message) = match outcome {
        RegistrationOutcome::Checkout { url } => (
            Some(url),
            "checkout",
            "Complete your payment to confirm your seat.".to_string(),
        ),
        RegistrationOutcome::Registered => (
            None,
            "registered",
            "You're registered — check your email for the details.".to_string(),
        ),
    };

    if wants_html {
        // A browser posted a form, so answer with navigation rather than
        // JSON: Stripe for a paid seat, back to the page for a free one
        // (where the confirmed state is read from the database).
        let location = checkout_url.unwrap_or_else(|| {
            format!(
                "{}/events/{}/register?registered=1",
                settings.server.base_url.trim_end_matches('/'),
                event.id,
            )
        });
        return Ok(axum::response::Redirect::to(&location).into_response());
    }

    Ok(Json(PublicEventRegisterResponse {
        checkout_url,
        status,
        message,
    })
    .into_response())
}

/// `POST /public/series/:id/enroll` — an anonymous visitor buys a pass to
/// a publicly-enrollable class.
///
/// The class-scope sibling of [`register_for_event`], with the identical
/// protection order and for the identical reasons — this is an
/// unauthenticated, money-moving, publicly-reachable endpoint:
///
///   1. CORS allowlist (the router's global layer),
///   2. the per-IP `money_limiter` — BEFORE the provider, so a bursting
///      IP can't burn the org's Turnstile quota,
///   3. bot-challenge verification, fail-closed,
///   4. the enrollability check (404, never 403),
///   5. then, and only then, a place and a charge.
///
/// Nothing before step 5 writes state, so a rate-limited or
/// challenge-failed request claims no place and creates no payment row.
#[utoipa::path(
    post,
    path = "/public/series/{id}/enroll",
    tag = "public",
    request_body = PublicEventRegisterRequest,
    responses(
        (status = 200, description = "Place held; redirect the guest to `checkout_url`, or the \
            enrollment is confirmed for a free class", body = PublicEventRegisterResponse),
        (status = 400, description = "Invalid name or email, or the class is full"),
        (status = 403, description = "Bot-challenge verification failed (fail-closed)"),
        (status = 404, description = "No publicly enrollable class with this id"),
        (status = 429, description = "Rate limited (per-IP money limiter)"),
        (status = 503, description = "Payment processing not configured"),
    ),
)]
#[allow(clippy::too_many_arguments)]
pub async fn enroll_in_class(
    State(settings): State<Arc<Settings>>,
    State(money_limiter): State<MoneyLimiter>,
    State(bot_challenge_verifier): State<Arc<dyn BotChallengeVerifier>>,
    State(event_repo): State<Arc<dyn EventRepository>>,
    State(series_repo): State<Arc<dyn EventSeriesRepository>>,
    State(enrollment_service): State<Arc<SeriesEnrollmentService>>,
    axum::extract::Path(series_id): axum::extract::Path<Uuid>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response> {
    let wants_html = is_form_encoded(&headers);
    let request: PublicEventRegisterRequest = parse_body(&headers, &body)?;

    // Rate limit FIRST, before the bot-challenge provider is consulted.
    let ip = crate::api::state::client_ip(&headers, settings.server.trust_forwarded_for());
    if !money_limiter.0.check_and_record(ip, "public.class_enroll") {
        return Err(AppError::TooManyRequests);
    }

    // Fail closed: when the org has configured a provider, every request
    // must carry a token the provider verifies.
    if bot_challenge_verifier
        .verify(
            "public/series/enroll",
            request.captcha_token.as_deref(),
            Some(ip),
        )
        .await
        .is_err()
    {
        return Err(AppError::Forbidden);
    }

    // 404, not 403: a members-only class id and a nonexistent one must be
    // indistinguishable. One home for the rule, shared with the page.
    let (series, occurrences) = crate::web::templates::class_register::load_enrollable(
        &*event_repo,
        &*series_repo,
        series_id,
    )
    .await
    .ok_or_else(|| AppError::NotFound("Class not found".to_string()))?;

    let title = occurrences
        .first()
        .map(|e| e.title.clone())
        .unwrap_or_else(|| "Class".to_string());

    let outcome = enrollment_service
        .enroll_guest(&series, &title, &request.name, &request.email)
        .await?;

    let (checkout_url, status, message) = match outcome {
        RegistrationOutcome::Checkout { url } => (
            Some(url),
            "checkout",
            "Complete your payment to confirm your place.".to_string(),
        ),
        RegistrationOutcome::Registered => (
            None,
            "registered",
            "You're enrolled — check your email for the details.".to_string(),
        ),
    };

    if wants_html {
        // A browser posted a form, so answer with navigation rather than
        // JSON: Stripe for a paid place, back to the page for a free one
        // (where the confirmed state is read from the database).
        let location = checkout_url.unwrap_or_else(|| {
            format!(
                "{}/classes/{}/register?registered=1",
                settings.server.base_url.trim_end_matches('/'),
                series.id,
            )
        });
        return Ok(axum::response::Redirect::to(&location).into_response());
    }

    Ok(Json(PublicEventRegisterResponse {
        checkout_url,
        status,
        message,
    })
    .into_response())
}

fn is_form_encoded(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("application/x-www-form-urlencoded"))
}

/// Parse the registration body as JSON or as a URL-encoded form. Two
/// callers, one contract: the marketing site posts JSON cross-origin,
/// and Coterie's own hosted page posts a plain `<form>` so it works
/// without JavaScript.
fn parse_body<T: serde::de::DeserializeOwned>(headers: &HeaderMap, body: &[u8]) -> Result<T> {
    if is_form_encoded(headers) {
        serde_urlencoded::from_bytes(body)
            .map_err(|e| AppError::BadRequest(format!("Invalid form body: {}", e)))
    } else {
        serde_json::from_slice(body)
            .map_err(|e| AppError::BadRequest(format!("Invalid JSON body: {}", e)))
    }
}
