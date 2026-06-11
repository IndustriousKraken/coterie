//! Service that owns the full side-effect chain for admin-driven
//! member mutations: repo update → session invalidation (where
//! applicable) → audit log → integration dispatch → transactional
//! emails. Handlers parse the wire shape and render the response;
//! everything between belongs here.
//!
//! Mirrors `PaymentService`'s shape — a per-domain service that
//! co-locates validation, persistence, and the post-work chain so a
//! contributor adding a new admin action can't accidentally forget
//! one piece (audit, integration event, session invalidation, email).
//! See the `member-admin-service` capability spec for the contract.
//!
//! Organized as a module directory with per-concern submodules so no
//! file exceeds ~400 lines. When adding a new admin action, place its
//! method in the submodule matching its concern:
//!
//! - [`status`] — `activate`, `suspend`, `expire_now`
//! - [`dues`] — `extend_dues`, `set_dues`
//! - [`updates`] — `update`, `update_discord_id`, `resend_verification`
//! - [`create`] — `create`, `send_welcome_email`
//! - [`bulk_import`] — `bulk_import` (extracted for size)
//! - [`queries`] — `audit_export`, `membership_type_name`
//! - [`events`] — `dispatch_member_updated` (private helper)

use std::sync::Arc;

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    auth::AuthService,
    domain::MemberStatus,
    email::EmailSender,
    integrations::IntegrationManager,
    repository::MemberRepository,
    service::{
        audit_service::AuditService, membership_type_service::MembershipTypeService,
        settings_service::SettingsService,
    },
};

mod bulk_import;
mod create;
mod dues;
mod events;
mod queries;
mod status;
mod updates;

#[cfg(test)]
mod test_helpers;

/// One parsed CSV row, ready for the service to validate and insert.
/// The handler is responsible for turning the raw CSV bytes into this
/// shape — the service stays format-agnostic so a future JSON/API
/// import path can call `bulk_import` with the same struct.
pub struct ImportRow {
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub membership_type_slug: String,
    pub status: Option<MemberStatus>,
    pub notes: Option<String>,
    pub discord_id: Option<String>,
    /// Billing-migration fields. All optional. When supplied, they
    /// seed the corresponding columns on the created member so an
    /// imported row from an existing billing system can preserve its
    /// paid-through date, Stripe linkage, historical join date, and
    /// already-verified email. See the `bulk-member-csv-import`
    /// capability spec for the per-field semantics.
    pub dues_paid_until: Option<DateTime<Utc>>,
    pub stripe_customer_id: Option<String>,
    pub stripe_subscription_id: Option<String>,
    pub joined_at: Option<DateTime<Utc>>,
    pub email_verified_at: Option<DateTime<Utc>>,
    /// Sentinel: parser sets this when a cell couldn't be coerced
    /// (e.g., malformed timestamp). `bulk_import` checks this first
    /// and fails the row with the carried reason so the row_index in
    /// the resulting summary aligns with the original CSV position.
    /// `None` for well-formed rows.
    pub parse_error: Option<String>,
}

/// A single row that didn't make it into the database. `row_index` is
/// 1-based and counts data rows (header is row 0; first data row is 1)
/// so operators can match it against the source spreadsheet.
pub struct ImportFailure {
    pub row_index: usize,
    pub email: Option<String>,
    pub reason: String,
}

/// Aggregate result of a bulk import. Per-row failures don't abort the
/// batch — see the `bulk-member-csv-import` capability spec for the
/// full failure-mode matrix.
pub struct BulkImportSummary {
    pub succeeded: u32,
    pub failed: u32,
    pub failures: Vec<ImportFailure>,
    pub created_member_ids: Vec<Uuid>,
}

pub struct MemberService {
    member_repo: Arc<dyn MemberRepository>,
    auth_service: Arc<AuthService>,
    audit_service: Arc<AuditService>,
    integration_manager: Arc<IntegrationManager>,
    email_sender: Arc<dyn EmailSender>,
    membership_type_service: Arc<MembershipTypeService>,
    settings_service: Arc<SettingsService>,
    db_pool: SqlitePool,
    /// Public base URL of this Coterie instance, used to build the
    /// portal and verification links inside transactional emails.
    /// Pulled from `Settings::server::base_url` at startup; we keep a
    /// copy here so emails can be sent without reaching back into
    /// `AppState`.
    base_url: String,
}

impl MemberService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        member_repo: Arc<dyn MemberRepository>,
        auth_service: Arc<AuthService>,
        audit_service: Arc<AuditService>,
        integration_manager: Arc<IntegrationManager>,
        email_sender: Arc<dyn EmailSender>,
        membership_type_service: Arc<MembershipTypeService>,
        settings_service: Arc<SettingsService>,
        db_pool: SqlitePool,
        base_url: String,
    ) -> Self {
        Self {
            member_repo,
            auth_service,
            audit_service,
            integration_manager,
            email_sender,
            membership_type_service,
            settings_service,
            db_pool,
            base_url,
        }
    }
}
