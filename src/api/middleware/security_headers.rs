use axum::{
    extract::{Request, State},
    http::{header, HeaderValue},
    middleware::Next,
    response::Response,
};

use crate::api::state::AppState;

/// Adds baseline security response headers including a Content-Security-Policy.
///
/// CDN scripts (HTMX, Alpine.js) are pinned with SRI hashes in the HTML.
/// Tailwind CSS is built locally — no CDN or unsafe-eval needed.
/// Stripe.js is loaded from js.stripe.com on payment pages only.
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

    // Content-Security-Policy:
    //  - script-src: self + CDN hosts for HTMX/Alpine/Stripe (all pinned with SRI),
    //    'unsafe-inline' for the small inline scripts in base.html.
    //  - style-src: self + 'unsafe-inline' (for small inline <style> blocks).
    //  - connect-src: self + Stripe API for payment processing.
    //  - img-src: self + data: (for inline images).
    //  - frame-src: js.stripe.com (Stripe 3D-Secure iframes).
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; \
             script-src 'self' https://unpkg.com https://js.stripe.com 'unsafe-inline'; \
             style-src 'self' 'unsafe-inline'; \
             img-src 'self' data:; \
             connect-src 'self' https://api.stripe.com; \
             frame-src https://js.stripe.com; \
             frame-ancestors 'none'; \
             object-src 'none'; \
             base-uri 'self'"
        ),
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
