//! Integration tests for the pay-at-signup funnel (see the
//! `pay-at-signup` OpenSpec change): `membership.signup_mode=payment`
//! turns `/public/signup` into a checkout-initiating endpoint, and a
//! completed membership payment activates the Pending member.

use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use coterie::{
    api::middleware::bot_challenge::{BotChallengeVerifier, VerifyError},
    integrations::{Integration, IntegrationEvent},
    payments::{fake_gateway::FakeStripeGateway, StripeClient, StripeHandle},
    repository::{SqliteMemberRepository, SqlitePaymentRepository},
};
use sqlx::SqlitePool;
use tower::ServiceExt;

mod common;
use common::{build_app_state_custom, fresh_pool};

// ---------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------

const PAID_SLUG: &str = "paid-tier";
const FREE_SLUG: &str = "free-tier";

async fn set_signup_mode(pool: &SqlitePool, mode: &str) {
    let updated = sqlx::query("UPDATE app_settings SET value = ? WHERE key = 'membership.signup_mode'")
        .bind(mode)
        .execute(pool)
        .await
        .expect("update signup_mode")
        .rows_affected();
    assert_eq!(updated, 1, "membership.signup_mode row must be seeded by migration 037");
}

async fn insert_membership_type(pool: &SqlitePool, slug: &str, fee_cents: i32) {
    sqlx::query(
        "INSERT INTO membership_types \
           (id, name, slug, description, sort_order, is_active, fee_cents, billing_period, \
            created_at, updated_at) \
         VALUES (?, ?, ?, 'test tier', 50, 1, ?, 'monthly', datetime('now'), datetime('now'))",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(slug)
    .bind(slug)
    .bind(fee_cents)
    .execute(pool)
    .await
    .expect("insert membership type");
}

/// AppState wired with a FakeStripeGateway-backed StripeClient, so the
/// handler's `Option<Arc<StripeClient>>` extractor sees a configured
/// Stripe surface without network access.
async fn state_with_fake_stripe(
    pool: &SqlitePool,
    verifier: Option<Arc<dyn BotChallengeVerifier>>,
) -> (coterie::api::state::AppState, Arc<FakeStripeGateway>) {
    let fake = Arc::new(FakeStripeGateway::new());
    let gw: Arc<dyn coterie::payments::gateway::StripeGateway> = fake.clone();
    let client = StripeClient::with_gateway(
        gw,
        Arc::new(SqlitePaymentRepository::new(pool.clone())),
        Arc::new(SqliteMemberRepository::new(pool.clone())),
    );
    let handle = Arc::new(StripeHandle::preloaded(Some(Arc::new(client)), None));
    let state = build_app_state_custom(pool.clone(), Some(handle), verifier).await;
    (state, fake)
}

fn signup_body(email: &str, username: &str, slug: &str) -> serde_json::Value {
    serde_json::json!({
        "email": email,
        "username": username,
        "full_name": "Pay AtSignup",
        "password": "Sup3rSecretPw!",
        "membership_type_slug": slug,
    })
}

async fn post_signup(app: Router, body: &serde_json::Value) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("POST")
        .uri("/public/signup")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

/// Like `post_signup` but returns the raw response body so callers can
/// assert two 409s are byte-identical.
async fn post_signup_raw(app: Router, body: &serde_json::Value) -> (StatusCode, String) {
    let req = Request::builder()
        .method("POST")
        .uri("/public/signup")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn member_status(pool: &SqlitePool, email: &str) -> String {
    sqlx::query_scalar("SELECT status FROM members WHERE email = ?")
        .bind(email)
        .fetch_one(pool)
        .await
        .expect("member status")
}

// ---------------------------------------------------------------------
// 4.1 Approval mode (default) — regression: unchanged, no checkout
// ---------------------------------------------------------------------

#[tokio::test]
async fn approval_mode_default_creates_pending_without_checkout() {
    let pool = fresh_pool().await;
    insert_membership_type(&pool, PAID_SLUG, 4500).await;
    let (state, fake) = state_with_fake_stripe(&pool, None).await;
    let app = coterie::api::create_app(state);

    let (status, json) = post_signup(app, &signup_body("a@x.com", "alpha", PAID_SLUG)).await;

    assert_eq!(status, StatusCode::CREATED);
    assert!(
        json.get("checkout_url").is_none(),
        "approval mode must not return a checkout URL (got {json})"
    );
    assert_eq!(member_status(&pool, "a@x.com").await, "Pending");
    assert!(
        fake.calls().is_empty(),
        "approval mode must not touch Stripe"
    );
}

// ---------------------------------------------------------------------
// 4.2 Payment mode — checkout URL with the portal metadata contract
// ---------------------------------------------------------------------

#[tokio::test]
async fn payment_mode_returns_checkout_url_with_contract_metadata() {
    let pool = fresh_pool().await;
    insert_membership_type(&pool, PAID_SLUG, 4500).await;
    set_signup_mode(&pool, "payment").await;
    let (state, fake) = state_with_fake_stripe(&pool, None).await;
    let app = coterie::api::create_app(state);

    let (status, json) = post_signup(app, &signup_body("b@x.com", "bravo", PAID_SLUG)).await;

    assert_eq!(status, StatusCode::CREATED, "body: {json}");
    let url = json["checkout_url"].as_str().expect("checkout_url present");
    assert!(!url.is_empty());
    assert_eq!(member_status(&pool, "b@x.com").await, "Pending");

    let calls = fake.calls();
    let session = calls
        .iter()
        .find_map(|c| match c {
            coterie::payments::fake_gateway::FakeCall::CreateCheckoutSession(input) => Some(input),
            _ => None,
        })
        .expect("a checkout session was created");
    assert_eq!(
        session.metadata.get("payment_type").map(String::as_str),
        Some("membership"),
    );
    assert_eq!(
        session.metadata.get("membership_type_slug").map(String::as_str),
        Some(PAID_SLUG),
    );
    let member_id: String = sqlx::query_scalar("SELECT id FROM members WHERE email = 'b@x.com'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        session.metadata.get("member_id").map(String::as_str),
        Some(member_id.as_str()),
    );
    // The pending payment row exists for the webhook to complete later.
    let pending: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM payments WHERE member_id = ? AND status = 'Pending'",
    )
    .bind(&member_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pending, 1);
}

// ---------------------------------------------------------------------
// 4.3 Completed membership payment activates the Pending member and
//     dispatches the member-activated integration event
// ---------------------------------------------------------------------

struct RecordingIntegration {
    events: Mutex<Vec<String>>,
}

#[async_trait]
impl Integration for RecordingIntegration {
    fn name(&self) -> &str {
        "recording"
    }
    fn is_enabled(&self) -> bool {
        true
    }
    async fn health_check(&self) -> coterie::error::Result<()> {
        Ok(())
    }
    async fn handle_event(&self, event: &IntegrationEvent) -> coterie::error::Result<()> {
        let tag = match event {
            IntegrationEvent::MemberActivated(m) => format!("activated:{}", m.email),
            _ => "other".to_string(),
        };
        self.events.lock().unwrap().push(tag);
        Ok(())
    }
}

#[tokio::test]
async fn completed_payment_activates_pending_member_and_dispatches_event() {
    let pool = fresh_pool().await;
    insert_membership_type(&pool, PAID_SLUG, 4500).await;
    set_signup_mode(&pool, "payment").await;
    let (state, _fake) = state_with_fake_stripe(&pool, None).await;

    let recorder = Arc::new(RecordingIntegration {
        events: Mutex::new(Vec::new()),
    });
    state
        .service_context
        .integration_manager
        .register(recorder.clone())
        .await;

    let app = coterie::api::create_app(state.clone());
    let (status, _json) = post_signup(app, &signup_body("c@x.com", "charlie", PAID_SLUG)).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(member_status(&pool, "c@x.com").await, "Pending");

    // Drive the dues extension exactly as the checkout webhook does
    // (extend_member_dues_by_slug is the shared choke point for the
    // webhook, admin manual-record, and auto-renew paths).
    let member_id: String = sqlx::query_scalar("SELECT id FROM members WHERE email = 'c@x.com'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let payment_id: String = sqlx::query_scalar("SELECT id FROM payments WHERE member_id = ?")
        .bind(&member_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    state
        .billing_service
        .auto_renew
        .extend_member_dues_by_slug(
            uuid::Uuid::parse_str(&payment_id).unwrap(),
            uuid::Uuid::parse_str(&member_id).unwrap(),
            PAID_SLUG,
        )
        .await
        .expect("dues extension succeeds");

    assert_eq!(
        member_status(&pool, "c@x.com").await,
        "Active",
        "completed membership payment must activate the Pending member"
    );
    let dues: Option<String> =
        sqlx::query_scalar("SELECT dues_paid_until FROM members WHERE email = 'c@x.com'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(dues.is_some(), "dues_paid_until must be extended");
    assert_eq!(
        recorder.events.lock().unwrap().as_slice(),
        ["activated:c@x.com"],
        "the member-activated integration event must be dispatched once"
    );

    // Idempotency: a webhook retry re-running the extension must not
    // re-dispatch the activation event.
    state
        .billing_service
        .auto_renew
        .extend_member_dues_by_slug(
            uuid::Uuid::parse_str(&payment_id).unwrap(),
            uuid::Uuid::parse_str(&member_id).unwrap(),
            PAID_SLUG,
        )
        .await
        .expect("retry is a no-op");
    assert_eq!(recorder.events.lock().unwrap().len(), 1);
}

// ---------------------------------------------------------------------
// 4.4 Fee-0 type in payment mode stays in the approval funnel
// ---------------------------------------------------------------------

#[tokio::test]
async fn payment_mode_free_type_stays_pending_without_checkout() {
    let pool = fresh_pool().await;
    insert_membership_type(&pool, FREE_SLUG, 0).await;
    set_signup_mode(&pool, "payment").await;
    let (state, fake) = state_with_fake_stripe(&pool, None).await;
    let app = coterie::api::create_app(state);

    let (status, json) = post_signup(app, &signup_body("d@x.com", "delta", FREE_SLUG)).await;

    assert_eq!(status, StatusCode::CREATED);
    assert!(json.get("checkout_url").is_none());
    assert_eq!(member_status(&pool, "d@x.com").await, "Pending");
    assert!(fake.calls().is_empty());
}

// ---------------------------------------------------------------------
// 4.5 Gate order: money limiter precedes the bot challenge
// ---------------------------------------------------------------------

/// Denies every request and counts how many times the provider was
/// consulted — the ordering probe.
struct CountingDenyVerifier {
    calls: AtomicUsize,
}

#[async_trait]
impl BotChallengeVerifier for CountingDenyVerifier {
    async fn verify(
        &self,
        _route: &'static str,
        _token: Option<&str>,
        _client_ip: Option<IpAddr>,
    ) -> Result<(), VerifyError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(VerifyError::Missing)
    }
}

#[tokio::test]
async fn payment_mode_rate_limit_precedes_bot_challenge() {
    let pool = fresh_pool().await;
    insert_membership_type(&pool, PAID_SLUG, 4500).await;
    set_signup_mode(&pool, "payment").await;

    let verifier = Arc::new(CountingDenyVerifier {
        calls: AtomicUsize::new(0),
    });
    let (state, _fake) = state_with_fake_stripe(&pool, Some(verifier.clone())).await;
    let app = coterie::api::create_app(state);

    // The common harness money limiter allows 10/min per IP. The first
    // 10 requests reach (and fail) the bot challenge; the 11th must be
    // rate-limited WITHOUT consulting the challenge provider.
    let body = signup_body("e@x.com", "echo", PAID_SLUG);
    for i in 0..10 {
        let (status, _) = post_signup(app.clone(), &body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "request {i} fails the challenge");
    }
    assert_eq!(verifier.calls.load(Ordering::SeqCst), 10);

    let (status, _) = post_signup(app, &body).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "11th request is rate-limited");
    assert_eq!(
        verifier.calls.load(Ordering::SeqCst),
        10,
        "the rate-limited request must not consult the bot-challenge provider"
    );
}

// Approval mode (the default) moves no money, but the money limiter now
// applies there too — the same gate order (limiter before bot challenge)
// must hold. Mirrors the payment-mode probe above.
#[tokio::test]
async fn approval_mode_rate_limit_precedes_bot_challenge() {
    let pool = fresh_pool().await;
    insert_membership_type(&pool, FREE_SLUG, 0).await;
    set_signup_mode(&pool, "approval").await;

    let verifier = Arc::new(CountingDenyVerifier {
        calls: AtomicUsize::new(0),
    });
    let (state, _fake) = state_with_fake_stripe(&pool, Some(verifier.clone())).await;
    let app = coterie::api::create_app(state);

    let body = signup_body("g@x.com", "golf", FREE_SLUG);
    for i in 0..10 {
        let (status, _) = post_signup(app.clone(), &body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "request {i} fails the challenge");
    }
    assert_eq!(verifier.calls.load(Ordering::SeqCst), 10);

    let (status, _) = post_signup(app, &body).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "11th request is rate-limited");
    assert_eq!(
        verifier.calls.load(Ordering::SeqCst),
        10,
        "the rate-limited request must not consult the bot-challenge provider"
    );
}

// ---------------------------------------------------------------------
// 4.6 Abandoned-checkout retry
// ---------------------------------------------------------------------

#[tokio::test]
async fn duplicate_signup_retry_rules() {
    let pool = fresh_pool().await;
    insert_membership_type(&pool, PAID_SLUG, 4500).await;
    set_signup_mode(&pool, "payment").await;
    let (state, fake) = state_with_fake_stripe(&pool, None).await;
    let app = coterie::api::create_app(state);

    let body = signup_body("f@x.com", "foxtrot", PAID_SLUG);
    let (status, first) = post_signup(app.clone(), &body).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(first["checkout_url"].as_str().is_some());

    // Correct password + still Pending + no completed payment → fresh
    // checkout, no second member.
    let (status, retry) = post_signup(app.clone(), &body).await;
    assert_eq!(status, StatusCode::OK, "body: {retry}");
    assert!(retry["checkout_url"].as_str().is_some());
    let members: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM members")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(members, 1, "retry must not create a second member");
    let sessions = fake.count_where(|c| {
        matches!(
            c,
            coterie::payments::fake_gateway::FakeCall::CreateCheckoutSession(_)
        )
    });
    assert_eq!(sessions, 2, "retry mints a fresh checkout session");

    // Wrong password → the generic duplicate outcome, no new session.
    let mut wrong = body.clone();
    wrong["password"] = serde_json::json!("WrongPassw0rd!");
    let (status, _) = post_signup(app.clone(), &wrong).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        fake.count_where(|c| matches!(
            c,
            coterie::payments::fake_gateway::FakeCall::CreateCheckoutSession(_)
        )),
        2,
        "wrong password must not mint a session"
    );

    // Completed payment on record → duplicate outcome even with the
    // right password.
    sqlx::query("UPDATE payments SET status = 'Completed' WHERE member_id = (SELECT id FROM members WHERE email = 'f@x.com') AND id IN (SELECT id FROM payments WHERE member_id = (SELECT id FROM members WHERE email = 'f@x.com') LIMIT 1)")
        .execute(&pool)
        .await
        .unwrap();
    let (status, _) = post_signup(app, &body).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

// ---------------------------------------------------------------------
// 4.6b Retry anti-enumeration: every non-resuming duplicate yields the
// SAME 409 body, across all three early-return branches — wrong
// password, unknown email (username-only collision), and non-Pending
// status. The handler also runs `AuthService::verify_dummy` on the
// email-not-found and non-Pending branches so their Argon2 latency
// matches the wrong-password branch, closing the timing side-channel.
// Timing itself is inherently flaky to assert; this test locks the
// observable-body invariant and exercises those dummy-verify call sites.
// ---------------------------------------------------------------------

#[tokio::test]
async fn duplicate_signup_409_is_identical_across_branches() {
    let pool = fresh_pool().await;
    insert_membership_type(&pool, PAID_SLUG, 4500).await;
    set_signup_mode(&pool, "payment").await;
    let (state, _fake) = state_with_fake_stripe(&pool, None).await;
    let app = coterie::api::create_app(state);

    // Seed a Pending member (alpha) and a to-be-Active member (bravo).
    let (status, _) = post_signup(app.clone(), &signup_body("a@x.com", "alpha", PAID_SLUG)).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = post_signup(app.clone(), &signup_body("b@x.com", "bravo", PAID_SLUG)).await;
    assert_eq!(status, StatusCode::CREATED);
    sqlx::query("UPDATE members SET status = 'Active' WHERE email = 'b@x.com'")
        .execute(&pool)
        .await
        .unwrap();

    // (c) Wrong password on the Pending member — the only branch that
    // ran Argon2 before the fix.
    let mut wrong_pw = signup_body("a@x.com", "alpha", PAID_SLUG);
    wrong_pw["password"] = serde_json::json!("WrongPassw0rd!");
    let (s_wrong, b_wrong) = post_signup_raw(app.clone(), &wrong_pw).await;

    // (a) Unknown email but colliding username → username unique
    // violation, find_by_email misses → verify_dummy, then Ok(None).
    let (s_unknown, b_unknown) =
        post_signup_raw(app.clone(), &signup_body("nobody@x.com", "alpha", PAID_SLUG)).await;

    // (b) Email matches a non-Pending (Active) member → verify_dummy,
    // then Ok(None).
    let (s_active, b_active) =
        post_signup_raw(app.clone(), &signup_body("b@x.com", "bravo", PAID_SLUG)).await;

    assert_eq!(s_wrong, StatusCode::CONFLICT);
    assert_eq!(s_unknown, StatusCode::CONFLICT);
    assert_eq!(s_active, StatusCode::CONFLICT);
    assert_eq!(
        b_wrong, b_unknown,
        "unknown-email 409 body must be byte-identical to the wrong-password 409 body"
    );
    assert_eq!(
        b_wrong, b_active,
        "non-Pending 409 body must be byte-identical to the wrong-password 409 body"
    );
}

// ---------------------------------------------------------------------
// 5.3 Retry reuses the open session / supersedes the stale one
// ---------------------------------------------------------------------

#[tokio::test]
async fn retry_reuses_open_session_without_new_payment_row() {
    let pool = fresh_pool().await;
    insert_membership_type(&pool, PAID_SLUG, 4500).await;
    set_signup_mode(&pool, "payment").await;
    let (state, fake) = state_with_fake_stripe(&pool, None).await;
    let app = coterie::api::create_app(state);

    let body = signup_body("g@x.com", "golf", PAID_SLUG);
    let (status, _) = post_signup(app.clone(), &body).await;
    assert_eq!(status, StatusCode::CREATED);

    // The previous session is still open on Stripe.
    fake.next_retrieve_checkout_session(
        coterie::payments::gateway::RetrievedCheckoutSession {
            payment_intent_id: None,
            is_open: true,
            url: Some("https://stripe.test/still-open".to_string()),
        },
    );

    let (status, retry) = post_signup(app, &body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        retry["checkout_url"].as_str(),
        Some("https://stripe.test/still-open"),
        "the open session's URL is reused"
    );

    let payments: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM payments")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(payments, 1, "no duplicate pending payment row");
    let sessions = fake.count_where(|c| {
        matches!(
            c,
            coterie::payments::fake_gateway::FakeCall::CreateCheckoutSession(_)
        )
    });
    assert_eq!(sessions, 1, "no second session minted");
}

#[tokio::test]
async fn retry_supersedes_stale_session_with_failed_row() {
    let pool = fresh_pool().await;
    insert_membership_type(&pool, PAID_SLUG, 4500).await;
    set_signup_mode(&pool, "payment").await;
    let (state, fake) = state_with_fake_stripe(&pool, None).await;
    let app = coterie::api::create_app(state);

    let body = signup_body("h@x.com", "hotel", PAID_SLUG);
    let (status, _) = post_signup(app.clone(), &body).await;
    assert_eq!(status, StatusCode::CREATED);

    // Previous session expired (fake default retrieve: is_open=false).
    let (status, retry) = post_signup(app, &body).await;
    assert_eq!(status, StatusCode::OK);
    assert!(retry["checkout_url"].as_str().is_some(), "fresh session");

    let (failed, pending): (i64, i64) = (
        sqlx::query_scalar("SELECT COUNT(*) FROM payments WHERE status = 'Failed'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT COUNT(*) FROM payments WHERE status = 'Pending'")
            .fetch_one(&pool)
            .await
            .unwrap(),
    );
    assert_eq!(failed, 1, "stale row superseded to Failed");
    assert_eq!(pending, 1, "exactly one live pending row");
}

// ---------------------------------------------------------------------
// 6.5 Auto-renew enrollment flag on the session
// ---------------------------------------------------------------------

#[tokio::test]
async fn signup_session_carries_customer_and_save_card_by_default() {
    let pool = fresh_pool().await;
    insert_membership_type(&pool, PAID_SLUG, 4500).await;
    set_signup_mode(&pool, "payment").await;
    let (state, fake) = state_with_fake_stripe(&pool, None).await;
    let app = coterie::api::create_app(state);

    let (status, _) = post_signup(app, &signup_body("i@x.com", "india", PAID_SLUG)).await;
    assert_eq!(status, StatusCode::CREATED);

    let calls = fake.calls();
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, coterie::payments::fake_gateway::FakeCall::CreateCustomer(_))),
        "a Stripe customer is created for the member"
    );
    let session = calls
        .iter()
        .find_map(|c| match c {
            coterie::payments::fake_gateway::FakeCall::CreateCheckoutSession(input) => Some(input),
            _ => None,
        })
        .expect("session created");
    assert!(session.customer_id.is_some(), "session bound to the customer");
    assert!(session.save_card_for_offsession, "card saved off-session");
    assert_eq!(
        session.metadata.get("save_card").map(String::as_str),
        Some("true"),
        "webhook enrollment keys on this stamp"
    );
}

#[tokio::test]
async fn signup_auto_renew_off_keeps_one_off_checkout() {
    let pool = fresh_pool().await;
    insert_membership_type(&pool, PAID_SLUG, 4500).await;
    set_signup_mode(&pool, "payment").await;
    sqlx::query(
        "UPDATE app_settings SET value = 'false' WHERE key = 'membership.signup_auto_renew'",
    )
    .execute(&pool)
    .await
    .expect("signup_auto_renew row seeded by migration 038");
    let (state, fake) = state_with_fake_stripe(&pool, None).await;
    let app = coterie::api::create_app(state);

    let (status, _) = post_signup(app, &signup_body("j@x.com", "juliet", PAID_SLUG)).await;
    assert_eq!(status, StatusCode::CREATED);

    let calls = fake.calls();
    assert!(
        !calls
            .iter()
            .any(|c| matches!(c, coterie::payments::fake_gateway::FakeCall::CreateCustomer(_))),
        "no customer creation when the setting is off"
    );
    let session = calls
        .iter()
        .find_map(|c| match c {
            coterie::payments::fake_gateway::FakeCall::CreateCheckoutSession(input) => Some(input),
            _ => None,
        })
        .expect("session created");
    assert!(session.customer_id.is_none());
    assert!(!session.save_card_for_offsession);
    assert!(session.metadata.get("save_card").is_none());
}
