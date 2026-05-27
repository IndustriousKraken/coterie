//! End-to-end test for the tax-prep CSV report. Drives the real
//! HTTP route (`GET /portal/admin/finance/reports/tax-prep?year=…`)
//! against a fixture year and parses the response body.

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
        PaymentStatus, StripeRef, UpdateMemberRequest,
    },
};
use sqlx::SqlitePool;
use tower::ServiceExt;
use uuid::Uuid;

mod common;
use common::{build_app_state, fresh_pool};

async fn create_admin_session(state: &AppState) -> String {
    let suffix = Uuid::new_v4();
    let m = state
        .service_context
        .member_repo
        .create(CreateMemberRequest {
            email: format!("u-{}@example.com", suffix),
            username: format!("u_{}", suffix.simple()),
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

async fn insert_payment(
    pool: &SqlitePool,
    state: &AppState,
    member_id: Uuid,
    amount_cents: i64,
    kind: PaymentKind,
    status: PaymentStatus,
    paid_at: chrono::DateTime<Utc>,
    stripe_id: Option<String>,
) -> Uuid {
    let id = Uuid::new_v4();
    let payment = Payment {
        id,
        payer: Payer::Member(member_id),
        amount_cents,
        currency: "USD".into(),
        status: PaymentStatus::Completed,
        payment_method: PaymentMethod::Stripe,
        external_id: stripe_id.as_deref().and_then(StripeRef::from_id),
        description: format!("desc-{}", id.simple()),
        kind,
        paid_at: Some(paid_at),
        created_at: paid_at,
        updated_at: paid_at,
    };
    state
        .service_context
        .payment_repo
        .create(payment)
        .await
        .unwrap();
    // Backdate paid_at + flip status to whatever the test wanted.
    let status_str = match status {
        PaymentStatus::Pending => "Pending",
        PaymentStatus::Completed => "Completed",
        PaymentStatus::Failed => "Failed",
        PaymentStatus::Refunded => "Refunded",
    };
    sqlx::query("UPDATE payments SET status = ?, paid_at = ?, updated_at = ? WHERE id = ?")
        .bind(status_str)
        .bind(paid_at.naive_utc())
        .bind(paid_at.naive_utc())
        .bind(id.to_string())
        .execute(pool)
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn tax_prep_csv_contains_expected_rows() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    let session = create_admin_session(&state).await;

    // Spec scenario: one $150 payment, one $50 donation, one $25
    // refund, one $30 expense. All in 2026.
    let member = state
        .service_context
        .member_repo
        .create(CreateMemberRequest {
            email: "payer@example.com".into(),
            username: "payer".into(),
            full_name: "Pay Er".into(),
            password: "p4ssword_long_enough".into(),
            membership_type_id: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let d_pay = Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap();
    let d_don = Utc.with_ymd_and_hms(2026, 4, 1, 12, 0, 0).unwrap();
    let d_ref = Utc.with_ymd_and_hms(2026, 5, 1, 12, 0, 0).unwrap();
    let d_exp = Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap();

    insert_payment(
        &pool,
        &state,
        member.id,
        150_00,
        PaymentKind::Membership,
        PaymentStatus::Completed,
        d_pay,
        Some("pi_payment".into()),
    )
    .await;
    insert_payment(
        &pool,
        &state,
        member.id,
        50_00,
        PaymentKind::Donation { campaign_id: None },
        PaymentStatus::Completed,
        d_don,
        Some("pi_donation".into()),
    )
    .await;
    insert_payment(
        &pool,
        &state,
        member.id,
        25_00,
        PaymentKind::Membership,
        PaymentStatus::Refunded,
        d_ref,
        Some("pi_refund".into()),
    )
    .await;

    // One expense.
    let cat = state
        .service_context
        .expense_category_repo
        .create(CreateExpenseCategoryRequest {
            name: "Supplies".into(),
            slug: None,
        })
        .await
        .unwrap();
    let acc = state
        .service_context
        .expense_account_repo
        .create(CreateExpenseAccountRequest {
            name: "Card 1".into(),
        })
        .await
        .unwrap();
    state
        .service_context
        .expense_repo
        .create(
            member.id,
            CreateExpenseRequest {
                spent_at: d_exp,
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

    // Header + 4 data rows = 5 lines, with a trailing newline.
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(
        lines.len(),
        5,
        "expected header + 4 rows, got {}: {}",
        lines.len(),
        body
    );

    assert!(lines[0].starts_with("date,type,amount"));

    let payment_row = lines
        .iter()
        .find(|l| l.contains(",\"payment\","))
        .expect("payment row");
    assert!(payment_row.contains("\"150.00\""), "{}", payment_row);
    let donation_row = lines
        .iter()
        .find(|l| l.contains(",\"donation\","))
        .expect("donation row");
    assert!(donation_row.contains("\"50.00\""), "{}", donation_row);
    let refund_row = lines
        .iter()
        .find(|l| l.contains(",\"refund\","))
        .expect("refund row");
    assert!(refund_row.contains("\"-25.00\""), "{}", refund_row);
    let expense_row = lines
        .iter()
        .find(|l| l.contains(",\"expense\","))
        .expect("expense row");
    assert!(expense_row.contains("\"30.00\""), "{}", expense_row);
}

#[tokio::test]
async fn tax_prep_csv_sorts_by_date() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    let session = create_admin_session(&state).await;

    let member = state
        .service_context
        .member_repo
        .create(CreateMemberRequest {
            email: "payer2@example.com".into(),
            username: "payer2".into(),
            full_name: "Pay Er".into(),
            password: "p4ssword_long_enough".into(),
            membership_type_id: None,
            ..Default::default()
        })
        .await
        .unwrap();

    // Insert in non-chronological order — the CSV must sort them.
    let dates = [
        Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap(),
    ];
    for (i, d) in dates.iter().enumerate() {
        insert_payment(
            &pool,
            &state,
            member.id,
            (10 + i as i64) * 100,
            PaymentKind::Membership,
            PaymentStatus::Completed,
            *d,
            Some(format!("pi_{}", i)),
        )
        .await;
    }

    let app = coterie::web::create_web_routes(state.clone());
    let req = Request::builder()
        .method("GET")
        .uri("/portal/admin/finance/reports/tax-prep?year=2026")
        .header(header::COOKIE, format!("session={}", session))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body_bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let body = String::from_utf8(body_bytes.to_vec()).unwrap();

    let data_lines: Vec<&str> = body.lines().skip(1).collect();
    assert_eq!(data_lines.len(), 3);
    // Each line begins with `"YYYY-MM-DD"`,…  — compare the prefix.
    let dates_in_csv: Vec<String> = data_lines
        .iter()
        .map(|l| l.split(',').next().unwrap().trim_matches('"').to_string())
        .collect();
    assert_eq!(
        dates_in_csv,
        vec!["2026-01-01", "2026-05-01", "2026-08-01"],
        "CSV rows must be sorted by date ASC",
    );
}
