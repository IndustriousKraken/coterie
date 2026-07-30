//! Service that owns the full side-effect chain for admin-driven
//! payment mutations: rate-limit → validate → atomic claim → call
//! Stripe (or no-op for manual) → audit log → integration dispatch.
//! Handlers parse the wire shape and render the response; the service
//! owns everything between.
//!
//! Mirrors the per-domain admin services (`MemberService`,
//! `EventAdminService`, `AnnouncementAdminService`) so a contributor
//! adding a new admin payment action can't accidentally forget one
//! piece (rate-limit, audit, integration event). See the
//! `payment-admin-service` capability spec for the contract.
//!
//! Today the only operation is `refund`. Future admin-payment actions
//! (partial refund, manual void, etc.) should extend this service
//! rather than re-implementing the chain inline in handlers.

use std::net::IpAddr;
use std::sync::Arc;

use uuid::Uuid;

use crate::{
    api::state::MoneyLimiter,
    domain::{PaymentMethod, PaymentStatus},
    error::AppError,
    integrations::{IntegrationEvent, IntegrationManager},
    payments::StripeHandle,
    repository::{EventRepository, PaymentRepository, SeriesEnrollmentRepository},
    service::audit_service::AuditService,
};

/// Typed success result of `PaymentAdminService::refund`. Carries the
/// data the handler needs to render a success fragment without having
/// to re-fetch the payment row.
#[derive(Debug)]
pub struct RefundOutcome {
    pub amount_cents: i64,
    pub stripe_refund_id: Option<String>,
    pub detail: String,
    pub payment_method: PaymentMethod,
}

/// Typed failure modes of `PaymentAdminService::refund`. Each variant
/// maps to a distinct user-facing message via `user_message()`. The
/// service stays display-agnostic; the handler renders the string.
#[derive(Debug)]
pub enum RefundError {
    RateLimited,
    PaymentNotFound,
    AlreadyRefunded,
    NotCompleted,
    WaivedNoRefund,
    StripeNotConfigured,
    NoStripeReferenceOnRecord,
    AnotherActorClaimedFirst,
    /// Carries the upstream error message for logging — the handler
    /// renders a generic "Stripe refund failed" string.
    StripeApiError(String),
    InternalDatabaseError(AppError),
}

impl RefundError {
    /// Human-readable message for the failure fragment. Static strings
    /// only so the handler doesn't need to allocate per call.
    pub fn user_message(&self) -> &'static str {
        match self {
            RefundError::RateLimited => "Too many refund attempts — try again in a minute.",
            RefundError::PaymentNotFound => "Payment not found",
            RefundError::AlreadyRefunded => "Payment is already refunded",
            RefundError::NotCompleted => "Only completed payments can be refunded",
            RefundError::WaivedNoRefund => {
                "Waived payments are $0 — nothing to refund. Use suspend or expire instead."
            }
            RefundError::StripeNotConfigured => {
                "Stripe isn't configured. Can't issue an API refund."
            }
            RefundError::NoStripeReferenceOnRecord => {
                "Stripe payment has no Stripe ID on record — can't refund through the API. Mark Refunded manually if needed."
            }
            RefundError::AnotherActorClaimedFirst => {
                "Payment was already refunded (or its status changed) by another action."
            }
            RefundError::StripeApiError(_) => "Stripe refund failed — see server logs.",
            RefundError::InternalDatabaseError(_) => "Database error — see server logs.",
        }
    }
}

pub struct PaymentAdminService {
    payment_repo: Arc<dyn PaymentRepository>,
    /// Refunding an event fee also releases its seat — a member doesn't
    /// keep a seat for an event whose fee went back to them.
    event_repo: Arc<dyn EventRepository>,
    /// Refunding a class pass cancels the enrollment and its remaining
    /// sessions, for the same reason.
    enrollment_repo: Arc<dyn SeriesEnrollmentRepository>,
    /// Hot-swappable Stripe wiring — read `current().client` at refund
    /// time (not captured once) so a portal key rotation reaches this
    /// path without a restart, same as the webhook + per-request charges.
    stripe_handle: Arc<StripeHandle>,
    audit_service: Arc<AuditService>,
    integration_manager: Arc<IntegrationManager>,
    money_limiter: MoneyLimiter,
}

impl PaymentAdminService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        payment_repo: Arc<dyn PaymentRepository>,
        event_repo: Arc<dyn EventRepository>,
        enrollment_repo: Arc<dyn SeriesEnrollmentRepository>,
        stripe_handle: Arc<StripeHandle>,
        audit_service: Arc<AuditService>,
        integration_manager: Arc<IntegrationManager>,
        money_limiter: MoneyLimiter,
    ) -> Self {
        Self {
            payment_repo,
            event_repo,
            enrollment_repo,
            stripe_handle,
            audit_service,
            integration_manager,
            money_limiter,
        }
    }

    /// Refund a previously-recorded payment. Behavior depends on
    /// payment_method:
    ///
    ///   - `Stripe`  → call Stripe's Refund API (full refund), then
    ///                 mark the local Payment row as Refunded
    ///   - `Manual`  → just mark the local row as Refunded; admin
    ///                 presumably returned cash / wrote a check etc.
    ///                 out-of-band
    ///   - `Waived`  → reject (nothing to refund — the row was $0
    ///                 to begin with)
    ///
    /// Already-Refunded payments return `Err(AlreadyRefunded)` without
    /// touching Stripe again (idempotent against double-clicks).
    ///
    /// Refunds DO NOT roll back `dues_paid_until`. Refunding is
    /// usually a customer-service gesture rather than an access
    /// revocation; an admin can manually adjust dues afterward via
    /// the existing extend/set-dues UI if they actually want to kick
    /// someone out.
    pub async fn refund(
        &self,
        actor_id: Uuid,
        payment_id: Uuid,
        ip: IpAddr,
    ) -> Result<RefundOutcome, RefundError> {
        // 1. Rate-limit. Money-moving actions are always per-IP capped;
        //    owning the check here means a caller can't accidentally
        //    skip it by going around the handler.
        if !self.money_limiter.0.check_and_record(ip, "admin.refund") {
            return Err(RefundError::RateLimited);
        }
        self.refund_claimed(actor_id, payment_id).await
    }

    /// Refund every `Completed` event-fee payment on an event, in one
    /// operator action. Used by the delete-an-event path, which MUST
    /// refund before deleting: `event_attendance` cascades on event
    /// delete, so delete-then-refund would destroy the roster while the
    /// charges stood. Aborts on the FIRST failure and returns it, so
    /// the caller leaves the event in place and fixable.
    ///
    /// The per-IP limit is checked once for the whole sweep rather than
    /// once per attendee: this is a single admin action, and charging
    /// it per attendee would abort a large event's delete halfway on the
    /// eleventh refund.
    pub async fn refund_all_event_fees(
        &self,
        actor_id: Uuid,
        event_id: Uuid,
        ip: IpAddr,
    ) -> Result<usize, RefundError> {
        if !self
            .money_limiter
            .0
            .check_and_record(ip, "admin.refund_event_fees")
        {
            return Err(RefundError::RateLimited);
        }

        let payments = self
            .payment_repo
            .list_completed_event_fees(event_id)
            .await
            .map_err(RefundError::InternalDatabaseError)?;
        if payments.is_empty() {
            return Ok(0);
        }

        let mut refunded = 0usize;
        for payment in &payments {
            self.refund_claimed(actor_id, payment.id).await?;
            refunded += 1;
        }

        self.audit_service
            .log(
                Some(actor_id),
                "refund_event_fees_bulk",
                "event",
                &event_id.to_string(),
                None,
                Some(&format!("{} event-fee payments refunded", refunded)),
                None,
            )
            .await;

        Ok(refunded)
    }

    /// Refund every `Completed` series-pass payment on a class, in one
    /// operator action. The series-scope sibling of
    /// [`Self::refund_all_event_fees`], used by the delete-a-series path,
    /// which MUST refund before deleting: occurrences and their attendance
    /// cascade on series delete, so delete-then-refund would destroy the
    /// roster while the charges stood. Aborts on the FIRST failure and
    /// returns it, so the caller leaves the class in place and fixable.
    pub async fn refund_all_series_passes(
        &self,
        actor_id: Uuid,
        series_id: Uuid,
        ip: IpAddr,
    ) -> Result<usize, RefundError> {
        if !self
            .money_limiter
            .0
            .check_and_record(ip, "admin.refund_series_passes")
        {
            return Err(RefundError::RateLimited);
        }

        let payments = self
            .payment_repo
            .list_completed_series_passes(series_id)
            .await
            .map_err(RefundError::InternalDatabaseError)?;
        if payments.is_empty() {
            return Ok(0);
        }

        let mut refunded = 0usize;
        for payment in &payments {
            self.refund_claimed(actor_id, payment.id).await?;
            refunded += 1;
        }

        self.audit_service
            .log(
                Some(actor_id),
                "refund_series_passes_bulk",
                "event_series",
                &series_id.to_string(),
                None,
                Some(&format!("{} series-pass payments refunded", refunded)),
                None,
            )
            .await;

        Ok(refunded)
    }

    /// The refund chain minus the rate-limit check, so the single-payment
    /// route and the delete-time sweep share one implementation of
    /// validate → claim → Stripe → seat → audit → alert.
    async fn refund_claimed(
        &self,
        actor_id: Uuid,
        payment_id: Uuid,
    ) -> Result<RefundOutcome, RefundError> {
        // 2. Load the payment row.
        let payment = self
            .payment_repo
            .find_by_id(payment_id)
            .await
            .map_err(RefundError::InternalDatabaseError)?
            .ok_or(RefundError::PaymentNotFound)?;

        // 3. Status / method validation.
        if payment.status == PaymentStatus::Refunded {
            return Err(RefundError::AlreadyRefunded);
        }
        if payment.status != PaymentStatus::Completed {
            return Err(RefundError::NotCompleted);
        }
        if payment.payment_method == PaymentMethod::Waived {
            return Err(RefundError::WaivedNoRefund);
        }

        // 4. Atomic claim BEFORE calling Stripe. Two simultaneous
        //    admin clicks both reach this point, but only one wins
        //    the Completed→Refunded flip; the other bails. Without
        //    this, both calls would invoke Stripe (idempotency-keyed
        //    so Stripe dedupes, but the audit log would still get two
        //    entries with different actors).
        let claimed = self
            .payment_repo
            .claim_payment_for_refund(payment.id)
            .await
            .map_err(RefundError::InternalDatabaseError)?;
        if !claimed {
            return Err(RefundError::AnotherActorClaimedFirst);
        }

        // 5. Branch on payment_method.
        let stripe_refund_id: Option<String> = match payment.payment_method {
            PaymentMethod::Waived => unreachable!("Waived already short-circuited above"),
            PaymentMethod::Stripe => {
                let stripe_ref = match payment.external_id.as_ref() {
                    Some(r) if !r.as_str().is_empty() => r,
                    _ => {
                        let _ = self.payment_repo.unclaim_refund(payment.id).await;
                        return Err(RefundError::NoStripeReferenceOnRecord);
                    }
                };
                // Read the CURRENT client from the hot-swappable handle so
                // a portal key rotation is picked up here without a restart
                // (same handle the webhook + per-request charge paths read).
                let runtime = self.stripe_handle.current();
                let stripe_client = match runtime.client.as_ref() {
                    Some(c) => c,
                    None => {
                        let _ = self.payment_repo.unclaim_refund(payment.id).await;
                        return Err(RefundError::StripeNotConfigured);
                    }
                };
                match stripe_client
                    .refund_payment(stripe_ref, &payment.id.to_string())
                    .await
                {
                    Ok(refund_id) => Some(refund_id),
                    Err(e) => {
                        // Stripe rejected — roll the local row back so
                        // a future retry can re-claim and re-issue.
                        let _ = self.payment_repo.unclaim_refund(payment.id).await;
                        return Err(RefundError::StripeApiError(e.to_string()));
                    }
                }
            }
            PaymentMethod::Manual => None, // No external system to update.
        };

        // 6. Build the human-readable detail.
        let detail = match (&payment.payment_method, &stripe_refund_id) {
            (PaymentMethod::Stripe, Some(rid)) => format!(
                "Refunded ${:.2} via Stripe (refund {})",
                payment.amount_cents as f64 / 100.0,
                rid,
            ),
            (PaymentMethod::Manual, _) => format!(
                "Marked ${:.2} manual payment as Refunded (no API call — refund the cash/check yourself)",
                payment.amount_cents as f64 / 100.0,
            ),
            _ => format!("Refunded ${:.2}", payment.amount_cents as f64 / 100.0),
        };

        // 7. A refunded event fee releases its seat. Done before the
        //    audit so the audit row describes a completed transition.
        if let Some(event_id) = payment.kind.event_id() {
            if let Err(e) = self.event_repo.cancel_seat_for_payment(payment.id).await {
                // The money is already back; a stuck seat is the lesser
                // problem and an admin can release it from the roster.
                tracing::error!(
                    "Refunded event-fee payment {} but could not cancel its seat: {}",
                    payment.id,
                    e,
                );
            }
            self.audit_service
                .log(
                    Some(actor_id),
                    "event_registration_refunded",
                    "event",
                    &event_id.to_string(),
                    None,
                    Some(&detail),
                    None,
                )
                .await;
        }

        // A refunded class pass cancels the enrollment and its attendance
        // for sessions that haven't started; past sessions keep theirs.
        if let Some(series_id) = payment.kind.series_id() {
            if let Err(e) =
                crate::service::series_enrollment_service::cancel_enrollment_for_payment(
                    &*self.enrollment_repo,
                    &*self.event_repo,
                    payment.id,
                )
                .await
            {
                // The money is already back; a stuck enrollment is the
                // lesser problem and an admin can release it.
                tracing::error!(
                    "Refunded series-pass payment {} but could not cancel its enrollment: {}",
                    payment.id,
                    e,
                );
            }
            self.audit_service
                .log(
                    Some(actor_id),
                    "series_enrollment_refunded",
                    "event_series",
                    &series_id.to_string(),
                    None,
                    Some(&detail),
                    None,
                )
                .await;
        }

        // 8. Audit. Failures are logged via tracing and swallowed
        //    inside AuditService::log.
        self.audit_service
            .log(
                Some(actor_id),
                "refund_payment",
                "payment",
                &payment_id.to_string(),
                None,
                Some(&detail),
                None,
            )
            .await;

        // 9. Visibility: a refund is unusual enough to alert on.
        //    Per-integration failures are logged inside
        //    IntegrationManager; the call always returns.
        self.integration_manager
            .handle_event(IntegrationEvent::AdminAlert {
                subject: format!(
                    "Payment refunded — ${:.2}",
                    payment.amount_cents as f64 / 100.0,
                ),
                body: format!(
                    "Refunded by: {}\nPayer: {:?}\nMethod: {:?}\nDetail: {}",
                    actor_id, payment.payer, payment.payment_method, detail,
                ),
            })
            .await;

        Ok(RefundOutcome {
            amount_cents: payment.amount_cents,
            stripe_refund_id,
            detail,
            payment_method: payment.payment_method,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use crate::{
        api::state::RateLimiter,
        domain::{Payer, Payment, PaymentKind, StripeRef},
        payments::StripeClient,
        repository::{PaymentRepository, SqlitePaymentRepository},
    };
    use sqlx::{Executor, SqlitePool};

    async fn fresh_pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .after_connect(|conn, _| {
                Box::pin(async move {
                    conn.execute("PRAGMA foreign_keys = ON").await?;
                    Ok(())
                })
            })
            .connect("sqlite::memory:")
            .await
            .expect(":memory:");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrate");
        pool
    }

    fn loopback() -> IpAddr {
        IpAddr::from([127, 0, 0, 1])
    }

    fn make_service(
        pool: SqlitePool,
        stripe_client: Option<Arc<StripeClient>>,
        money_limiter: MoneyLimiter,
    ) -> (PaymentAdminService, Arc<dyn PaymentRepository>) {
        let payment_repo: Arc<dyn PaymentRepository> =
            Arc::new(SqlitePaymentRepository::new(pool.clone()));
        let event_repo: Arc<dyn EventRepository> =
            Arc::new(crate::repository::SqliteEventRepository::new(pool.clone()));
        let audit = Arc::new(AuditService::new(pool.clone()));
        let integrations = Arc::new(IntegrationManager::new());
        // Wrap the (optional) client in a fixed handle — these unit tests
        // exercise the present/absent-client refund branch, not hot-reload.
        let stripe_handle = Arc::new(StripeHandle::preloaded(stripe_client, None));
        let enrollment_repo: Arc<dyn SeriesEnrollmentRepository> = Arc::new(
            crate::repository::SqliteSeriesEnrollmentRepository::new(pool.clone()),
        );
        let svc = PaymentAdminService::new(
            payment_repo.clone(),
            event_repo,
            enrollment_repo,
            stripe_handle,
            audit,
            integrations,
            money_limiter,
        );
        (svc, payment_repo)
    }

    fn permissive_limiter() -> MoneyLimiter {
        MoneyLimiter(RateLimiter::new(1000, Duration::from_secs(60)))
    }

    fn saturated_limiter() -> MoneyLimiter {
        let lim = RateLimiter::new(0, Duration::from_secs(60));
        MoneyLimiter(lim)
    }

    async fn make_actor(pool: &SqlitePool) -> Uuid {
        use crate::domain::CreateMemberRequest;
        use crate::repository::{MemberRepository, SqliteMemberRepository};
        let repo = SqliteMemberRepository::new(pool.clone());
        let m = repo
            .create(CreateMemberRequest {
                email: format!("a-{}@example.com", Uuid::new_v4()),
                username: format!("u_{}", Uuid::new_v4().simple()),
                full_name: "Test Admin".to_string(),
                password: "p4ssword_long_enough".to_string(),
                membership_type_id: None,
                ..Default::default()
            })
            .await
            .unwrap();
        m.id
    }

    async fn insert_payment(
        repo: &Arc<dyn PaymentRepository>,
        member_id: Uuid,
        method: PaymentMethod,
        status: PaymentStatus,
        external_id: Option<StripeRef>,
    ) -> Payment {
        let now = chrono::Utc::now();
        let p = Payment {
            id: Uuid::new_v4(),
            payer: Payer::Member(member_id),
            amount_cents: 5000,
            currency: "USD".to_string(),
            status,
            payment_method: method,
            kind: PaymentKind::Membership,
            external_id,
            description: "test".to_string(),
            paid_at: Some(now),
            created_at: now,
            updated_at: now,
        };
        repo.create(p).await.unwrap()
    }

    async fn audit_count(pool: &SqlitePool, action: &str, entity_id: &str) -> i64 {
        let (n,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM audit_logs WHERE action = ? AND entity_id = ?")
                .bind(action)
                .bind(entity_id)
                .fetch_one(pool)
                .await
                .unwrap();
        n
    }

    #[tokio::test]
    async fn manual_refund_success_no_stripe_call() {
        let pool = fresh_pool().await;
        let (svc, repo) = make_service(pool.clone(), None, permissive_limiter());
        let actor = make_actor(&pool).await;
        let payment = insert_payment(
            &repo,
            actor,
            PaymentMethod::Manual,
            PaymentStatus::Completed,
            None,
        )
        .await;

        let outcome = svc.refund(actor, payment.id, loopback()).await.unwrap();
        assert!(outcome.stripe_refund_id.is_none());
        assert_eq!(outcome.amount_cents, 5000);
        assert!(outcome.detail.contains("Marked $50.00 manual payment"));

        // Row flipped to Refunded.
        let after = repo.find_by_id(payment.id).await.unwrap().unwrap();
        assert_eq!(after.status, PaymentStatus::Refunded);

        // Audit row written.
        assert_eq!(
            audit_count(&pool, "refund_payment", &payment.id.to_string()).await,
            1
        );
    }

    #[tokio::test]
    async fn already_refunded_short_circuits() {
        let pool = fresh_pool().await;
        let (svc, repo) = make_service(pool.clone(), None, permissive_limiter());
        let actor = make_actor(&pool).await;
        let payment = insert_payment(
            &repo,
            actor,
            PaymentMethod::Manual,
            PaymentStatus::Refunded,
            None,
        )
        .await;

        let err = svc.refund(actor, payment.id, loopback()).await.unwrap_err();
        assert!(matches!(err, RefundError::AlreadyRefunded));

        // No audit row written.
        assert_eq!(
            audit_count(&pool, "refund_payment", &payment.id.to_string()).await,
            0
        );
    }

    #[tokio::test]
    async fn not_completed_rejected() {
        let pool = fresh_pool().await;
        let (svc, repo) = make_service(pool.clone(), None, permissive_limiter());
        let actor = make_actor(&pool).await;
        let payment = insert_payment(
            &repo,
            actor,
            PaymentMethod::Manual,
            PaymentStatus::Pending,
            None,
        )
        .await;

        let err = svc.refund(actor, payment.id, loopback()).await.unwrap_err();
        assert!(matches!(err, RefundError::NotCompleted));

        assert_eq!(
            audit_count(&pool, "refund_payment", &payment.id.to_string()).await,
            0
        );
    }

    #[tokio::test]
    async fn waived_rejected() {
        let pool = fresh_pool().await;
        let (svc, repo) = make_service(pool.clone(), None, permissive_limiter());
        let actor = make_actor(&pool).await;
        let payment = insert_payment(
            &repo,
            actor,
            PaymentMethod::Waived,
            PaymentStatus::Completed,
            None,
        )
        .await;

        let err = svc.refund(actor, payment.id, loopback()).await.unwrap_err();
        assert!(matches!(err, RefundError::WaivedNoRefund));
    }

    #[tokio::test]
    async fn payment_not_found() {
        let pool = fresh_pool().await;
        let (svc, _repo) = make_service(pool.clone(), None, permissive_limiter());
        let actor = make_actor(&pool).await;

        let err = svc
            .refund(actor, Uuid::new_v4(), loopback())
            .await
            .unwrap_err();
        assert!(matches!(err, RefundError::PaymentNotFound));
    }

    #[tokio::test]
    async fn rate_limited_short_circuits_before_any_io() {
        let pool = fresh_pool().await;
        let (svc, repo) = make_service(pool.clone(), None, saturated_limiter());
        let actor = make_actor(&pool).await;
        let payment = insert_payment(
            &repo,
            actor,
            PaymentMethod::Manual,
            PaymentStatus::Completed,
            None,
        )
        .await;

        let err = svc.refund(actor, payment.id, loopback()).await.unwrap_err();
        assert!(matches!(err, RefundError::RateLimited));

        // No DB write — row still Completed, no audit.
        let after = repo.find_by_id(payment.id).await.unwrap().unwrap();
        assert_eq!(after.status, PaymentStatus::Completed);
        assert_eq!(
            audit_count(&pool, "refund_payment", &payment.id.to_string()).await,
            0
        );
    }

    #[tokio::test]
    async fn double_click_only_one_wins_claim() {
        // Two concurrent calls on the same Completed row. Only one
        // claim_payment_for_refund returns true; the other observes
        // AnotherActorClaimedFirst.
        let pool = fresh_pool().await;
        let (svc, repo) = make_service(pool.clone(), None, permissive_limiter());
        let svc = Arc::new(svc);
        let actor = make_actor(&pool).await;
        let payment = insert_payment(
            &repo,
            actor,
            PaymentMethod::Manual,
            PaymentStatus::Completed,
            None,
        )
        .await;

        let pid = payment.id;
        let svc1 = svc.clone();
        let svc2 = svc.clone();
        let h1 = tokio::spawn(async move { svc1.refund(actor, pid, loopback()).await });
        let h2 = tokio::spawn(async move { svc2.refund(actor, pid, loopback()).await });
        let r1 = h1.await.unwrap();
        let r2 = h2.await.unwrap();

        let (ok_count, race_count) = [&r1, &r2].iter().fold((0, 0), |(ok, race), r| match r {
            Ok(_) => (ok + 1, race),
            // The loser legitimately observes the race two ways depending on
            // scheduling: it either read `Completed` and then lost the atomic
            // claim (AnotherActorClaimedFirst), or — when the winner finished
            // marking the row Refunded before the loser's find_by_id read (more
            // likely on a slow/loaded CI runner) — it saw the refunded row
            // (AlreadyRefunded). Both correctly prevent a double refund.
            Err(RefundError::AnotherActorClaimedFirst | RefundError::AlreadyRefunded) => {
                (ok, race + 1)
            }
            Err(e) => panic!("unexpected error: {:?}", e),
        });
        assert_eq!(ok_count, 1, "exactly one call should succeed");
        assert_eq!(race_count, 1, "exactly one call should lose the race");

        assert_eq!(
            audit_count(&pool, "refund_payment", &pid.to_string()).await,
            1,
            "exactly one audit row",
        );
    }

    #[tokio::test]
    async fn stripe_method_without_client_unclaims_and_errors() {
        // Stripe-method payment but the service was built without a
        // configured client — should unclaim and return
        // StripeNotConfigured.
        let pool = fresh_pool().await;
        let (svc, repo) = make_service(pool.clone(), None, permissive_limiter());
        let actor = make_actor(&pool).await;
        let payment = insert_payment(
            &repo,
            actor,
            PaymentMethod::Stripe,
            PaymentStatus::Completed,
            Some(StripeRef::PaymentIntent("pi_test123".to_string())),
        )
        .await;

        let err = svc.refund(actor, payment.id, loopback()).await.unwrap_err();
        assert!(matches!(err, RefundError::StripeNotConfigured));

        // Row was unclaimed back to Completed.
        let after = repo.find_by_id(payment.id).await.unwrap().unwrap();
        assert_eq!(after.status, PaymentStatus::Completed);
        // No audit row on failure.
        assert_eq!(
            audit_count(&pool, "refund_payment", &payment.id.to_string()).await,
            0
        );
    }

    #[tokio::test]
    async fn stripe_method_without_reference_unclaims_and_errors() {
        // Stripe-method payment with no external_id — should unclaim
        // and return NoStripeReferenceOnRecord even when a client is
        // configured. (We can't easily construct a real StripeClient
        // here, so we exercise the no-client path covering the same
        // unclaim-on-validation-failure branch via the missing-ref
        // check below by leaving stripe_client None — see test above.
        // This test asserts the missing-ref branch is reachable; we
        // pass None so the missing-ref check fires first (it does,
        // since the match on external_id happens before the
        // stripe_client.as_ref() check).
        let pool = fresh_pool().await;
        let (svc, repo) = make_service(pool.clone(), None, permissive_limiter());
        let actor = make_actor(&pool).await;
        let payment = insert_payment(
            &repo,
            actor,
            PaymentMethod::Stripe,
            PaymentStatus::Completed,
            None,
        )
        .await;

        let err = svc.refund(actor, payment.id, loopback()).await.unwrap_err();
        assert!(matches!(err, RefundError::NoStripeReferenceOnRecord));

        let after = repo.find_by_id(payment.id).await.unwrap().unwrap();
        assert_eq!(after.status, PaymentStatus::Completed);
    }
}
