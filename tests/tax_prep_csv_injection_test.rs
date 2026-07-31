//! CWE-1236 regression for the tax-prep CSV export
//! (`GET /portal/admin/finance/reports/tax-prep`). A public donor picks
//! their own `donor_name` through the anonymous `POST /public/donate`
//! body, and that string lands in the export's `counterparty` /
//! `description` cells; an expense category / account name lands in
//! `category` / `account`. All four must be neutralized so the
//! treasurer's spreadsheet renders them as text, while the
//! server-controlled columns (notably a refund's negative `amount`)
//! stay byte-for-byte as before.
//!
//! Run with: cargo test --features test-utils --test tax_prep_csv_injection_test

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use chrono::{TimeZone, Utc};
use coterie::{
    api::state::AppState,
    domain::{
        CreateExpenseAccountRequest, CreateExpenseCategoryRequest, CreateExpenseRequest,
        CreateMemberRequest, MemberStatus, Payer, Payment, PaymentKind, PaymentMethod,
        PaymentStatus, UpdateMemberRequest,
    },
};
use tower::ServiceExt;
use uuid::Uuid;

mod common;
use common::{build_app_state, fresh_pool};

/// The donor-supplied payload. Contains `"` so the assertions also
/// pin the RFC 4180 doubling that happens around the neutralization.
const PAYLOAD: &str = r#"=HYPERLINK("http://evil","x")"#;

async fn create_admin_session(state: &AppState) -> String {
    let suffix = Uuid::new_v4();
    let m = state
        .service_context
        .member_repo
        .create(CreateMemberRequest {
            email: format!("admin-{}@example.com", suffix),
            username: format!("a_{}", suffix.simple()),
            full_name: "Admin".into(),
            password: "p4ssword_long_enough".into(),
            membership_type_id: None,
            ..Default::default()
        })
        .await
        .unwrap();
    state
        .service_context
        .member_repo
        .update(
            m.id,
            UpdateMemberRequest {
                status: Some(MemberStatus::Active),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    state
        .service_context
        .member_repo
        .set_admin(m.id, true)
        .await
        .unwrap();
    let (_, tok) = state
        .service_context
        .auth_service
        .create_session(m.id, 24)
        .await
        .unwrap();
    tok
}

#[tokio::test]
async fn tax_prep_csv_neutralizes_formula_injection() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    let session = create_admin_session(&state).await;

    // ---- Row 1: a completed public donation from a hostile donor ----
    // `description` is seeded with the payload too: the column is
    // free text influenced by the same anonymous caller, so it has to
    // be neutralized on its own merits, not just because the Stripe
    // flow happens to prefix it with a product name.
    state
        .service_context
        .payment_repo
        .create(Payment {
            id: Uuid::new_v4(),
            payer: Payer::PublicDonor {
                name: PAYLOAD.into(),
                email: "evil@example.com".into(),
            },
            amount_cents: 100_00,
            currency: "USD".into(),
            status: PaymentStatus::Completed,
            payment_method: PaymentMethod::Stripe,
            external_id: None,
            description: PAYLOAD.into(),
            kind: PaymentKind::Donation { campaign_id: None },
            paid_at: Some(Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap()),
            created_at: Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap(),
        })
        .await
        .unwrap();

    // ---- Row 2: an ordinary member refund (the control row) --------
    let member = state
        .service_context
        .member_repo
        .create(CreateMemberRequest {
            email: "obrien@example.com".into(),
            username: "sobrien".into(),
            full_name: "O'Brien, Sean".into(),
            password: "p4ssword_long_enough".into(),
            membership_type_id: None,
            ..Default::default()
        })
        .await
        .unwrap();
    state
        .service_context
        .payment_repo
        .create(Payment {
            id: Uuid::new_v4(),
            payer: Payer::Member(member.id),
            amount_cents: 25_00,
            currency: "USD".into(),
            status: PaymentStatus::Refunded,
            payment_method: PaymentMethod::Stripe,
            external_id: None,
            description: "Refunded dues".into(),
            kind: PaymentKind::Membership,
            paid_at: Some(Utc.with_ymd_and_hms(2026, 5, 1, 12, 0, 0).unwrap()),
            created_at: Utc.with_ymd_and_hms(2026, 5, 1, 12, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 5, 1, 12, 0, 0).unwrap(),
        })
        .await
        .unwrap();

    // ---- Row 3: an expense whose category / account start with a trigger ----
    let cat = state
        .service_context
        .expense_category_repo
        .create(CreateExpenseCategoryRequest {
            name: "+Rebates".into(),
            slug: None,
        })
        .await
        .unwrap();
    let acc = state
        .service_context
        .expense_account_repo
        .create(CreateExpenseAccountRequest {
            name: "-Petty Cash".into(),
        })
        .await
        .unwrap();
    state
        .service_context
        .expense_repo
        .create(
            member.id,
            CreateExpenseRequest {
                spent_at: Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap(),
                amount_cents: 30_00,
                currency: None,
                description: "Sticky notes".into(),
                category_id: cat.id,
                account_id: acc.id,
                notes: None,
            },
        )
        .await
        .unwrap();

    let app = coterie::web::create_web_routes(state.clone());
    let req = Request::builder()
        .method("GET")
        .uri("/portal/admin/finance/reports/tax-prep?year=2026")
        .header(header::COOKIE, format!("session={}", session))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body_bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let body = String::from_utf8(body_bytes.to_vec()).unwrap();
    let lines: Vec<&str> = body.lines().collect();

    // ---- Donation row: description + counterparty neutralized -------
    let donation_row = lines
        .iter()
        .find(|l| l.contains(",\"donation\","))
        .unwrap_or_else(|| panic!("donation row missing:\n{}", body));
    // Both cells open `"'=` — a single quote directly after the
    // opening double-quote — with the payload's own quotes doubled.
    let quoted_payload = r#"'=HYPERLINK(""http://evil"",""x"")"#;
    assert!(
        donation_row.contains(&format!(",\"{}\",", quoted_payload)),
        "description cell must be neutralized, got:\n{}",
        donation_row,
    );
    assert!(
        donation_row.contains(&format!(",\"{} <evil@example.com>\",", quoted_payload)),
        "counterparty cell must be neutralized, got:\n{}",
        donation_row,
    );
    // No field anywhere may still start with a bare `=`.
    assert!(
        !body.contains(",\"=HYPERLINK"),
        "an un-neutralized formula field survived:\n{}",
        body,
    );

    // ---- Expense row: category + account neutralized ----------------
    let expense_row = lines
        .iter()
        .find(|l| l.contains(",\"expense\","))
        .unwrap_or_else(|| panic!("expense row missing:\n{}", body));
    assert!(
        expense_row.contains(",\"'+Rebates\","),
        "category cell must be neutralized, got:\n{}",
        expense_row,
    );
    assert!(
        expense_row.contains(",\"'-Petty Cash\","),
        "account cell must be neutralized, got:\n{}",
        expense_row,
    );

    // ---- Refund row: ordinary + server-controlled cells untouched ---
    let refund_row = lines
        .iter()
        .find(|l| l.contains(",\"refund\","))
        .unwrap_or_else(|| panic!("refund row missing:\n{}", body));
    assert!(
        refund_row.contains(",\"O'Brien, Sean\","),
        "an ordinary counterparty must not gain a leading quote, got:\n{}",
        refund_row,
    );
    // `amount` is server-controlled: the leading `-` stays a minus sign.
    assert!(
        refund_row.contains(",\"-25.00\","),
        "refund amount must stay a negative number, got:\n{}",
        refund_row,
    );
    assert!(
        !refund_row.contains("\"'-25.00\""),
        "refund amount must not be neutralized, got:\n{}",
        refund_row,
    );
}
