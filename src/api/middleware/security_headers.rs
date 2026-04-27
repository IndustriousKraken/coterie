use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    http::{header, HeaderValue},
    middleware::Next,
    response::Response,
};
use base64::Engine;
use rand::RngCore;

use crate::api::state::AppState;

/// Sentinel that templates use in `<script nonce="__CSP_NONCE__">`.
/// The middleware substitutes the per-request nonce on the way out.
/// Distinctive enough that legitimate HTML won't carry it, but easy
/// to grep for in templates.
const NONCE_PLACEHOLDER: &str = "__CSP_NONCE__";

/// Cap on response-body size we'll rewrite in-memory. HTML pages on
/// this app top out in the low hundreds of KB; 4 MB is generous and
/// also bounds memory under any pathological response.
const MAX_REWRITE_BYTES: usize = 4 * 1024 * 1024;

/// Adds baseline security response headers including a strict
/// Content-Security-Policy with a per-request script nonce.
///
/// Why nonces and not 'unsafe-inline': removing 'unsafe-inline' from
/// `script-src` is the single biggest XSS-mitigation lever in CSP.
/// An injected `<script>` payload no longer executes because it
/// can't carry a matching nonce. Inline scripts in our templates
/// stamp the placeholder NONCE_PLACEHOLDER which this middleware
/// replaces with a fresh per-request value AND echoes into the CSP
/// header, so legit scripts keep working.
///
/// `'strict-dynamic'` lets nonced scripts (HTMX, Alpine, our inline
/// bootstrap) load further scripts transitively without each chained
/// load needing its own nonce — important for Stripe.js, which
/// dynamically injects more script tags.
///
/// Alpine.js: served via the CSP build (no `Function()` constructor),
/// so we don't need `'unsafe-eval'`.
///
/// CDN scripts are pinned with SRI hashes in the HTML.
/// Stripe.js is loaded from js.stripe.com on payment pages only.
pub async fn security_headers(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    // Generate a fresh script-src nonce for this request. 24 random
    // bytes → 32 base64 chars, well above the 128-bit minimum.
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    let nonce = base64::engine::general_purpose::STANDARD.encode(bytes);

    let response = next.run(request).await;
    let mut response = rewrite_html_nonce(response, &nonce).await;
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

    let csp = format!(
        "default-src 'self'; \
         script-src 'self' 'nonce-{nonce}' 'strict-dynamic' https://js.stripe.com https://unpkg.com; \
         style-src 'self' 'unsafe-inline'; \
         img-src 'self' data:; \
         connect-src 'self' https://api.stripe.com; \
         frame-src https://js.stripe.com; \
         frame-ancestors 'none'; \
         object-src 'none'; \
         base-uri 'self'",
    );
    if let Ok(value) = HeaderValue::from_str(&csp) {
        headers.insert(header::CONTENT_SECURITY_POLICY, value);
    }

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

/// Substitute the per-request nonce into HTML responses. Other
/// content types (JSON API, static assets, redirects, errors) pass
/// through untouched.
async fn rewrite_html_nonce(response: Response, nonce: &str) -> Response {
    let is_html = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.starts_with("text/html"))
        .unwrap_or(false);
    if !is_html {
        return response;
    }

    let (parts, body) = response.into_parts();
    let bytes = match to_bytes(body, MAX_REWRITE_BYTES).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("Failed to read HTML body for nonce rewrite: {}", e);
            return Response::from_parts(parts, Body::empty());
        }
    };

    // Fast-path: most responses don't carry the placeholder (e.g.
    // partial HTMX fragments without scripts). `String::replace`
    // does a single pass and returns the original buffer if there's
    // nothing to replace.
    let s = match std::str::from_utf8(&bytes) {
        Ok(s) => s,
        Err(_) => {
            // Non-UTF8 HTML is theoretically possible but we don't
            // produce it. Return as-is rather than mangling.
            return Response::from_parts(parts, Body::from(bytes));
        }
    };
    if !s.contains(NONCE_PLACEHOLDER) {
        return Response::from_parts(parts, Body::from(bytes));
    }
    let replaced = s.replace(NONCE_PLACEHOLDER, nonce);
    Response::from_parts(parts, Body::from(replaced))
}
