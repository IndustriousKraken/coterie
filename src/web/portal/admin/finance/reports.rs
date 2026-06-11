//! Monthly + annual reconciliation reports and the tax-prep CSV
//! export. All three combine expense data with the existing payments
//! ledger; nothing here mutates state.
//!
//! Reports are cash-basis: expenses count on their `spent_at` date;
//! payments / refunds count on their `paid_at`. The report templates
//! display this caveat so an operator comparing against a bank
//! statement understands what they're looking at.

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Extension,
};
use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    api::middleware::auth::{CurrentUser, SessionInfo},
    auth::CsrfService,
    repository::{DateRange, ExpenseRepository},
    service::{
        expense_account_service::ExpenseAccountService,
        expense_category_service::ExpenseCategoryService,
    },
    web::{
        portal::admin::{csv::push_csv, partials},
        templates::{BaseContext, HtmlTemplate},
    },
};

// =============================================================================
// Monthly report
// =============================================================================

#[derive(Debug, Clone)]
pub struct ReportRow {
    pub label: String,
    pub amount: String,
}

#[derive(Template)]
#[template(path = "admin/finance/report_monthly.html")]
pub struct MonthlyReportTemplate {
    pub base: BaseContext,
    pub year: i32,
    pub month: u32,
    pub period_label: String,
    pub account_rows: Vec<ReportRow>,
    pub category_rows: Vec<ReportRow>,
    pub expense_total: String,
    pub income_total: String,
    pub net: String,
    pub prev_year: i32,
    pub prev_month: u32,
    pub next_year: i32,
    pub next_month: u32,
}

#[derive(Debug, Deserialize)]
pub struct MonthlyQuery {
    pub year: Option<i32>,
    pub month: Option<u32>,
}

pub async fn monthly_report(
    State(expense_repo): State<Arc<dyn ExpenseRepository>>,
    State(category_service): State<Arc<ExpenseCategoryService>>,
    State(account_service): State<Arc<ExpenseAccountService>>,
    State(pool): State<SqlitePool>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Query(query): Query<MonthlyQuery>,
) -> Response {
    let now = Utc::now();
    let year = query.year.unwrap_or(now.year());
    let month = query.month.unwrap_or(now.month()).clamp(1, 12);

    let range = match month_range(year, month) {
        Some(r) => r,
        None => {
            return partials::admin_alert("error", "Invalid year / month", false).into_response()
        }
    };

    let categories = category_service.list(true).await.unwrap_or_default();
    let accounts = account_service.list(true).await.unwrap_or_default();

    let by_account = expense_repo.sum_by_account(range).await.unwrap_or_default();
    let by_category = expense_repo
        .sum_by_category(range)
        .await
        .unwrap_or_default();
    let expense_total = expense_repo.total_in_range(range).await.unwrap_or(0);

    let account_rows: Vec<ReportRow> = by_account
        .into_iter()
        .map(|s| ReportRow {
            label: accounts
                .iter()
                .find(|a| a.id == s.key_id)
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "(unknown account)".to_string()),
            amount: format_cents(s.total_cents),
        })
        .collect();
    let category_rows: Vec<ReportRow> = by_category
        .into_iter()
        .map(|s| ReportRow {
            label: categories
                .iter()
                .find(|c| c.id == s.key_id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "(unknown category)".to_string()),
            amount: format_cents(s.total_cents),
        })
        .collect();

    let income_total = income_in_range(&pool, range).await.unwrap_or(0);
    let net = income_total - expense_total;

    let (prev_year, prev_month) = previous_month(year, month);
    let (next_year, next_month) = next_month(year, month);

    HtmlTemplate(MonthlyReportTemplate {
        base: BaseContext::for_member(&csrf_service, &current_user, &session_info).await,
        year,
        month,
        period_label: format!("{} {}", month_name(month), year),
        account_rows,
        category_rows,
        expense_total: format_cents(expense_total),
        income_total: format_cents(income_total),
        net: format_cents(net),
        prev_year,
        prev_month,
        next_year,
        next_month,
    })
    .into_response()
}

// =============================================================================
// Annual report
// =============================================================================

#[derive(Template)]
#[template(path = "admin/finance/report_annual.html")]
pub struct AnnualReportTemplate {
    pub base: BaseContext,
    pub year: i32,
    pub category_rows: Vec<ReportRow>,
    pub dues_total: String,
    pub donations_total: String,
    pub other_total: String,
    pub income_total: String,
    pub expense_total: String,
    pub net: String,
}

#[derive(Debug, Deserialize)]
pub struct AnnualQuery {
    pub year: Option<i32>,
}

pub async fn annual_report(
    State(expense_repo): State<Arc<dyn ExpenseRepository>>,
    State(category_service): State<Arc<ExpenseCategoryService>>,
    State(pool): State<SqlitePool>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Query(query): Query<AnnualQuery>,
) -> Response {
    let year = query.year.unwrap_or_else(|| Utc::now().year());
    let range = match year_range(year) {
        Some(r) => r,
        None => return partials::admin_alert("error", "Invalid year", false).into_response(),
    };

    let categories = category_service.list(true).await.unwrap_or_default();
    let by_category = expense_repo
        .sum_by_category(range)
        .await
        .unwrap_or_default();
    let expense_total = expense_repo.total_in_range(range).await.unwrap_or(0);

    let category_rows: Vec<ReportRow> = by_category
        .into_iter()
        .map(|s| ReportRow {
            label: categories
                .iter()
                .find(|c| c.id == s.key_id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "(unknown category)".to_string()),
            amount: format_cents(s.total_cents),
        })
        .collect();

    let income_by_kind = income_by_kind_in_range(&pool, range)
        .await
        .unwrap_or_default();
    let dues_total = income_by_kind
        .iter()
        .find(|(k, _)| k == "membership")
        .map(|(_, v)| *v)
        .unwrap_or(0);
    let donations_total = income_by_kind
        .iter()
        .find(|(k, _)| k == "donation")
        .map(|(_, v)| *v)
        .unwrap_or(0);
    let other_total = income_by_kind
        .iter()
        .find(|(k, _)| k == "other")
        .map(|(_, v)| *v)
        .unwrap_or(0);
    let income_total = dues_total + donations_total + other_total;
    let net = income_total - expense_total;

    HtmlTemplate(AnnualReportTemplate {
        base: BaseContext::for_member(&csrf_service, &current_user, &session_info).await,
        year,
        category_rows,
        dues_total: format_cents(dues_total),
        donations_total: format_cents(donations_total),
        other_total: format_cents(other_total),
        income_total: format_cents(income_total),
        expense_total: format_cents(expense_total),
        net: format_cents(net),
    })
    .into_response()
}

// =============================================================================
// Tax-prep CSV
// =============================================================================

#[derive(Debug, Deserialize)]
pub struct TaxPrepQuery {
    pub year: Option<i32>,
}

/// Row used internally by `tax_prep_csv` to merge + sort across the
/// three sources before serializing. `date` carries the original
/// timestamp so sorting is unambiguous.
#[derive(Debug, Clone)]
struct CsvRow {
    date: DateTime<Utc>,
    type_str: &'static str,
    amount_cents: i64,
    description: String,
    counterparty: String,
    category: String,
    account: String,
    reference: String,
}

pub async fn tax_prep_csv(
    State(expense_repo): State<Arc<dyn ExpenseRepository>>,
    State(category_service): State<Arc<ExpenseCategoryService>>,
    State(account_service): State<Arc<ExpenseAccountService>>,
    State(pool): State<SqlitePool>,
    Extension(_current_user): Extension<CurrentUser>,
    Query(query): Query<TaxPrepQuery>,
) -> Response {
    let year = query.year.unwrap_or_else(|| Utc::now().year());
    let range = match year_range(year) {
        Some(r) => r,
        None => {
            return (StatusCode::BAD_REQUEST, "Invalid year").into_response();
        }
    };

    let mut rows: Vec<CsvRow> = Vec::new();

    // ---- Payments + donations + refunds ---------------------------------
    match payment_rows_for_year(&pool, range).await {
        Ok(mut v) => rows.append(&mut v),
        Err(e) => {
            tracing::error!("tax-prep CSV: payment query failed: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Query failure").into_response();
        }
    }

    // ---- Expenses --------------------------------------------------------
    let categories = category_service.list(true).await.unwrap_or_default();
    let accounts = account_service.list(true).await.unwrap_or_default();
    let expense_filter = crate::repository::ExpenseFilter {
        date_range: Some(range),
        ..Default::default()
    };
    let expenses = expense_repo.list(expense_filter).await.unwrap_or_default();
    for e in expenses {
        let category_name = categories
            .iter()
            .find(|c| c.id == e.category_id)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        let account_name = accounts
            .iter()
            .find(|a| a.id == e.account_id)
            .map(|a| a.name.clone())
            .unwrap_or_default();
        rows.push(CsvRow {
            date: e.spent_at,
            type_str: "expense",
            amount_cents: e.amount_cents,
            description: e.description,
            counterparty: String::new(),
            category: category_name,
            account: account_name,
            reference: String::new(),
        });
    }

    // ---- Sort by date ASC -----------------------------------------------
    rows.sort_by_key(|r| r.date);

    // ---- Serialize -------------------------------------------------------
    let mut out = String::with_capacity(16 * 1024);
    out.push_str("date,type,amount,description,counterparty,category,account,reference\n");
    for r in &rows {
        push_csv(&mut out, &r.date.format("%Y-%m-%d").to_string());
        out.push(',');
        push_csv(&mut out, r.type_str);
        out.push(',');
        push_csv(&mut out, &format!("{:.2}", r.amount_cents as f64 / 100.0));
        out.push(',');
        push_csv(&mut out, &r.description);
        out.push(',');
        push_csv(&mut out, &r.counterparty);
        out.push(',');
        push_csv(&mut out, &r.category);
        out.push(',');
        push_csv(&mut out, &r.account);
        out.push(',');
        push_csv(&mut out, &r.reference);
        out.push('\n');
    }

    let filename = format!("coterie-tax-prep-{}.csv", year);
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", filename),
            ),
        ],
        out,
    )
        .into_response()
}

// =============================================================================
// Helpers
// =============================================================================

fn month_range(year: i32, month: u32) -> Option<DateRange> {
    let start_naive = NaiveDate::from_ymd_opt(year, month, 1)?.and_hms_opt(0, 0, 0)?;
    let start = Utc.from_local_datetime(&start_naive).single()?;
    let (ny, nm) = next_month(year, month);
    let end_naive = NaiveDate::from_ymd_opt(ny, nm, 1)?.and_hms_opt(0, 0, 0)?;
    let end = Utc.from_local_datetime(&end_naive).single()?;
    Some(DateRange { start, end })
}

fn year_range(year: i32) -> Option<DateRange> {
    let start_naive = NaiveDate::from_ymd_opt(year, 1, 1)?.and_hms_opt(0, 0, 0)?;
    let start = Utc.from_local_datetime(&start_naive).single()?;
    let end_naive = NaiveDate::from_ymd_opt(year + 1, 1, 1)?.and_hms_opt(0, 0, 0)?;
    let end = Utc.from_local_datetime(&end_naive).single()?;
    Some(DateRange { start, end })
}

fn next_month(year: i32, month: u32) -> (i32, u32) {
    if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    }
}

fn previous_month(year: i32, month: u32) -> (i32, u32) {
    if month == 1 {
        (year - 1, 12)
    } else {
        (year, month - 1)
    }
}

fn month_name(month: u32) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "(invalid)",
    }
}

fn format_cents(cents: i64) -> String {
    let negative = cents < 0;
    let abs = cents.abs();
    let dollars = abs / 100;
    let pennies = abs % 100;
    let body = format!("${}.{:02}", dollars, pennies);
    if negative {
        format!("-{}", body)
    } else {
        body
    }
}

/// Sum of completed payment cents over `range`, on `paid_at`.
async fn income_in_range(pool: &SqlitePool, range: DateRange) -> Result<i64, sqlx::Error> {
    let total: Option<i64> = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount_cents), 0) \
         FROM payments \
         WHERE status = 'Completed' \
           AND paid_at IS NOT NULL \
           AND paid_at >= ? AND paid_at < ?",
    )
    .bind(range.start.naive_utc())
    .bind(range.end.naive_utc())
    .fetch_optional(pool)
    .await?;
    Ok(total.unwrap_or(0))
}

/// Income broken down by `payment_type` over `range`. Returns
/// `(payment_type_string, total_cents)` pairs.
async fn income_by_kind_in_range(
    pool: &SqlitePool,
    range: DateRange,
) -> Result<Vec<(String, i64)>, sqlx::Error> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT payment_type, COALESCE(SUM(amount_cents), 0) \
         FROM payments \
         WHERE status = 'Completed' \
           AND paid_at IS NOT NULL \
           AND paid_at >= ? AND paid_at < ? \
         GROUP BY payment_type",
    )
    .bind(range.start.naive_utc())
    .bind(range.end.naive_utc())
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// One-shot row shape for the joined `payments` + `members` query
/// that backs the tax-prep CSV. Tuple ordering matches the SELECT
/// list verbatim; the destructuring loop below names each slot.
type PaymentTaxRow = (
    Option<chrono::NaiveDateTime>, // paid_at
    chrono::NaiveDateTime,         // created_at
    String,                        // status
    String,                        // payment_type
    i64,                           // amount_cents
    String,                        // description
    Option<String>,                // stripe_payment_id
    Option<String>,                // donor_name
    Option<String>,                // donor_email
    Option<String>,                // member full_name
);

/// Build the payment / donation / refund rows for the tax-prep CSV.
///
/// - Completed donation rows → `type=donation`, positive amount.
/// - Completed non-donation rows → `type=payment`, positive amount.
/// - Refunded rows → `type=refund`, NEGATIVE amount.
async fn payment_rows_for_year(
    pool: &SqlitePool,
    range: DateRange,
) -> Result<Vec<CsvRow>, sqlx::Error> {
    // We pull a flat row tuple here rather than reuse `PaymentRow`
    // because the report needs the joined member name + donor_name,
    // and we don't need the structured `Payment` domain object.
    let recs: Vec<PaymentTaxRow> = sqlx::query_as(
        "SELECT p.paid_at, p.created_at, p.status, p.payment_type, p.amount_cents, \
                p.description, p.stripe_payment_id, p.donor_name, p.donor_email, m.full_name \
         FROM payments p \
         LEFT JOIN members m ON m.id = p.member_id \
         WHERE (p.status = 'Completed' OR p.status = 'Refunded') \
           AND COALESCE(p.paid_at, p.updated_at) >= ? \
           AND COALESCE(p.paid_at, p.updated_at) < ?",
    )
    .bind(range.start.naive_utc())
    .bind(range.end.naive_utc())
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(recs.len());
    for (
        paid_at,
        created_at,
        status,
        payment_type,
        amount,
        description,
        stripe_id,
        donor_name,
        donor_email,
        member_name,
    ) in recs
    {
        let date_naive = paid_at.unwrap_or(created_at);
        let date = DateTime::<Utc>::from_naive_utc_and_offset(date_naive, Utc);

        // Counterparty: prefer the joined member name; fall back to
        // the donor-name/email combo for public donations.
        let counterparty = match (member_name, donor_name, donor_email) {
            (Some(name), _, _) => name,
            (None, Some(name), Some(email)) => format!("{} <{}>", name, email),
            (None, Some(name), None) => name,
            (None, None, Some(email)) => email,
            (None, None, None) => String::new(),
        };

        let (type_str, signed_amount, category) = match status.as_str() {
            "Refunded" => ("refund", -amount, "refund".to_string()),
            _ => {
                // Completed
                if payment_type == "donation" {
                    ("donation", amount, "donation".to_string())
                } else {
                    ("payment", amount, payment_type.clone())
                }
            }
        };

        let account = if stripe_id.is_some() {
            "stripe".to_string()
        } else {
            "manual".to_string()
        };
        let reference = stripe_id.unwrap_or_default();

        out.push(CsvRow {
            date,
            type_str,
            amount_cents: signed_amount,
            description,
            counterparty,
            category,
            account,
            reference,
        });
    }
    Ok(out)
}
