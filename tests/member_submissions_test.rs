//! Web/security-layer regression tests for member-proposal-submissions.
//!
//! Service-layer invariants (create/bounds/cap/ownership/promotion) are
//! covered by unit tests in `submission_service`. These exercise the
//! HTTP surface: the toggle gate, IDOR at the route, stored-XSS escaping
//! in the admin view, attachment authorization, and CSRF.

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use chrono::Utc;
use coterie::{
    api::state::AppState,
    domain::{
        CreateMemberRequest, MemberStatus, Submission, SubmissionStatus, SubmissionVisibility,
        UpdateMemberRequest,
    },
};
use tower::ServiceExt;
use uuid::Uuid;

mod common;
use common::{build_app_state, fresh_pool};

async fn enable_submissions(state: &AppState) {
    sqlx::query("UPDATE app_settings SET value = 'true' WHERE key = 'submissions.enabled'")
        .execute(&state.service_context.db_pool)
        .await
        .expect("enable submissions toggle");
}

/// Create an Active (optionally admin) member and return (id, session).
async fn make_session(state: &AppState, is_admin: bool) -> (Uuid, String) {
    let suffix = Uuid::new_v4();
    let member = state
        .service_context
        .member_repo
        .create(CreateMemberRequest {
            email: format!("u-{}@example.com", suffix),
            username: format!("u_{}", suffix.simple()),
            full_name: "Test Member".into(),
            password: "p4ssword_long_enough".into(),
            membership_type_id: None,
            ..Default::default()
        })
        .await
        .expect("create member");
    state
        .service_context
        .member_repo
        .update(
            member.id,
            UpdateMemberRequest {
                status: Some(MemberStatus::Active),
                ..Default::default()
            },
        )
        .await
        .expect("activate member");
    if is_admin {
        state
            .service_context
            .member_repo
            .set_admin(member.id, true)
            .await
            .expect("set admin");
    }
    let (_, token) = state
        .service_context
        .auth_service
        .create_session(member.id, 24)
        .await
        .expect("create session");
    (member.id, token)
}

/// Insert a submission row directly (bypassing the service) so tests can
/// pin an exact owner/status/visibility/attachment.
async fn insert_submission(
    state: &AppState,
    owner: Uuid,
    title: &str,
    status: SubmissionStatus,
    visibility: SubmissionVisibility,
    attachment_path: Option<String>,
) -> Uuid {
    let now = Utc::now();
    let submission = Submission {
        id: Uuid::new_v4(),
        submitter_member_id: owner,
        title: title.to_string(),
        abstract_text: "Body".to_string(),
        visibility_requested: visibility,
        attachment_path,
        preferred_start: None,
        timezone: "UTC".to_string(),
        duration_minutes: None,
        status,
        reviewer_note: None,
        decided_by: None,
        event_id: None,
        created_at: now,
        updated_at: now,
    };
    state
        .service_context
        .submission_repo
        .create(submission)
        .await
        .expect("insert submission")
        .id
}

fn get(uri: &str, session: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().method("GET").uri(uri);
    if let Some(t) = session {
        b = b.header(header::COOKIE, format!("session={}", t));
    }
    b.body(Body::empty()).unwrap()
}

async fn body_string(resp: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8_lossy(&bytes).to_string()
}

// --- 5.8 Toggle off → routes 404, no entry point ------------------------

#[tokio::test]
async fn toggle_off_submission_routes_are_not_found() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool).await;
    let (_, member) = make_session(&state, false).await;
    let (_, admin) = make_session(&state, true).await;
    let app = coterie::web::create_web_routes(state.clone());

    // Default: submissions.enabled = false → routes 404 for authorized callers.
    for (path, session) in [
        ("/portal/submissions", &member),
        ("/portal/submissions/new", &member),
        ("/portal/admin/submissions", &admin),
    ] {
        let resp = app.clone().oneshot(get(path, Some(session))).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "{} should 404 when the toggle is off",
            path
        );
    }

    // No entry point on the dashboard.
    let resp = app
        .clone()
        .oneshot(get("/portal/dashboard", Some(&member)))
        .await
        .unwrap();
    let body = body_string(resp).await;
    assert!(
        !body.contains("/portal/submissions"),
        "dashboard must show no submissions entry point when the toggle is off"
    );
}

#[tokio::test]
async fn toggle_on_member_reaches_list_and_dashboard_link() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool).await;
    enable_submissions(&state).await;
    let (_, member) = make_session(&state, false).await;
    let app = coterie::web::create_web_routes(state.clone());

    let resp = app
        .clone()
        .oneshot(get("/portal/submissions", Some(&member)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = app
        .clone()
        .oneshot(get("/portal/dashboard", Some(&member)))
        .await
        .unwrap();
    let body = body_string(resp).await;
    assert!(
        body.contains("/portal/submissions"),
        "dashboard should link to submissions when the toggle is on"
    );
}

// --- 5.2 IDOR: cross-member read denied without disclosure --------------

#[tokio::test]
async fn cross_member_read_is_denied_at_route() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool).await;
    enable_submissions(&state).await;
    let (member_b, _) = make_session(&state, false).await;
    let (_, session_a) = make_session(&state, false).await;
    let id = insert_submission(
        &state,
        member_b,
        "B's secret proposal",
        SubmissionStatus::Submitted,
        SubmissionVisibility::Members,
        None,
    )
    .await;
    let app = coterie::web::create_web_routes(state.clone());

    let resp = app
        .clone()
        .oneshot(get(
            &format!("/portal/submissions/{}", id),
            Some(&session_a),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body = body_string(resp).await;
    assert!(
        !body.contains("B's secret proposal"),
        "denied read must not disclose the submission's contents"
    );

    // Owner can read it.
    let (_, session_b) = {
        // reuse member_b by creating a session for them
        let (_, tok) = state
            .service_context
            .auth_service
            .create_session(member_b, 24)
            .await
            .unwrap();
        ((), tok)
    };
    let resp = app
        .oneshot(get(
            &format!("/portal/submissions/{}", id),
            Some(&session_b),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// --- 5.3 Stored XSS: script-title inert in the admin view ---------------

#[tokio::test]
async fn script_title_is_escaped_in_admin_detail() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool).await;
    enable_submissions(&state).await;
    let (owner, _) = make_session(&state, false).await;
    let (_, admin) = make_session(&state, true).await;
    let payload = "<script>alert(document.cookie)</script>";
    let id = insert_submission(
        &state,
        owner,
        payload,
        SubmissionStatus::Submitted,
        SubmissionVisibility::Members,
        None,
    )
    .await;
    let app = coterie::web::create_web_routes(state.clone());

    let resp = app
        .oneshot(get(
            &format!("/portal/admin/submissions/{}", id),
            Some(&admin),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp).await;
    assert!(
        !body.contains(payload),
        "raw <script> payload must not appear as active markup"
    );
    assert!(
        body.contains("&lt;script&gt;alert(document.cookie)&lt;/script&gt;"),
        "title must render HTML-escaped in the admin view"
    );
}

// --- 5.4 Attachment authorization + Content-Disposition -----------------

#[tokio::test]
async fn attachment_authorization_is_enforced() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool).await;
    enable_submissions(&state).await;
    let uploads_dir = state.settings.server.uploads_path();

    // A private (members) submission owned by B with a real PDF on disk.
    let private_path = coterie::web::uploads::save_uploaded_document(&uploads_dir, b"%PDF-1.4 hi")
        .await
        .expect("save pdf");
    let (owner_b, _) = make_session(&state, false).await;
    let (_, session_c) = make_session(&state, false).await; // non-owner, non-admin
    let private_id = insert_submission(
        &state,
        owner_b,
        "Private",
        SubmissionStatus::Submitted,
        SubmissionVisibility::Members,
        Some(private_path),
    )
    .await;

    let app = coterie::web::create_web_routes(state.clone());

    // Non-owner, non-reviewer → denied.
    let resp = app
        .clone()
        .oneshot(get(
            &format!("/portal/submissions/{}/attachment", private_id),
            Some(&session_c),
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "non-owner must not fetch a private attachment"
    );

    // Owner → allowed, served as an attachment download.
    let (_, session_b) = {
        let (_, tok) = state
            .service_context
            .auth_service
            .create_session(owner_b, 24)
            .await
            .unwrap();
        ((), tok)
    };
    let resp = app
        .clone()
        .oneshot(get(
            &format!("/portal/submissions/{}/attachment", private_id),
            Some(&session_b),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let disp = resp
        .headers()
        .get(header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        disp.contains("attachment"),
        "attachment must be served with Content-Disposition: attachment, got {:?}",
        disp
    );

    // An accepted + public attachment is reachable by a non-owner.
    let public_path = coterie::web::uploads::save_uploaded_document(&uploads_dir, b"%PDF-1.4 pub")
        .await
        .expect("save pdf");
    let public_id = insert_submission(
        &state,
        owner_b,
        "Accepted public",
        SubmissionStatus::Accepted,
        SubmissionVisibility::Public,
        Some(public_path),
    )
    .await;
    let resp = app
        .oneshot(get(
            &format!("/portal/submissions/{}/attachment", public_id),
            Some(&session_c),
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "an accepted+public attachment should be reachable by any member"
    );
}

// --- 5.6 Member cannot drive an admin decision --------------------------

#[tokio::test]
async fn member_cannot_reach_admin_accept_route() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool).await;
    enable_submissions(&state).await;
    let (owner, session) = make_session(&state, false).await;
    let id = insert_submission(
        &state,
        owner,
        "Mine",
        SubmissionStatus::Submitted,
        SubmissionVisibility::Public,
        None,
    )
    .await;
    let app = coterie::web::create_web_routes(state.clone());

    // GET the admin detail as a plain member → redirected off the admin
    // surface (never served the review UI).
    let resp = app
        .oneshot(get(
            &format!("/portal/admin/submissions/{}", id),
            Some(&session),
        ))
        .await
        .unwrap();
    assert!(
        matches!(
            resp.status(),
            StatusCode::SEE_OTHER | StatusCode::FOUND | StatusCode::TEMPORARY_REDIRECT
        ),
        "a non-admin must be redirected off the admin review route, got {}",
        resp.status()
    );
    // Status unchanged.
    let s = state
        .service_context
        .submission_repo
        .find_by_id(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.status, SubmissionStatus::Submitted);
}

// --- 5.7 CSRF: a withdraw POST without a token is rejected --------------

/// Build the full merged app (mirrors main.rs) so the top-level CSRF
/// layer covers the merged `/portal/*` routes.
fn full_app(state: AppState) -> Router {
    let api_app = coterie::api::create_app(state.clone());
    let web_app = coterie::web::create_web_routes(state.clone());
    api_app
        .merge(web_app)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            coterie::api::middleware::setup::require_setup,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state,
            coterie::api::middleware::security::csrf_protect_unless_exempt,
        ))
}

#[tokio::test]
async fn withdraw_without_csrf_token_is_rejected() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool).await;
    enable_submissions(&state).await;
    let app = full_app(state.clone());

    let req = Request::builder()
        .method("POST")
        .uri(format!("/portal/submissions/{}/withdraw", Uuid::new_v4()))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "a withdraw POST without a CSRF token must be rejected"
    );
}
