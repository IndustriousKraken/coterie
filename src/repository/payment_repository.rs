use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use crate::{
    domain::{
        configurable_types::BillingPeriod, MemberStatus, Payer, Payment, PaymentKind,
        PaymentMethod, PaymentStatus, StripeRef,
    },
    error::{AppError, Result},
};

/// Single (month, payment_type) bucket for the admin billing dashboard.
/// `payment_type` is the raw lowercase DB-column value
/// (`"membership" | "donation" | "other"`) — this is a SQL aggregation
/// row, not a real `Payment`, so we don't try to lift it into the
/// richer `PaymentKind` (Donation needs a campaign id we don't carry
/// at the bucket level). Callers match on the string.
#[derive(Debug, Clone)]
pub struct MonthlyRevenue {
    pub year: i32,
    pub month: u32,
    pub payment_type: String,
    pub total_cents: i64,
    pub payment_count: i64,
}

#[async_trait]
pub trait PaymentRepository: Send + Sync {
    async fn create(&self, payment: Payment) -> Result<Payment>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Payment>>;
    async fn find_by_member(&self, member_id: Uuid) -> Result<Vec<Payment>>;
    async fn find_by_stripe_id(&self, stripe_id: &str) -> Result<Option<Payment>>;
    async fn update(&self, id: Uuid, payment: Payment) -> Result<Payment>;
    /// Atomically flip a Pending payment to Completed and stamp the
    /// Stripe PaymentIntent ID. Returns `true` if the row was actually
    /// flipped (we own the post-payment work — extend dues, schedule
    /// next renewal); `false` if the row had already been completed by
    /// another caller (sync path vs. webhook race). The semantics
    /// guarantee that exactly one caller does the post-work.
    async fn complete_pending_payment(&self, id: Uuid, stripe_payment_id: &str) -> Result<bool>;
    /// Counterpart to `complete_pending_payment` for the failure path:
    /// flip a Pending row to Failed when the Stripe charge errored.
    /// Returns true if a row was flipped. Idempotent against double-
    /// failure reports.
    async fn fail_pending_payment(&self, id: Uuid) -> Result<bool>;
    /// Claim a Completed payment for refund. Atomic conditional UPDATE
    /// (`WHERE status='Completed'`) — only the first caller observes
    /// rows_affected==1; concurrent admin clicks see false and bail.
    /// Pair with `unclaim_refund` if the subsequent Stripe call fails.
    async fn claim_payment_for_refund(&self, id: Uuid) -> Result<bool>;
    /// Roll back `claim_payment_for_refund` after a Stripe failure so
    /// the row goes back to Completed and a future refund attempt can
    /// re-claim. Conditional on status='Refunded' so this can't undo
    /// a legitimate completed refund from a different code path.
    async fn unclaim_refund(&self, id: Uuid) -> Result<()>;
    /// Mark a payment Refunded, unconditionally. Used by the Stripe
    /// `charge.refunded` webhook handler when the row hasn't already
    /// been flipped by our own admin-button refund (caller filters
    /// out `Refunded` echoes). Idempotent under repeat calls.
    async fn mark_refunded(&self, id: Uuid) -> Result<()>;
    /// Idempotently extend a member's dues for a single Payment.
    ///
    /// Implemented as a transactional claim-then-update: the row's
    /// `dues_extended_at` is set to NOW under a per-payment uniqueness
    /// guard, and `dues_paid_until` is recomputed from the latest
    /// member state (read inside the same transaction so concurrent
    /// payments can't lose each other's increments). Returns
    /// `Some(prior_status)` — the member's status BEFORE the update —
    /// if THIS call did the extension (callers use it to detect the
    /// Pending→Active activation, see the pay-at-signup spec); `None`
    /// if a previous call already extended dues for this payment (or
    /// the member row is gone).
    ///
    /// This single method addresses two correctness issues:
    /// (1) Stripe webhook retries that re-run a handler after a
    ///     transient failure no longer double-extend dues (the second
    ///     call sees the claim and no-ops).
    /// (2) Two payments for the same member processed concurrently
    ///     can't both compute `D + 1y` from the same starting `D` —
    ///     the SQLite write lock serializes the SELECT/UPDATE pair.
    async fn extend_dues_for_payment_atomic(
        &self,
        payment_id: Uuid,
        member_id: Uuid,
        billing_period: crate::domain::configurable_types::BillingPeriod,
    ) -> Result<Option<MemberStatus>>;

    /// Give back the dues extension a single payment granted — the
    /// inverse of `extend_dues_for_payment_atomic`, for when that
    /// payment is refunded.
    ///
    /// Subtracts the extension THIS payment applied
    /// (`dues_extension_seconds`, recorded by the extension) from the
    /// member's current `dues_paid_until`. It is a subtraction, not a
    /// reset: a member holding dues granted by several payments keeps
    /// the others' contributions.
    ///
    /// Idempotent by the same claim-then-update shape as the extension,
    /// anchored on `dues_retracted_at` — the admin refund path and its
    /// own `charge.refunded` echo can both reach it, and the second
    /// caller must not reduce the window again.
    ///
    /// Returns `Some((member_id, seconds))` if THIS call performed the
    /// retraction (callers audit off that); `None` when there is
    /// nothing to retract — the payment granted no dues extension
    /// (any non-membership payment, or a row predating the delta
    /// column), it was already retracted, or its member row is gone.
    /// None of those is an error.
    ///
    /// Deliberately does NOT write member status: where the retracted
    /// window leaves a member no longer paid up, the daily expiration
    /// sweep transitions them, so there stays one path to `Expired`.
    async fn retract_dues_for_payment(&self, payment_id: Uuid) -> Result<Option<(Uuid, i64)>>;

    // ---- Paid-event support -------------------------------------------

    /// The payer's live event-fee payment for this event, if any —
    /// the double-charge guard. `Failed` rows are excluded: they hold
    /// neither money nor a seat, so they must not block a retry. The
    /// result is ordered `Completed` first, then `Pending`, then
    /// anything else (a `Refunded` row, whose seat was cancelled and
    /// which therefore does not stop the payer registering again).
    ///
    /// A member payer matches on `member_id`, a guest on `donor_email` —
    /// the same identity split the `payments` table has carried since
    /// public donations (migration 016), so the guard is keyed on
    /// `(event_id, guest_email)` for a guest without a second query.
    async fn find_event_fee_payment(
        &self,
        event_id: Uuid,
        payer: &crate::domain::Payer,
    ) -> Result<Option<Payment>>;

    /// Every `Completed` event-fee payment for this event. Drives the
    /// refund-before-delete sweep, which must not leave a charged
    /// attendee behind when the event (and its cascading roster) goes.
    async fn list_completed_event_fees(&self, event_id: Uuid) -> Result<Vec<Payment>>;

    // ---- Series-pass support ------------------------------------------
    //
    // The two below are the series-scope siblings of the two above, with
    // the same ordering and same `Failed`-excluded semantics — a pass is
    // a seat bought at series scope, so the guards are the same guards.

    /// The payer's live series-pass payment for this series, if any —
    /// the double-charge guard for enrollment.
    async fn find_series_pass_payment(
        &self,
        series_id: Uuid,
        payer: &crate::domain::Payer,
    ) -> Result<Option<Payment>>;

    /// Every `Completed` series-pass payment for this series. Drives the
    /// refund-before-delete sweep on a class.
    async fn list_completed_series_passes(&self, series_id: Uuid) -> Result<Vec<Payment>>;

    // ---- Admin billing dashboard support ------------------------------

    /// Sum of completed-payment cents grouped by (year, month,
    /// payment_type) across the last `months_back` months of `paid_at`.
    /// Refunded / Pending / Failed rows are excluded — they'd mislead
    /// "what we actually collected." Ordered newest month first.
    async fn revenue_by_month(&self, months_back: u32) -> Result<Vec<MonthlyRevenue>>;
}

#[derive(FromRow)]
struct PaymentRow {
    id: String,
    member_id: Option<String>,
    /// SQLite INTEGER is up to 8 bytes; using i64 here matches both
    /// the schema's actual storage and the domain's `Payment.amount_cents`.
    /// The previous i32 silently truncated values >$21.5M cents.
    amount_cents: i64,
    currency: String,
    status: String,
    payment_method: String,
    stripe_payment_id: Option<String>,
    description: String,
    payment_type: String,
    donation_campaign_id: Option<String>,
    event_id: Option<String>,
    series_id: Option<String>,
    donor_name: Option<String>,
    donor_email: Option<String>,
    paid_at: Option<NaiveDateTime>,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
}

pub struct SqlitePaymentRepository {
    pool: SqlitePool,
}

impl SqlitePaymentRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn row_to_payment(row: PaymentRow) -> Result<Payment> {
        // The DB CHECK constraint guarantees `member_id IS NOT NULL OR
        // (donor_name AND donor_email)`, so exactly one of the two
        // identity paths is populated. Construct the right `Payer`
        // variant; fail-fast if a row somehow violates the invariant
        // (only possible if the constraint was bypassed by a manual
        // migration). We don't soft-fall-back here — letting a payment
        // through with a fabricated payer would be worse than a 500.
        let member_id = row
            .member_id
            .as_deref()
            .map(Uuid::parse_str)
            .transpose()
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let payer = match (member_id, row.donor_name, row.donor_email) {
            (Some(id), _, _) => Payer::Member(id),
            (None, Some(name), Some(email)) => Payer::PublicDonor { name, email },
            _ => {
                return Err(AppError::Internal(format!(
                    "Payment {} has neither member_id nor (donor_name, donor_email) — row violates schema CHECK",
                    row.id,
                )));
            }
        };

        // Tolerate unknown payment_type values from older rows by
        // falling back to Membership (the column default).
        let donation_campaign_id = row
            .donation_campaign_id
            .as_deref()
            .and_then(|s| Uuid::parse_str(s).ok());
        let event_id = row
            .event_id
            .as_deref()
            .and_then(|s| Uuid::parse_str(s).ok());
        let series_id = row
            .series_id
            .as_deref()
            .and_then(|s| Uuid::parse_str(s).ok());
        let kind = match row.payment_type.as_str() {
            "membership" => PaymentKind::Membership,
            "donation" => PaymentKind::Donation {
                campaign_id: donation_campaign_id,
            },
            // An `event_fee` row without a parseable event_id can't name
            // its seat, so it degrades to the untyped bucket rather than
            // fabricating a uuid. Only reachable if the column was
            // cleared out from under us by hand.
            "event_fee" => match event_id {
                Some(event_id) => PaymentKind::EventFee { event_id },
                None => PaymentKind::Other,
            },
            // Same degradation rule as `event_fee`: a pass row that can't
            // name its series can't reach its enrollment, so it falls back
            // to the untyped bucket rather than fabricating a uuid.
            "series_pass" => match series_id {
                Some(series_id) => PaymentKind::SeriesPass { series_id },
                None => PaymentKind::Other,
            },
            "other" => PaymentKind::Other,
            _ => PaymentKind::Membership,
        };

        // Stripe id: parse the prefix into a typed variant. Unknown
        // prefixes (or shapes we no longer recognize) are dropped to
        // `None` rather than panicking — they'll just lose Stripe-
        // side functionality (refund-via-API) until reconciled.
        let external_id = row
            .stripe_payment_id
            .as_deref()
            .and_then(StripeRef::from_id);

        Ok(Payment {
            id: Uuid::parse_str(&row.id).map_err(|e| AppError::Internal(e.to_string()))?,
            payer,
            amount_cents: row.amount_cents,
            currency: row.currency,
            status: Self::parse_payment_status(&row.status)?,
            payment_method: Self::parse_payment_method(&row.payment_method)?,
            kind,
            external_id,
            description: row.description,
            paid_at: row
                .paid_at
                .map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc)),
            created_at: DateTime::from_naive_utc_and_offset(row.created_at, Utc),
            updated_at: DateTime::from_naive_utc_and_offset(row.updated_at, Utc),
        })
    }

    fn parse_payment_status(s: &str) -> Result<PaymentStatus> {
        match s {
            "Pending" => Ok(PaymentStatus::Pending),
            "Completed" => Ok(PaymentStatus::Completed),
            "Failed" => Ok(PaymentStatus::Failed),
            "Refunded" => Ok(PaymentStatus::Refunded),
            _ => Err(AppError::Internal(format!("Invalid payment status: {}", s))),
        }
    }

    fn payment_status_to_str(status: &PaymentStatus) -> &'static str {
        match status {
            PaymentStatus::Pending => "Pending",
            PaymentStatus::Completed => "Completed",
            PaymentStatus::Failed => "Failed",
            PaymentStatus::Refunded => "Refunded",
        }
    }

    fn parse_payment_method(s: &str) -> Result<PaymentMethod> {
        match s {
            "Stripe" => Ok(PaymentMethod::Stripe),
            "Manual" => Ok(PaymentMethod::Manual),
            "Waived" => Ok(PaymentMethod::Waived),
            _ => Err(AppError::Internal(format!("Invalid payment method: {}", s))),
        }
    }

    fn payment_method_to_str(method: &PaymentMethod) -> &'static str {
        match method {
            PaymentMethod::Stripe => "Stripe",
            PaymentMethod::Manual => "Manual",
            PaymentMethod::Waived => "Waived",
        }
    }

    /// The payer's live payment for one paid registration target — an
    /// event's fee or a class's pass. The double-charge guard.
    ///
    /// `Failed` rows are excluded: they hold neither money nor a seat, so
    /// they must not block a retry. Ordered `Completed` first, then
    /// `Pending`, then anything else (a `Refunded` row, whose seat was
    /// cancelled and which therefore does not stop a fresh purchase).
    ///
    /// `IS` rather than `=` on both identity columns so a bound NULL
    /// matches the NULL column: one statement answers "this member's" and
    /// "this guest email's". A member payer matches on `member_id`, a
    /// guest on `donor_email` — the same identity split `payments` has
    /// carried since public donations (migration 016).
    async fn find_paid_registration(
        &self,
        target: PaidTarget,
        target_id: Uuid,
        payer: &crate::domain::Payer,
    ) -> Result<Option<Payment>> {
        let row = sqlx::query_as::<_, PaymentRow>(&format!(
            "SELECT {PAYMENT_COLUMNS} FROM payments \
             WHERE {} = ? \
               AND member_id IS ? \
               AND donor_email IS ? \
               AND payment_type = '{}' \
               AND status <> 'Failed' \
             ORDER BY CASE status \
                          WHEN 'Completed' THEN 0 \
                          WHEN 'Pending' THEN 1 \
                          ELSE 2 \
                      END, \
                      created_at DESC \
             LIMIT 1",
            target.id_column, target.payment_type,
        ))
        .bind(target_id.to_string())
        .bind(payer.member_id().map(|id| id.to_string()))
        .bind(match payer {
            crate::domain::Payer::Member(_) => None,
            crate::domain::Payer::PublicDonor { email, .. } => Some(email.as_str()),
        })
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        match row {
            Some(r) => Ok(Some(Self::row_to_payment(r)?)),
            None => Ok(None),
        }
    }

    /// Every `Completed` payment against one paid registration target.
    /// Drives the refund-before-delete sweeps, which must not leave a
    /// charged attendee behind when the event or class (and its cascading
    /// roster) goes.
    async fn list_completed_registrations(
        &self,
        target: PaidTarget,
        target_id: Uuid,
    ) -> Result<Vec<Payment>> {
        let rows = sqlx::query_as::<_, PaymentRow>(&format!(
            "SELECT {PAYMENT_COLUMNS} FROM payments \
             WHERE {} = ? AND payment_type = '{}' AND status = 'Completed' \
             ORDER BY created_at ASC",
            target.id_column, target.payment_type,
        ))
        .bind(target_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        rows.into_iter().map(Self::row_to_payment).collect()
    }
}

/// Full `payments` projection, in one place so the queries below can't
/// disagree about it when a column is added.
const PAYMENT_COLUMNS: &str = "id, member_id, amount_cents, currency, status, \
     payment_method, stripe_payment_id, description, \
     payment_type, donation_campaign_id, event_id, series_id, \
     donor_name, donor_email, paid_at, created_at, updated_at";

/// What a paid registration is against: one event's fee, or one class's
/// pass. The two differ only in which id column and which `payment_type`
/// literal they key on, so the guards share their SQL rather than being
/// copies that drift.
///
/// Both fields are compile-time constants below — never caller input —
/// so interpolating them into the statement is not a parameter position.
#[derive(Clone, Copy)]
struct PaidTarget {
    id_column: &'static str,
    payment_type: &'static str,
}

const PAID_EVENT: PaidTarget = PaidTarget {
    id_column: "event_id",
    payment_type: "event_fee",
};

const PAID_CLASS: PaidTarget = PaidTarget {
    id_column: "series_id",
    payment_type: "series_pass",
};

#[async_trait]
impl PaymentRepository for SqlitePaymentRepository {
    async fn create(&self, payment: Payment) -> Result<Payment> {
        let id_str = payment.id.to_string();
        let amount_cents_int = payment.amount_cents;
        let status_str = Self::payment_status_to_str(&payment.status);
        let method_str = Self::payment_method_to_str(&payment.payment_method);
        let paid_at_naive = payment.paid_at.map(|dt| dt.naive_utc());
        let now = Utc::now().naive_utc();

        // Decompose the typed Payer / PaymentKind / StripeRef back
        // into the wide DB columns. The schema is unchanged — only
        // the in-memory shape moved to sum types.
        let (member_id_str, donor_name, donor_email) = match &payment.payer {
            Payer::Member(id) => (Some(id.to_string()), None, None),
            Payer::PublicDonor { name, email } => (None, Some(name.clone()), Some(email.clone())),
        };
        let payment_type_str = payment.kind.as_str();
        let donation_campaign_id_str = payment.kind.campaign_id().map(|u| u.to_string());
        let event_id_str = payment.kind.event_id().map(|u| u.to_string());
        let series_id_str = payment.kind.series_id().map(|u| u.to_string());
        let stripe_id_str = payment.external_id.as_ref().map(|r| r.as_str().to_string());

        sqlx::query(
            r#"
            INSERT INTO payments (
                id, member_id, amount_cents, currency, status,
                payment_method, stripe_payment_id, description,
                payment_type, donation_campaign_id, event_id, series_id,
                donor_name, donor_email,
                paid_at, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id_str)
        .bind(&member_id_str)
        .bind(amount_cents_int)
        .bind(&payment.currency)
        .bind(status_str)
        .bind(method_str)
        .bind(&stripe_id_str)
        .bind(&payment.description)
        .bind(payment_type_str)
        .bind(&donation_campaign_id_str)
        .bind(&event_id_str)
        .bind(&series_id_str)
        .bind(&donor_name)
        .bind(&donor_email)
        .bind(paid_at_naive)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

        self.find_by_id(payment.id)
            .await?
            .ok_or_else(|| AppError::Internal("Failed to retrieve created payment".to_string()))
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<Payment>> {
        let id_str = id.to_string();
        let row = sqlx::query_as::<_, PaymentRow>(
            r#"
            SELECT id, member_id, amount_cents, currency, status,
                   payment_method, stripe_payment_id, description,
                   payment_type, donation_campaign_id, event_id, series_id,
                   donor_name, donor_email,
                   paid_at, created_at, updated_at
            FROM payments
            WHERE id = ?
            "#,
        )
        .bind(id_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        match row {
            Some(r) => Ok(Some(Self::row_to_payment(r)?)),
            None => Ok(None),
        }
    }

    async fn find_by_member(&self, member_id: Uuid) -> Result<Vec<Payment>> {
        let member_id_str = member_id.to_string();
        let rows = sqlx::query_as::<_, PaymentRow>(
            r#"
            SELECT id, member_id, amount_cents, currency, status,
                   payment_method, stripe_payment_id, description,
                   payment_type, donation_campaign_id, event_id, series_id,
                   donor_name, donor_email,
                   paid_at, created_at, updated_at
            FROM payments
            WHERE member_id = ?
            ORDER BY COALESCE(paid_at, created_at) DESC
            "#,
        )
        .bind(member_id_str)
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        rows.into_iter().map(Self::row_to_payment).collect()
    }

    async fn find_by_stripe_id(&self, stripe_id: &str) -> Result<Option<Payment>> {
        let row = sqlx::query_as::<_, PaymentRow>(
            r#"
            SELECT id, member_id, amount_cents, currency, status,
                   payment_method, stripe_payment_id, description,
                   payment_type, donation_campaign_id, event_id, series_id,
                   donor_name, donor_email,
                   paid_at, created_at, updated_at
            FROM payments
            WHERE stripe_payment_id = ?
            "#,
        )
        .bind(stripe_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        match row {
            Some(r) => Ok(Some(Self::row_to_payment(r)?)),
            None => Ok(None),
        }
    }

    async fn update(&self, id: Uuid, payment: Payment) -> Result<Payment> {
        let id_str = id.to_string();
        let now = Utc::now().naive_utc();
        let status_str = Self::payment_status_to_str(&payment.status);
        let method_str = Self::payment_method_to_str(&payment.payment_method);
        let paid_at_naive = payment.paid_at.map(|dt| dt.naive_utc());

        sqlx::query(
            r#"
            UPDATE payments
            SET member_id = ?,
                amount_cents = ?,
                currency = ?,
                status = ?,
                payment_method = ?,
                stripe_payment_id = ?,
                description = ?,
                paid_at = ?,
                updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(payment.member_id().map(|id| id.to_string()))
        .bind(payment.amount_cents)
        .bind(&payment.currency)
        .bind(status_str)
        .bind(method_str)
        .bind(payment.external_id.as_ref().map(|r| r.as_str().to_string()))
        .bind(&payment.description)
        .bind(paid_at_naive)
        .bind(now)
        .bind(&id_str)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

        self.find_by_id(id)
            .await?
            .ok_or_else(|| AppError::Internal("Failed to retrieve updated payment".to_string()))
    }

    async fn complete_pending_payment(&self, id: Uuid, stripe_payment_id: &str) -> Result<bool> {
        let now = Utc::now().naive_utc();
        let res = sqlx::query(
            "UPDATE payments \
             SET status = 'Completed', \
                 stripe_payment_id = ?, \
                 paid_at = ?, \
                 updated_at = ? \
             WHERE id = ? AND status = 'Pending'",
        )
        .bind(stripe_payment_id)
        .bind(now)
        .bind(now)
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(res.rows_affected() == 1)
    }

    async fn fail_pending_payment(&self, id: Uuid) -> Result<bool> {
        let now = Utc::now().naive_utc();
        let res = sqlx::query(
            "UPDATE payments \
             SET status = 'Failed', updated_at = ? \
             WHERE id = ? AND status = 'Pending'",
        )
        .bind(now)
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(res.rows_affected() == 1)
    }

    async fn claim_payment_for_refund(&self, id: Uuid) -> Result<bool> {
        let now = Utc::now().naive_utc();
        let res = sqlx::query(
            "UPDATE payments \
             SET status = 'Refunded', updated_at = ? \
             WHERE id = ? AND status = 'Completed'",
        )
        .bind(now)
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(res.rows_affected() == 1)
    }

    async fn unclaim_refund(&self, id: Uuid) -> Result<()> {
        let now = Utc::now().naive_utc();
        sqlx::query(
            "UPDATE payments \
             SET status = 'Completed', updated_at = ? \
             WHERE id = ? AND status = 'Refunded'",
        )
        .bind(now)
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }

    async fn mark_refunded(&self, id: Uuid) -> Result<()> {
        sqlx::query("UPDATE payments SET status = 'Refunded', updated_at = ? WHERE id = ?")
            .bind(Utc::now().naive_utc())
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(())
    }

    async fn extend_dues_for_payment_atomic(
        &self,
        payment_id: Uuid,
        member_id: Uuid,
        billing_period: BillingPeriod,
    ) -> Result<Option<MemberStatus>> {
        use chrono::Months;

        let mut tx = self.pool.begin().await.map_err(AppError::Database)?;

        // Atomic claim. dues_extended_at is the per-payment idempotency
        // anchor: only the first caller for this payment_id sees
        // rows_affected == 1; any later caller (including a webhook
        // retry after rollback) sees 0 and no-ops below.
        let now_naive = Utc::now().naive_utc();
        let claim = sqlx::query(
            "UPDATE payments SET dues_extended_at = ? \
             WHERE id = ? AND dues_extended_at IS NULL",
        )
        .bind(now_naive)
        .bind(payment_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;

        if claim.rows_affected() == 0 {
            tx.commit().await.map_err(AppError::Database)?;
            return Ok(None);
        }

        // Read current dues + status INSIDE the transaction so SQLite's
        // write lock serializes us against any concurrent payment for
        // the same member. Without the txn, two payments could both
        // read D and both write D+1y, losing one period. The status
        // read is what lets the caller observe the Pending→Active
        // activation this payment performs.
        let row: Option<(Option<DateTime<Utc>>, String)> =
            sqlx::query_as("SELECT dues_paid_until, status FROM members WHERE id = ?")
                .bind(member_id.to_string())
                .fetch_optional(&mut *tx)
                .await
                .map_err(AppError::Database)?;

        let Some((current_dues, prior_status_str)) = row else {
            // Member row gone (payment for a deleted member): the claim
            // stands so retries stay no-ops, but there is nothing to
            // extend or activate.
            tx.commit().await.map_err(AppError::Database)?;
            return Ok(None);
        };

        let now_utc = Utc::now();
        let base_date = current_dues.filter(|d| *d > now_utc).unwrap_or(now_utc);
        let new_dues_date = match billing_period {
            BillingPeriod::Monthly => base_date
                .checked_add_months(Months::new(1))
                .unwrap_or(base_date),
            BillingPeriod::SemiAnnual => base_date
                .checked_add_months(Months::new(6))
                .unwrap_or(base_date),
            BillingPeriod::Yearly => base_date
                .checked_add_months(Months::new(12))
                .unwrap_or(base_date),
            BillingPeriod::Lifetime => DateTime::<Utc>::MAX_UTC,
        };

        // Record how much this payment moved the window, so a refund of
        // it can give back exactly that and no more (see
        // `retract_dues_for_payment`). The delta, not the before/after
        // pair: a member may hold dues from several payments and only
        // the refunded one is ever undone.
        sqlx::query("UPDATE payments SET dues_extension_seconds = ? WHERE id = ?")
            .bind((new_dues_date - base_date).num_seconds())
            .bind(payment_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(AppError::Database)?;

        // Revival: a completed dues payment makes Expired members
        // Active (restoration) and Pending members Active (pay-at-
        // signup activation — payment IS the approval).
        sqlx::query(
            "UPDATE members \
             SET dues_paid_until = ?, \
                 status = CASE WHEN status IN ('Expired', 'Pending') THEN 'Active' ELSE status END, \
                 dues_reminder_sent_at = NULL, \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?",
        )
        .bind(new_dues_date)
        .bind(member_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;

        tx.commit().await.map_err(AppError::Database)?;
        // Unknown status strings (corrupt row) map to None-ish behavior:
        // report Active so callers don't fire a spurious activation.
        Ok(Some(
            MemberStatus::from_str(&prior_status_str).unwrap_or(MemberStatus::Active),
        ))
    }

    async fn retract_dues_for_payment(&self, payment_id: Uuid) -> Result<Option<(Uuid, i64)>> {
        let mut tx = self.pool.begin().await.map_err(AppError::Database)?;

        // Atomic claim, mirroring the extension's. Only the first caller
        // for this payment stamps dues_retracted_at; the admin refund's
        // own webhook echo (or a webhook retry) sees 0 rows and no-ops.
        // The `dues_extension_seconds IS NOT NULL` half is what makes
        // "this payment granted no dues" a silent no-op rather than an
        // error — every non-membership refund reaches here too.
        let claim = sqlx::query(
            "UPDATE payments SET dues_retracted_at = ? \
             WHERE id = ? AND dues_extension_seconds IS NOT NULL \
               AND dues_retracted_at IS NULL",
        )
        .bind(Utc::now().naive_utc())
        .bind(payment_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;

        if claim.rows_affected() == 0 {
            tx.commit().await.map_err(AppError::Database)?;
            return Ok(None);
        }

        let row: Option<(Option<String>, i64)> =
            sqlx::query_as("SELECT member_id, dues_extension_seconds FROM payments WHERE id = ?")
                .bind(payment_id.to_string())
                .fetch_optional(&mut *tx)
                .await
                .map_err(AppError::Database)?;

        // A dues-extending payment with no resolvable member is a corrupt
        // row; the claim stands so retries stay no-ops.
        let member_id = row
            .as_ref()
            .and_then(|(m, _)| m.as_deref())
            .and_then(|m| Uuid::parse_str(m).ok());
        let (Some((_, seconds)), Some(member_id)) = (&row, member_id) else {
            tx.commit().await.map_err(AppError::Database)?;
            return Ok(None);
        };
        let seconds = *seconds;

        // Read-then-write inside the transaction for the same reason the
        // extension does: SQLite's write lock serializes us against a
        // concurrent extension for this member, so the two can't both
        // compute from the same starting date and lose one another.
        let current_dues: Option<Option<DateTime<Utc>>> =
            sqlx::query_scalar("SELECT dues_paid_until FROM members WHERE id = ?")
                .bind(member_id.to_string())
                .fetch_optional(&mut *tx)
                .await
                .map_err(AppError::Database)?;

        let Some(Some(current_dues)) = current_dues else {
            // Member row gone, or never had a dues window to reduce.
            tx.commit().await.map_err(AppError::Database)?;
            return Ok(None);
        };

        // Status is left alone on purpose — the daily expiration sweep
        // owns the Active→Expired transition and reads this same column.
        sqlx::query(
            "UPDATE members SET dues_paid_until = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(
            current_dues
                .checked_sub_signed(chrono::Duration::seconds(seconds))
                .unwrap_or(current_dues),
        )
        .bind(member_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;

        tx.commit().await.map_err(AppError::Database)?;
        Ok(Some((member_id, seconds)))
    }

    async fn find_event_fee_payment(
        &self,
        event_id: Uuid,
        payer: &crate::domain::Payer,
    ) -> Result<Option<Payment>> {
        self.find_paid_registration(PAID_EVENT, event_id, payer)
            .await
    }

    async fn list_completed_event_fees(&self, event_id: Uuid) -> Result<Vec<Payment>> {
        self.list_completed_registrations(PAID_EVENT, event_id)
            .await
    }

    async fn find_series_pass_payment(
        &self,
        series_id: Uuid,
        payer: &crate::domain::Payer,
    ) -> Result<Option<Payment>> {
        self.find_paid_registration(PAID_CLASS, series_id, payer)
            .await
    }

    async fn list_completed_series_passes(&self, series_id: Uuid) -> Result<Vec<Payment>> {
        self.list_completed_registrations(PAID_CLASS, series_id)
            .await
    }

    async fn revenue_by_month(&self, months_back: u32) -> Result<Vec<MonthlyRevenue>> {
        // SQLite-friendly: strftime extracts year/month; we filter on
        // paid_at being non-null AND status='Completed' so refunded /
        // pending / failed rows don't pollute the totals. The cutoff
        // is `now - months_back months`, computed at the DB level so
        // all timestamps stay UTC.
        //
        // Result is ordered newest-month first; the dashboard
        // presents months top-down. payment_type comes back as the
        // raw lowercase string and is stored on `MonthlyRevenue`
        // as-is — see the doc on that struct for why.
        let cutoff_months = months_back as i64;
        let rows: Vec<(String, String, String, i64, i64)> = sqlx::query_as(
            r#"
            SELECT
                strftime('%Y', paid_at)        AS year_str,
                strftime('%m', paid_at)        AS month_str,
                payment_type                    AS payment_type,
                SUM(amount_cents)               AS total_cents,
                COUNT(*)                        AS payment_count
            FROM payments
            WHERE status = 'Completed'
              AND paid_at IS NOT NULL
              AND paid_at >= datetime('now', ?)
            GROUP BY year_str, month_str, payment_type
            ORDER BY year_str DESC, month_str DESC, payment_type ASC
            "#,
        )
        .bind(format!("-{} months", cutoff_months))
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        let mut out = Vec::with_capacity(rows.len());
        for (year_str, month_str, type_str, total, count) in rows {
            let year: i32 = year_str
                .parse()
                .map_err(|e: std::num::ParseIntError| AppError::Internal(e.to_string()))?;
            let month: u32 = month_str
                .parse()
                .map_err(|e: std::num::ParseIntError| AppError::Internal(e.to_string()))?;
            out.push(MonthlyRevenue {
                year,
                month,
                payment_type: type_str,
                total_cents: total,
                payment_count: count,
            });
        }
        Ok(out)
    }
}
