//! Idempotency claim against `processed_stripe_events`.
//!
//! Stripe webhook delivery is at-least-once: the same event can arrive
//! twice if a previous response was lost or slow. This repo's job is the
//! atomic "have we already handled this event id?" claim that fronts
//! the per-event-type dispatch in `WebhookDispatcher`. The pair of
//! methods here is small but load-bearing — without `release`, a
//! mid-handler failure would leave the event permanently claimed and
//! the next Stripe retry would skip without re-running the failed step.

use async_trait::async_trait;
use sqlx::SqlitePool;

use crate::error::{AppError, Result};

#[async_trait]
pub trait ProcessedEventsRepository: Send + Sync {
    /// Atomically claim an event id. Returns `true` if this caller
    /// won the claim (i.e. the event has not been processed before),
    /// `false` if a prior worker / retry already claimed it. Implemented
    /// as `INSERT OR IGNORE` so concurrent callers are race-safe.
    async fn claim(&self, event_id: &str, event_type: &str) -> Result<bool>;
    /// Release a previously-claimed event. Called when a handler fails
    /// after the claim, so the next Stripe retry can re-run. Best-effort
    /// — if THIS fails the original error is the more important signal.
    async fn release(&self, event_id: &str) -> Result<()>;
}

pub struct SqliteProcessedEventsRepository {
    pool: SqlitePool,
}

impl SqliteProcessedEventsRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ProcessedEventsRepository for SqliteProcessedEventsRepository {
    async fn claim(&self, event_id: &str, event_type: &str) -> Result<bool> {
        let result = sqlx::query(
            "INSERT OR IGNORE INTO processed_stripe_events (event_id, event_type) VALUES (?, ?)",
        )
        .bind(event_id)
        .bind(event_type)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Internal(format!("Idempotency claim failed: {}", e)))?;
        Ok(result.rows_affected() == 1)
    }

    async fn release(&self, event_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM processed_stripe_events WHERE event_id = ?")
            .bind(event_id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::Internal(format!("Idempotency release failed: {}", e)))?;
        Ok(())
    }
}
