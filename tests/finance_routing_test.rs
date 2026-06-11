//! Routing-level tests for `/portal/admin/finance/*`. Confirms the
//! shared `require_admin_redirect` middleware bounces non-admin and
//! unauthenticated callers.

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use coterie::{
    api::state::AppState,
    domain::{CreateMemberRequest, MemberStatus, UpdateMemberRequest},
};
use tower::ServiceExt;
use uuid::Uuid;

mod common;
use common::{build_app_state, fresh_pool};

async fn make_active_session(state: &AppState, is_admin: bool) -> String {
    let suffix = Uuid::new_v4();
    let member = state
        .service_context
        .member_repo
        .create(CreateMemberRequest {
            email: format!("u-{}@example.com", suffix),
            username: format!("u_{}", suffix.simple()),
            full_name: "Test".into(),
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
    token
}

fn req(uri: &str, session: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(t) = session {
        builder = builder.header(header::COOKIE, format!("session={}", t));
    }
    builder.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn unauthenticated_finance_routes_redirect_to_login() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool).await;
    let app = coterie::web::create_web_routes(state.clone());

    for path in [
        "/portal/admin/finance/expenses",
        "/portal/admin/finance/categories",
        "/portal/admin/finance/accounts",
        "/portal/admin/finance/reports/monthly",
    ] {
        let resp = app.clone().oneshot(req(path, None)).await.unwrap();
        assert!(
            matches!(
                resp.status(),
                StatusCode::SEE_OTHER | StatusCode::FOUND | StatusCode::TEMPORARY_REDIRECT
            ),
            "{} should redirect anonymous to /login (got {})",
            path,
            resp.status()
        );
        let loc = resp
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            loc.starts_with("/login"),
            "{} → {} should land at /login",
            path,
            loc
        );
    }
}

#[tokio::test]
async fn non_admin_finance_routes_redirect_to_dashboard() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool).await;
    let session = make_active_session(&state, false).await;
    let app = coterie::web::create_web_routes(state.clone());

    for path in [
        "/portal/admin/finance/expenses",
        "/portal/admin/finance/categories",
        "/portal/admin/finance/accounts",
    ] {
        let resp = app
            .clone()
            .oneshot(req(path, Some(&session)))
            .await
            .unwrap();
        assert!(
            matches!(
                resp.status(),
                StatusCode::SEE_OTHER | StatusCode::FOUND | StatusCode::TEMPORARY_REDIRECT
            ),
            "{} should redirect non-admin",
            path
        );
        let loc = resp
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            loc.starts_with("/portal/dashboard"),
            "{} → {} should land at /portal/dashboard",
            path,
            loc
        );
    }
}

#[tokio::test]
async fn admin_finance_routes_render_ok() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool).await;
    let session = make_active_session(&state, true).await;
    let app = coterie::web::create_web_routes(state.clone());

    for path in [
        "/portal/admin/finance/expenses",
        "/portal/admin/finance/categories",
        "/portal/admin/finance/accounts",
        "/portal/admin/finance/reports/monthly",
        "/portal/admin/finance/reports/annual",
    ] {
        let resp = app
            .clone()
            .oneshot(req(path, Some(&session)))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "{} should render 200 for admin (got {})",
            path,
            resp.status()
        );
    }
}
