//! Persistence for `series_enrollment` — who bought a pass to a class.
//!
//! Deliberately the same shape as the paid-seat half of
//! [`crate::repository::EventRepository`]: claim → link → confirm →
//! release / cancel, with the same held-seat predicate and the same
//! count-and-insert-in-one-statement race guard. An enrollment is a seat
//! at series scope, so it is the same state machine, not a second one.

use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    domain::{AttendanceStatus, Attendee, PaymentMethod, PaymentStatus, SeriesEnrollment},
    error::{AppError, Result},
    repository::RosterEntry,
};

/// Identity predicate for one enrollment row, NULL-safe on both sides —
/// the same trick `event_attendance` uses so one statement serves a
/// member's enrollment and a guest's. Binds are (member_id, guest_email).
const ENROLLEE_MATCH: &str = "member_id IS ? AND guest_email IS ?";

/// A held place in the class: a confirmed enrollment, or an in-flight one
/// whose payment is still `Pending` (or not yet linked, which is the
/// instant between claiming and creating the Checkout session). Same rule
/// as `HELD_SEAT_PREDICATE` on a single event, so an abandoned checkout
/// frees the place by virtue of its payment being flipped to `Failed`.
/// Expects the enrollment row aliased `e` and a LEFT JOIN of `payments`
/// aliased `p`.
const HELD_ENROLLMENT_PREDICATE: &str = "(e.status = 'Registered' \
     OR (e.status = 'PendingPayment' AND (e.payment_id IS NULL OR p.status = 'Pending')))";

#[async_trait]
pub trait SeriesEnrollmentRepository: Send + Sync {
    /// Places currently held — confirmed plus in-flight. What the class
    /// capacity is checked against.
    async fn count_held(&self, series_id: Uuid) -> Result<i64>;

    /// Atomically claim a place as `PendingPayment`, rejecting with
    /// `BadRequest` when the class is full. The count and the insert are
    /// ONE statement so two people can't claim the last place at once.
    /// `max_enrollments` of `None` means uncapped.
    async fn claim(
        &self,
        series_id: Uuid,
        enrollee: &Attendee,
        max_enrollments: Option<i32>,
    ) -> Result<()>;

    /// Point a claimed enrollment at the payment that will pay for it.
    async fn link_payment(
        &self,
        series_id: Uuid,
        enrollee: &Attendee,
        payment_id: Uuid,
    ) -> Result<()>;

    /// Enroll outright as `Registered`, capacity advisory — the free-class
    /// confirm step and the admin's at-the-door / comp path.
    async fn register(&self, series_id: Uuid, enrollee: &Attendee) -> Result<()>;

    /// Promote the enrollment linked to `payment_id` from `PendingPayment`
    /// to `Registered`. Conditional on it still being pending, so a late
    /// webhook can't resurrect a cancelled enrollment. Returns true when a
    /// row moved.
    async fn confirm_for_payment(&self, payment_id: Uuid) -> Result<bool>;

    /// Drop a `PendingPayment` claim entirely — the rollback for a
    /// Checkout session that couldn't be created, and the admin's
    /// release-a-stuck-enrollment control. Never touches a confirmed one.
    async fn release(&self, series_id: Uuid, enrollee: &Attendee) -> Result<()>;

    /// Cancel the enrollment linked to `payment_id` — the refund path.
    async fn cancel_for_payment(&self, payment_id: Uuid) -> Result<()>;

    /// The enrollment holding `payment_id`, if any. The refund and
    /// completion webhooks arrive with a payment id and nothing else.
    async fn find_by_payment(&self, payment_id: Uuid) -> Result<Option<SeriesEnrollment>>;

    /// This identity's enrollment in this series, whatever its status.
    async fn find(&self, series_id: Uuid, enrollee: &Attendee) -> Result<Option<SeriesEnrollment>>;

    /// Enrollments that should be seated on a newly materialized
    /// occurrence: `Registered` ones only. A `PendingPayment` enrollment
    /// gets its attendance when the webhook confirms it.
    async fn list_confirmed(&self, series_id: Uuid) -> Result<Vec<SeriesEnrollment>>;

    /// Every enrollee with their identity and payment state, for the
    /// admin class roster. Reuses [`RosterEntry`] — the columns an
    /// operator needs are the same ones a single event's roster shows.
    async fn roster(&self, series_id: Uuid) -> Result<Vec<RosterEntry>>;
}

pub struct SqliteSeriesEnrollmentRepository {
    pool: SqlitePool,
}

impl SqliteSeriesEnrollmentRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct EnrollmentRow {
    id: String,
    series_id: String,
    member_id: Option<String>,
    guest_name: Option<String>,
    guest_email: Option<String>,
    status: String,
    enrolled_at: NaiveDateTime,
    payment_id: Option<String>,
}

const ENROLLMENT_COLUMNS: &str =
    "id, series_id, member_id, guest_name, guest_email, status, enrolled_at, payment_id";

impl EnrollmentRow {
    fn into_domain(self) -> Result<SeriesEnrollment> {
        // The DB CHECK guarantees exactly one identity, so the member
        // branch is taken iff member_id is set.
        let enrollee = match self.member_id {
            Some(id) => Attendee::Member(
                Uuid::parse_str(&id).map_err(|e| AppError::Internal(e.to_string()))?,
            ),
            None => Attendee::Guest {
                name: self.guest_name.unwrap_or_default(),
                email: self.guest_email.unwrap_or_default(),
            },
        };
        Ok(SeriesEnrollment {
            id: Uuid::parse_str(&self.id).map_err(|e| AppError::Internal(e.to_string()))?,
            series_id: Uuid::parse_str(&self.series_id)
                .map_err(|e| AppError::Internal(e.to_string()))?,
            enrollee,
            status: parse_status(&self.status)?,
            enrolled_at: DateTime::from_naive_utc_and_offset(self.enrolled_at, Utc),
            payment_id: self
                .payment_id
                .as_deref()
                .and_then(|s| Uuid::parse_str(s).ok()),
        })
    }
}

fn parse_status(s: &str) -> Result<AttendanceStatus> {
    match s {
        "Registered" => Ok(AttendanceStatus::Registered),
        "Waitlisted" => Ok(AttendanceStatus::Waitlisted),
        "Cancelled" => Ok(AttendanceStatus::Cancelled),
        "PendingPayment" => Ok(AttendanceStatus::PendingPayment),
        _ => Err(AppError::Internal(format!(
            "Invalid enrollment status: {}",
            s
        ))),
    }
}

#[async_trait]
impl SeriesEnrollmentRepository for SqliteSeriesEnrollmentRepository {
    async fn count_held(&self, series_id: Uuid) -> Result<i64> {
        let sql = format!(
            "SELECT COUNT(*) FROM series_enrollment e \
             LEFT JOIN payments p ON p.id = e.payment_id \
             WHERE e.series_id = ? AND {HELD_ENROLLMENT_PREDICATE}",
        );
        let row: (i64,) = sqlx::query_as(&sql)
            .bind(series_id.to_string())
            .fetch_one(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(row.0)
    }

    async fn claim(
        &self,
        series_id: Uuid,
        enrollee: &Attendee,
        max_enrollments: Option<i32>,
    ) -> Result<()> {
        // One statement, so SQLite holds the write lock across the count
        // AND the insert: two people racing for the last place cannot
        // both see "one left". The `ON CONFLICT` arm re-claims a row the
        // enrollee previously cancelled or abandoned — neither of those
        // states holds a place, so the capacity guard still applies.
        let cap = max_enrollments.map(i64::from).unwrap_or(i64::MAX);
        let sql = format!(
            "INSERT INTO series_enrollment \
                 (id, series_id, member_id, guest_name, guest_email, status, enrolled_at, payment_id) \
             SELECT ?1, ?2, ?3, ?4, ?5, 'PendingPayment', CURRENT_TIMESTAMP, NULL \
             WHERE (SELECT COUNT(*) FROM series_enrollment e \
                    LEFT JOIN payments p ON p.id = e.payment_id \
                    WHERE e.series_id = ?2 AND {HELD_ENROLLMENT_PREDICATE}) < ?6 \
             ON CONFLICT DO UPDATE \
             SET status = 'PendingPayment', enrolled_at = CURRENT_TIMESTAMP, payment_id = NULL",
        );
        let res = sqlx::query(&sql)
            .bind(Uuid::new_v4().to_string())
            .bind(series_id.to_string())
            .bind(enrollee.member_id().map(|id| id.to_string()))
            .bind(enrollee.guest_name())
            .bind(enrollee.guest_email())
            .bind(cap)
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;

        if res.rows_affected() == 0 {
            return Err(AppError::BadRequest(
                "This class is full — no places are available".to_string(),
            ));
        }
        Ok(())
    }

    async fn link_payment(
        &self,
        series_id: Uuid,
        enrollee: &Attendee,
        payment_id: Uuid,
    ) -> Result<()> {
        let sql = format!(
            "UPDATE series_enrollment SET payment_id = ? \
             WHERE series_id = ? AND {ENROLLEE_MATCH}",
        );
        sqlx::query(&sql)
            .bind(payment_id.to_string())
            .bind(series_id.to_string())
            .bind(enrollee.member_id().map(|id| id.to_string()))
            .bind(enrollee.guest_email())
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(())
    }

    async fn register(&self, series_id: Uuid, enrollee: &Attendee) -> Result<()> {
        // Conflict target omitted so the one statement upserts on
        // whichever identity constraint the row collides with.
        sqlx::query(
            "INSERT INTO series_enrollment \
                 (id, series_id, member_id, guest_name, guest_email, status, enrolled_at) \
             VALUES (?, ?, ?, ?, ?, 'Registered', CURRENT_TIMESTAMP) \
             ON CONFLICT DO UPDATE \
             SET status = 'Registered', enrolled_at = CURRENT_TIMESTAMP",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(series_id.to_string())
        .bind(enrollee.member_id().map(|id| id.to_string()))
        .bind(enrollee.guest_name())
        .bind(enrollee.guest_email())
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }

    async fn confirm_for_payment(&self, payment_id: Uuid) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE series_enrollment SET status = 'Registered' \
             WHERE payment_id = ? AND status = 'PendingPayment'",
        )
        .bind(payment_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(res.rows_affected() > 0)
    }

    async fn release(&self, series_id: Uuid, enrollee: &Attendee) -> Result<()> {
        let sql = format!(
            "DELETE FROM series_enrollment \
             WHERE series_id = ? AND {ENROLLEE_MATCH} AND status = 'PendingPayment'",
        );
        sqlx::query(&sql)
            .bind(series_id.to_string())
            .bind(enrollee.member_id().map(|id| id.to_string()))
            .bind(enrollee.guest_email())
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(())
    }

    async fn cancel_for_payment(&self, payment_id: Uuid) -> Result<()> {
        sqlx::query("UPDATE series_enrollment SET status = 'Cancelled' WHERE payment_id = ?")
            .bind(payment_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(())
    }

    async fn find_by_payment(&self, payment_id: Uuid) -> Result<Option<SeriesEnrollment>> {
        let row = sqlx::query_as::<_, EnrollmentRow>(&format!(
            "SELECT {ENROLLMENT_COLUMNS} FROM series_enrollment WHERE payment_id = ?",
        ))
        .bind(payment_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;
        row.map(EnrollmentRow::into_domain).transpose()
    }

    async fn find(&self, series_id: Uuid, enrollee: &Attendee) -> Result<Option<SeriesEnrollment>> {
        let row = sqlx::query_as::<_, EnrollmentRow>(&format!(
            "SELECT {ENROLLMENT_COLUMNS} FROM series_enrollment \
             WHERE series_id = ? AND {ENROLLEE_MATCH}",
        ))
        .bind(series_id.to_string())
        .bind(enrollee.member_id().map(|id| id.to_string()))
        .bind(enrollee.guest_email())
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;
        row.map(EnrollmentRow::into_domain).transpose()
    }

    async fn list_confirmed(&self, series_id: Uuid) -> Result<Vec<SeriesEnrollment>> {
        let rows = sqlx::query_as::<_, EnrollmentRow>(&format!(
            "SELECT {ENROLLMENT_COLUMNS} FROM series_enrollment \
             WHERE series_id = ? AND status = 'Registered' \
             ORDER BY enrolled_at ASC",
        ))
        .bind(series_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;
        rows.into_iter().map(EnrollmentRow::into_domain).collect()
    }

    async fn roster(&self, series_id: Uuid) -> Result<Vec<RosterEntry>> {
        // LEFT JOIN, not JOIN: a guest enrollment has no member row to
        // join to, and an inner join would hide every guest.
        #[allow(clippy::type_complexity)]
        let rows: Vec<(
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<i64>,
        )> = sqlx::query_as(
            r#"
            SELECT e.member_id, m.full_name, m.email, e.guest_name, e.guest_email, e.status,
                   e.payment_id, p.status, p.payment_method, p.amount_cents
            FROM series_enrollment e
            LEFT JOIN members m ON m.id = e.member_id
            LEFT JOIN payments p ON p.id = e.payment_id
            WHERE e.series_id = ?
            ORDER BY e.enrolled_at ASC
            "#,
        )
        .bind(series_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        rows.into_iter()
            .map(
                |(
                    mid,
                    m_name,
                    m_email,
                    guest_name,
                    guest_email,
                    status,
                    pay_id,
                    pay_status,
                    pay_method,
                    amount,
                )| {
                    let (attendee, name, email) = match mid {
                        Some(mid) => (
                            Attendee::Member(
                                Uuid::parse_str(&mid)
                                    .map_err(|e| AppError::Internal(e.to_string()))?,
                            ),
                            m_name.unwrap_or_default(),
                            m_email.unwrap_or_default(),
                        ),
                        None => {
                            let email = guest_email.unwrap_or_default();
                            let name = guest_name.unwrap_or_default();
                            (
                                Attendee::Guest {
                                    name: name.clone(),
                                    email: email.clone(),
                                },
                                name,
                                email,
                            )
                        }
                    };
                    Ok(RosterEntry {
                        attendee,
                        name,
                        email,
                        status: parse_status(&status)?,
                        payment_id: pay_id.as_deref().and_then(|s| Uuid::parse_str(s).ok()),
                        payment_status: pay_status.as_deref().and_then(parse_payment_status),
                        payment_method: pay_method.as_deref().and_then(parse_payment_method),
                        amount_cents: amount,
                    })
                },
            )
            .collect()
    }
}

/// Joined payment columns are read leniently: an unrecognized value on
/// the roster costs a display detail, not the whole page. Mirrors the
/// same pair in `event_repository`.
fn parse_payment_status(s: &str) -> Option<PaymentStatus> {
    match s {
        "Pending" => Some(PaymentStatus::Pending),
        "Completed" => Some(PaymentStatus::Completed),
        "Failed" => Some(PaymentStatus::Failed),
        "Refunded" => Some(PaymentStatus::Refunded),
        _ => None,
    }
}

fn parse_payment_method(s: &str) -> Option<PaymentMethod> {
    match s {
        "Stripe" => Some(PaymentMethod::Stripe),
        "Manual" => Some(PaymentMethod::Manual),
        "Waived" => Some(PaymentMethod::Waived),
        _ => None,
    }
}
