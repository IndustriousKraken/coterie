use axum::{
    extract::{Request, State},
    http::{header, HeaderValue},
    middleware::Next,
    response::Response,
};

use crate::api::state::AppState;

/// Adds baseline security response headers.
///
/// Intentionally omits a strict Content-Security-Policy for now — the portal
/// templates pull HTMX, Alpine, Tailwind, and Stripe from third-party CDNs
/// without SRI, and locking down script-src would break them. Once those
/// are either vendored locally or pinned with SRI, add a script-src/style-src
/// directive here.
pub async fn security_headers(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    headers.insert(
        header::X_FRAME_OPTIONS,
        HeaderValue::from_static("DENY"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );

    // Limit the blast radius of any XSS to /self/ framing and object embeds
    // even without a full script-src policy.
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("frame-ancestors 'none'; object-src 'none'; base-uri 'self'"),
    );

    // HSTS only meaningful on TLS deployments — sending it over plain HTTP
    // is ignored by browsers but sending it at all on dev would be noise.
    if state.settings.server.cookies_are_secure() {
        headers.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        );
    }

    response
}
