//! Daily expiration sweep: members past dues + grace period get
//! status flipped to `Expired` and their live sessions invalidated.
//!
//! Standalone — only the daily job in `main.rs` (via `BillingService`
//! facade → `Expiration::check_expired_members`) drives it. Doesn't
//! share state or helpers with the auto-renew lifecycle or the
//! notifications module.

use sqlx::SqlitePool;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    error::{AppError, Result},
    integrations::{IntegrationEvent, IntegrationManager},
    repository::MemberRepository,
    service::settings_service::SettingsService,
};

pub struct Expiration {
    member_repo: Arc<dyn MemberRepository>,
    settings_service: Arc<SettingsService>,
    integration_manager: Arc<IntegrationManager>,
    /// Held for the multi-table sweep (UPDATE members RETURNING +
    /// DELETE FROM sessions). F1 left this site as raw SQL because
    /// no repo method covers the cross-table dependency.
    db_pool: SqlitePool,
}

impl Expiration {
    pub fn new(
        member_repo: Arc<dyn MemberRepository>,
        settings_service: Arc<SettingsService>,
        integration_manager: Arc<IntegrationManager>,
        db_pool: SqlitePool,
    ) -> Self {
        Self {
            member_repo,
            settings_service,
            integration_manager,
            db_pool,
        }
    }

    /// Check for members past grace period and expire them. Also kills
    /// any live sessions for the affected members so they stop having
    /// portal access on the next request rather than the one after.
    pub async fn check_expired_members(&self) -> Result<u32> {
        let grace_days = self.get_grace_period_days().await;

        // UPDATE...RETURNING gives us the affected IDs in one round-trip
        // so we can invalidate their sessions below.
        let expired_ids: Vec<(String,)> = sqlx::query_as(
            r#"
            UPDATE members
            SET status = 'Expired', updated_at = CURRENT_TIMESTAMP
            WHERE status = 'Active'
              AND dues_paid_until IS NOT NULL
              AND date(dues_paid_until, '+' || ? || ' days') < date('now')
              AND bypass_dues = 0
            RETURNING id
            "#,
        )
        .bind(grace_days)
        .fetch_all(&self.db_pool)
        .await
        .map_err(|e| AppError::Internal(format!("Database error: {}", e)))?;

        let expired_count = expired_ids.len() as u32;

        // Force-logout expired members. `require_auth_redirect` would
        // bounce them to /portal/restore on their next request anyway,
        // but killing the session makes the expiration immediate from
        // the browser's perspective.
        //
        // Placeholder count is derived from our own DB row count, not
        // user input — format!() is safe here.
        if !expired_ids.is_empty() {
            let placeholders = vec!["?"; expired_ids.len()].join(",");
            let sql = format!("DELETE FROM sessions WHERE member_id IN ({})", placeholders);
            let mut q = sqlx::query(&sql);
            for (id,) in &expired_ids {
                q = q.bind(id);
            }
            if let Err(e) = q.execute(&self.db_pool).await {
                tracing::warn!(
                    "Marked {} members Expired but session cleanup failed: {}. \
                     Middleware still rejects Expired status, so members are \
                     bounced to /portal/restore on next request.",
                    expired_count, e
                );
            }
        }

        // Fire MemberExpired events so integrations (Discord role swap,
        // future Unifi access revocation) can react. Best-effort — a
        // failure here doesn't roll back the expiration.
        for (id_str,) in &expired_ids {
            if let Ok(uuid) = Uuid::parse_str(id_str) {
                if let Ok(Some(member)) = self.member_repo.find_by_id(uuid).await {
                    self.integration_manager
                        .handle_event(IntegrationEvent::MemberExpired(member))
                        .await;
                }
            }
        }

        if expired_count > 0 {
            tracing::info!(
                "Expired {} members past grace period ({} days); sessions invalidated",
                expired_count,
                grace_days
            );
        }

        Ok(expired_count)
    }

    async fn get_grace_period_days(&self) -> i64 {
        self.settings_service
            .get_number("membership.grace_period_days")
            .await
            .unwrap_or(3)
    }
}
