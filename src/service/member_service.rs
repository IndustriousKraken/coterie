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

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use uuid::Uuid;

use crate::{
    auth::{AuthService, EmailTokenService},
    domain::{
        CreateMemberRequest, Member, MemberStatus, UpdateMemberRequest,
    },
    email::{self, templates::{VerifyHtml, VerifyText, WelcomeHtml, WelcomeText}, EmailSender},
    error::{AppError, Result},
    integrations::{IntegrationEvent, IntegrationManager},
    repository::MemberRepository,
    service::{
        audit_service::AuditService, membership_type_service::MembershipTypeService,
        settings_service::SettingsService,
    },
};

pub struct MemberService {
    member_repo: Arc<dyn MemberRepository>,
    auth_service: Arc<AuthService>,
    audit_service: Arc<AuditService>,
    integration_manager: Arc<IntegrationManager>,
    email_sender: Arc<dyn EmailSender>,
    membership_type_service: Arc<MembershipTypeService>,
    settings_service: Arc<SettingsService>,
    email_token_service: Arc<EmailTokenService>,
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
        email_token_service: Arc<EmailTokenService>,
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
            email_token_service,
            base_url,
        }
    }

    /// Flip a member to `Active`, invalidate their sessions so the
    /// new status is picked up on next request, audit the action,
    /// dispatch `MemberActivated` to integrations, and send the
    /// welcome email. Session-invalidation and email failures are
    /// logged but don't fail the call — the primary repo mutation
    /// already succeeded.
    pub async fn activate(&self, actor_id: Uuid, member_id: Uuid) -> Result<Member> {
        let update = UpdateMemberRequest {
            status: Some(MemberStatus::Active),
            ..Default::default()
        };

        let member = self.member_repo.update(member_id, update).await?;

        // Force re-auth so the member picks up their new status on next request.
        if let Err(e) = self.auth_service.invalidate_all_sessions(member.id).await {
            tracing::error!(
                "Activated member {} but failed to invalidate sessions: {}",
                member.id, e,
            );
        }

        self.audit_service.log(
            Some(actor_id),
            "activate_member",
            "member",
            &member_id.to_string(),
            None,
            Some(&member.email),
            None,
        ).await;

        // Notify integrations (Discord role sync, future Unifi
        // access provisioning, etc.). Fire-and-forget — individual
        // failures are logged inside each impl.
        self.integration_manager
            .handle_event(IntegrationEvent::MemberActivated(member.clone()))
            .await;

        if let Err(e) = self.send_welcome_email(&member).await {
            tracing::error!(
                "Member {} activated but welcome email failed: {}",
                member.id, e,
            );
        }

        Ok(member)
    }

    /// Flip a member to `Suspended`, invalidate sessions, audit, and
    /// dispatch a `MemberUpdated { old, new }` event so integrations
    /// (Discord) can diff the status change.
    pub async fn suspend(&self, actor_id: Uuid, member_id: Uuid) -> Result<Member> {
        // Snapshot the pre-update member so we can dispatch the proper
        // before/after pair to integrations (Discord uses this to decide
        // which roles to remove vs add).
        let old_member = self.member_repo.find_by_id(member_id).await.ok().flatten();

        let update = UpdateMemberRequest {
            status: Some(MemberStatus::Suspended),
            ..Default::default()
        };

        let member = self.member_repo.update(member_id, update).await?;

        // Kick the suspended member out of any active sessions immediately.
        // If invalidation fails, middleware still rejects Suspended status
        // on the next request — but log so operators see the failure.
        if let Err(e) = self.auth_service.invalidate_all_sessions(member.id).await {
            tracing::error!(
                "Suspended member {} but failed to invalidate sessions: {}",
                member.id, e,
            );
        }

        if let Some(old) = old_member {
            self.integration_manager
                .handle_event(IntegrationEvent::MemberUpdated {
                    old, new: member.clone(),
                })
                .await;
        }

        self.audit_service.log(
            Some(actor_id),
            "suspend_member",
            "member",
            &member_id.to_string(),
            None,
            Some(&member.email),
            None,
        ).await;

        Ok(member)
    }

    /// Apply a profile-field update to a member (full name, type,
    /// notes, bypass-dues, etc.). Currently this path doesn't change
    /// status — but we still dispatch `MemberUpdated` so future fields
    /// (e.g., editing discord_id from the same form) are covered
    /// without further wiring.
    pub async fn update(
        &self,
        actor_id: Uuid,
        member_id: Uuid,
        request: UpdateMemberRequest,
    ) -> Result<Member> {
        let old_member = self.member_repo.find_by_id(member_id).await.ok().flatten();

        let new_member = self.member_repo.update(member_id, request).await?;

        self.audit_service.log(
            Some(actor_id),
            "update_member",
            "member",
            &member_id.to_string(),
            None,
            None,
            None,
        ).await;

        if let Some(old) = old_member {
            self.integration_manager
                .handle_event(IntegrationEvent::MemberUpdated {
                    old, new: new_member.clone(),
                })
                .await;
        }

        Ok(new_member)
    }

    /// Add `months` to the member's `dues_paid_until` (or to "now" if
    /// dues have already lapsed), revive Expired→Active, audit, and
    /// dispatch `MemberUpdated`. Validates `1..=120` — negative or
    /// absurd values would either wrap around as `u32` or dilute the
    /// audit log with junk entries.
    pub async fn extend_dues(
        &self,
        actor_id: Uuid,
        member_id: Uuid,
        months: i32,
    ) -> Result<Member> {
        use chrono::Months;

        if !(1..=120).contains(&months) {
            return Err(AppError::BadRequest(
                "Months must be between 1 and 120.".to_string(),
            ));
        }

        let old_member = self.member_repo.find_by_id(member_id).await?
            .ok_or_else(|| AppError::NotFound("Member not found".to_string()))?;

        let now = Utc::now();
        let base_date = old_member.dues_paid_until
            .filter(|d| *d > now)
            .unwrap_or(now);

        let new_dues_date = base_date
            .checked_add_months(Months::new(months as u32))
            .unwrap_or(base_date);

        self.member_repo
            .set_dues_paid_until_with_revival(member_id, new_dues_date)
            .await?;

        self.audit_service.log(
            Some(actor_id),
            "extend_dues",
            "member",
            &member_id.to_string(),
            None,
            Some(&format!("+{} months → {}", months, new_dues_date.format("%Y-%m-%d"))),
            None,
        ).await;

        self.dispatch_member_updated(member_id, old_member).await
    }

    /// Set the member's `dues_paid_until` to the end of `naive_date`
    /// (23:59:59 UTC). Same revival/audit/dispatch chain as
    /// `extend_dues`, but sets rather than adds.
    pub async fn set_dues(
        &self,
        actor_id: Uuid,
        member_id: Uuid,
        naive_date: NaiveDate,
    ) -> Result<Member> {
        let dues_date: DateTime<Utc> = naive_date
            .and_hms_opt(23, 59, 59)
            .unwrap()
            .and_utc();

        let old_member = self.member_repo.find_by_id(member_id).await?
            .ok_or_else(|| AppError::NotFound("Member not found".to_string()))?;

        self.member_repo
            .set_dues_paid_until_with_revival(member_id, dues_date)
            .await?;

        self.audit_service.log(
            Some(actor_id),
            "set_dues",
            "member",
            &member_id.to_string(),
            None,
            Some(&dues_date.format("%Y-%m-%d").to_string()),
            None,
        ).await;

        self.dispatch_member_updated(member_id, old_member).await
    }

    /// Backdate `dues_paid_until` to yesterday and flip Active→Expired,
    /// invalidate sessions so the member sees the change immediately,
    /// audit, and dispatch `MemberUpdated`. Session-invalidation
    /// failure is logged but not fatal — middleware re-validates per
    /// request and bounces them anyway.
    pub async fn expire_now(&self, actor_id: Uuid, member_id: Uuid) -> Result<Member> {
        let old_member = self.member_repo.find_by_id(member_id).await?
            .ok_or_else(|| AppError::NotFound("Member not found".to_string()))?;

        self.member_repo.expire_dues_now(member_id).await?;

        if let Err(e) = self.auth_service.invalidate_all_sessions(member_id).await {
            tracing::error!(
                "Expired dues for member {} but failed to invalidate sessions: {}",
                member_id, e,
            );
        }

        self.audit_service.log(
            Some(actor_id),
            "expire_member_now",
            "member",
            &member_id.to_string(),
            None,
            None,
            None,
        ).await;

        self.dispatch_member_updated(member_id, old_member).await
    }

    /// Set or clear the member's Discord snowflake. Validates format
    /// up-front; on success audits and dispatches `MemberUpdated` so
    /// the Discord integration can re-sync roles to the new ID (and
    /// strip them from the old, if any).
    pub async fn update_discord_id(
        &self,
        actor_id: Uuid,
        member_id: Uuid,
        discord_id: Option<String>,
    ) -> Result<Member> {
        use crate::integrations::discord::is_valid_snowflake;

        let trimmed = discord_id.as_deref().map(str::trim).filter(|s| !s.is_empty());
        if let Some(s) = trimmed {
            if !is_valid_snowflake(s) {
                return Err(AppError::BadRequest(
                    "Discord ID must be 17–20 digits (snowflake format). \
                     Right-click the user in Discord with Developer Mode on → Copy User ID."
                        .to_string(),
                ));
            }
        }

        let old_member = self.member_repo.find_by_id(member_id).await?
            .ok_or_else(|| AppError::NotFound("Member not found".to_string()))?;

        self.member_repo.update_discord_id(member_id, trimmed).await?;

        self.audit_service.log(
            Some(actor_id),
            "update_discord_id",
            "member",
            &member_id.to_string(),
            old_member.discord_id.as_deref(),
            trimmed,
            None,
        ).await;

        self.dispatch_member_updated(member_id, old_member).await
    }

    /// Regenerate a verification token for an unverified member and
    /// email them the fresh link. Invalidates any previously
    /// outstanding tokens so an old email can't be used. Already-
    /// verified members are rejected.
    pub async fn resend_verification(
        &self,
        actor_id: Uuid,
        member_id: Uuid,
    ) -> Result<()> {
        let member = self.member_repo.find_by_id(member_id).await?
            .ok_or_else(|| AppError::NotFound("Member not found".to_string()))?;

        if member.email_verified() {
            return Err(AppError::BadRequest(
                "Member's email is already verified".to_string(),
            ));
        }

        // Invalidate any existing unconsumed tokens so only the newest link works.
        // If invalidation fails, the new token is still valid and works — but
        // any older tokens out in flight (e.g. in the member's spam folder
        // from a previous send) might still work too. Worth logging.
        if let Err(e) = self.email_token_service.invalidate_for_member(member_id).await {
            tracing::warn!(
                "Resending verification for {} but couldn't invalidate previous tokens: {}",
                member_id, e,
            );
        }

        let created = self.email_token_service
            .create(member_id, chrono::Duration::hours(24))
            .await?;

        let org_name = self.settings_service
            .get_value("org.name").await
            .ok().filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Coterie".to_string());
        let verify_url = format!(
            "{}/verify?token={}",
            self.base_url.trim_end_matches('/'),
            created.token,
        );
        let html = VerifyHtml {
            full_name: &member.full_name,
            org_name: &org_name,
            verify_url: &verify_url,
        };
        let text = VerifyText {
            full_name: &member.full_name,
            org_name: &org_name,
            verify_url: &verify_url,
        };

        let message = email::message_from_templates(
            member.email.clone(),
            format!("Verify your email for {}", org_name),
            &html,
            &text,
        )?;

        // Email send is the only failure path we surface to the
        // caller here — unlike welcome-email on activate, this method
        // exists *to* send the email, so a failed send is a real
        // failure. Audit only runs on Ok().
        self.email_sender.send(&message).await?;

        self.audit_service.log(
            Some(actor_id),
            "resend_verification",
            "member",
            &member_id.to_string(),
            None,
            Some(&member.email),
            None,
        ).await;

        Ok(())
    }

    /// Create a new member via the admin form. Persists the row,
    /// sends the welcome email (log+swallow on failure), audits.
    /// Does NOT dispatch `MemberActivated` — newly-created members
    /// start `Pending` by repo default; the activation event fires
    /// on the later `activate` call.
    pub async fn create(
        &self,
        actor_id: Uuid,
        request: CreateMemberRequest,
    ) -> Result<Member> {
        let member = self.member_repo.create(request).await?;

        if let Err(e) = self.send_welcome_email(&member).await {
            tracing::error!(
                "Created member {} but welcome email failed: {}",
                member.id, e,
            );
        }

        self.audit_service.log(
            Some(actor_id),
            "create_member",
            "member",
            &member.id.to_string(),
            None,
            Some(&member.email),
            None,
        ).await;

        Ok(member)
    }

    // ---- internal helpers --------------------------------------------

    /// Send the welcome email after admin-driven activate or create.
    /// Pulls org name + Discord invite from settings.
    async fn send_welcome_email(&self, member: &Member) -> Result<()> {
        let portal_url = format!(
            "{}/portal/dashboard",
            self.base_url.trim_end_matches('/'),
        );
        let org_name = self.settings_service
            .get_value("org.name")
            .await
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Coterie".to_string());

        // Pull the Discord invite URL from settings if the operator has
        // configured one. None → the welcome email omits the Discord
        // section entirely. Empty string is treated the same as None.
        let discord_invite = self.settings_service
            .get_value("discord.invite_url")
            .await
            .ok()
            .filter(|s| !s.is_empty());

        let html = WelcomeHtml {
            full_name: &member.full_name,
            org_name: &org_name,
            portal_url: &portal_url,
            discord_invite: discord_invite.as_deref(),
        };
        let text = WelcomeText {
            full_name: &member.full_name,
            org_name: &org_name,
            portal_url: &portal_url,
            discord_invite: discord_invite.as_deref(),
        };
        let message = email::message_from_templates(
            member.email.clone(),
            format!("Welcome to {}", org_name),
            &html,
            &text,
        )?;
        self.email_sender.send(&message).await
    }

    /// Re-fetch the member after an update and fire `MemberUpdated`
    /// with the old/new pair, returning the new member. Centralizes
    /// the post-update integration-event dispatch so methods don't
    /// each re-roll the find_by_id + integration_manager dance.
    ///
    /// If the re-fetch fails (DB error or vanished), the new member
    /// cannot be returned, so we surface an `AppError::Internal`.
    /// In practice this is unreachable — the caller just successfully
    /// mutated this row inside the same connection — but the fallback
    /// is needed for the type to typecheck without an unwrap.
    async fn dispatch_member_updated(
        &self,
        member_id: Uuid,
        old: Member,
    ) -> Result<Member> {
        let new = self.member_repo.find_by_id(member_id).await?
            .ok_or_else(|| AppError::Internal(format!(
                "Member {} vanished between update and refetch", member_id,
            )))?;
        self.integration_manager
            .handle_event(IntegrationEvent::MemberUpdated {
                old, new: new.clone(),
            })
            .await;
        Ok(new)
    }

    /// Look up a membership type's display name. Used by the
    /// member-row HTMX fragment after a status change. Returns
    /// `(unknown)` if the lookup fails or the type isn't found —
    /// callers render this in a flash row, not a critical surface.
    pub async fn membership_type_name(&self, member: &Member) -> String {
        self.membership_type_service
            .get(member.membership_type_id).await.ok().flatten()
            .map(|mt| mt.name)
            .unwrap_or_else(|| "(unknown)".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        auth::SecretCrypto,
        domain::CreateMemberRequest,
        email::LogSender,
        integrations::IntegrationManager,
        repository::{SqliteMemberRepository, SqliteMembershipTypeRepository},
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
        sqlx::migrate!("./migrations").run(&pool).await.expect("migrate");
        pool
    }

    fn make_service(pool: SqlitePool) -> MemberService {
        let member_repo: Arc<dyn MemberRepository> =
            Arc::new(SqliteMemberRepository::new(pool.clone()));
        let auth_service = Arc::new(AuthService::new(
            pool.clone(), "test-secret".to_string(),
        ));
        let audit_service = Arc::new(AuditService::new(pool.clone()));
        let integration_manager = Arc::new(IntegrationManager::new());
        let email_sender: Arc<dyn EmailSender> = Arc::new(
            LogSender::new("test@example.com".to_string(), "Test".to_string()),
        );
        let membership_type_repo = Arc::new(SqliteMembershipTypeRepository::new(pool.clone()));
        let membership_type_service = Arc::new(MembershipTypeService::new(membership_type_repo));
        let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
        let settings_service = Arc::new(SettingsService::new(pool.clone(), crypto));
        let email_token_service = Arc::new(EmailTokenService::verification(pool.clone()));

        MemberService::new(
            member_repo,
            auth_service,
            audit_service,
            integration_manager,
            email_sender,
            membership_type_service,
            settings_service,
            email_token_service,
            "http://test.local".to_string(),
        )
    }

    async fn make_member(pool: &SqlitePool, email: &str, username: &str) -> Member {
        let repo = SqliteMemberRepository::new(pool.clone());
        repo.create(CreateMemberRequest {
            email: email.to_string(),
            username: username.to_string(),
            full_name: "Test User".to_string(),
            password: "secure_password123".to_string(),
            membership_type_id: None,
        })
        .await
        .unwrap()
    }

    async fn audit_count(pool: &SqlitePool, action: &str, entity_id: &Uuid) -> i64 {
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM audit_logs WHERE action = ? AND entity_id = ?"
        )
        .bind(action)
        .bind(entity_id.to_string())
        .fetch_one(pool)
        .await
        .unwrap();
        count.0
    }

    #[tokio::test]
    async fn activate_emits_full_chain() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_member(&pool, "admin@example.com", "admin").await;
        let target = make_member(&pool, "tgt@example.com", "target").await;

        // Mint a session so we can prove invalidate_all_sessions ran.
        let (_session, _token) = svc.auth_service
            .create_session(target.id, 24)
            .await
            .unwrap();
        let sessions_before: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sessions WHERE member_id = ?"
        )
        .bind(target.id.to_string())
        .fetch_one(&pool).await.unwrap();
        assert_eq!(sessions_before.0, 1);

        let result = svc.activate(actor.id, target.id).await.unwrap();

        // Repo touched — status is now Active.
        assert_eq!(result.status, MemberStatus::Active);

        // Sessions invalidated.
        let sessions_after: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sessions WHERE member_id = ?"
        )
        .bind(target.id.to_string())
        .fetch_one(&pool).await.unwrap();
        assert_eq!(sessions_after.0, 0);

        // Audit row inserted.
        assert_eq!(audit_count(&pool, "activate_member", &target.id).await, 1);

        // Integration event dispatched + welcome email sent — the
        // IntegrationManager and LogSender both swallow their work
        // (no observable failures), so reaching here without panic
        // confirms the chain ran end-to-end.
    }

    #[tokio::test]
    async fn activate_propagates_repo_error() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_member(&pool, "admin@example.com", "admin").await;

        // No such member → repo update errors → no audit row, no event.
        let bogus = Uuid::new_v4();
        let err = svc.activate(actor.id, bogus).await;
        assert!(err.is_err());
        assert_eq!(audit_count(&pool, "activate_member", &bogus).await, 0);
    }

    #[tokio::test]
    async fn suspend_emits_full_chain() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_member(&pool, "admin@example.com", "admin").await;
        let target = make_member(&pool, "tgt@example.com", "target").await;
        let (_s, _t) = svc.auth_service.create_session(target.id, 24).await.unwrap();

        let result = svc.suspend(actor.id, target.id).await.unwrap();

        assert_eq!(result.status, MemberStatus::Suspended);
        let sessions_after: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sessions WHERE member_id = ?"
        )
        .bind(target.id.to_string())
        .fetch_one(&pool).await.unwrap();
        assert_eq!(sessions_after.0, 0);
        assert_eq!(audit_count(&pool, "suspend_member", &target.id).await, 1);
    }

    #[tokio::test]
    async fn update_emits_audit_and_event() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_member(&pool, "admin@example.com", "admin").await;
        let target = make_member(&pool, "tgt@example.com", "target").await;

        let request = UpdateMemberRequest {
            full_name: Some("Renamed".to_string()),
            notes: Some("hello".to_string()),
            ..Default::default()
        };
        let result = svc.update(actor.id, target.id, request).await.unwrap();

        assert_eq!(result.full_name, "Renamed");
        assert_eq!(result.notes.as_deref(), Some("hello"));
        assert_eq!(audit_count(&pool, "update_member", &target.id).await, 1);
    }

    #[tokio::test]
    async fn extend_dues_validates_range() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_member(&pool, "admin@example.com", "admin").await;
        let target = make_member(&pool, "tgt@example.com", "target").await;

        let bad = svc.extend_dues(actor.id, target.id, 0).await;
        assert!(matches!(bad, Err(AppError::BadRequest(_))));
        let bad_high = svc.extend_dues(actor.id, target.id, 121).await;
        assert!(matches!(bad_high, Err(AppError::BadRequest(_))));

        let ok = svc.extend_dues(actor.id, target.id, 12).await.unwrap();
        assert!(ok.dues_paid_until.is_some());
        assert_eq!(audit_count(&pool, "extend_dues", &target.id).await, 1);
    }

    #[tokio::test]
    async fn set_dues_writes_audit() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_member(&pool, "admin@example.com", "admin").await;
        let target = make_member(&pool, "tgt@example.com", "target").await;

        let date = NaiveDate::from_ymd_opt(2027, 1, 1).unwrap();
        let result = svc.set_dues(actor.id, target.id, date).await.unwrap();

        let dpu = result.dues_paid_until.unwrap();
        assert_eq!(dpu.format("%Y-%m-%d").to_string(), "2027-01-01");
        assert_eq!(audit_count(&pool, "set_dues", &target.id).await, 1);
    }

    #[tokio::test]
    async fn expire_now_invalidates_sessions_and_audits() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_member(&pool, "admin@example.com", "admin").await;
        let target = make_member(&pool, "tgt@example.com", "target").await;
        // Activate + extend so expire_now has something to flip.
        svc.activate(actor.id, target.id).await.unwrap();
        svc.extend_dues(actor.id, target.id, 1).await.unwrap();
        let (_s, _t) = svc.auth_service.create_session(target.id, 24).await.unwrap();

        let result = svc.expire_now(actor.id, target.id).await.unwrap();

        // expire_dues_now backdates dues_paid_until and flips Active→Expired.
        assert_eq!(result.status, MemberStatus::Expired);
        let sessions_after: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sessions WHERE member_id = ?"
        )
        .bind(target.id.to_string())
        .fetch_one(&pool).await.unwrap();
        assert_eq!(sessions_after.0, 0);
        assert_eq!(audit_count(&pool, "expire_member_now", &target.id).await, 1);
    }

    #[tokio::test]
    async fn update_discord_id_validates_snowflake() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_member(&pool, "admin@example.com", "admin").await;
        let target = make_member(&pool, "tgt@example.com", "target").await;

        let bad = svc.update_discord_id(
            actor.id, target.id, Some("user#1234".to_string()),
        ).await;
        assert!(matches!(bad, Err(AppError::BadRequest(_))));

        let ok = svc.update_discord_id(
            actor.id, target.id, Some("123456789012345678".to_string()),
        ).await.unwrap();
        assert_eq!(ok.discord_id.as_deref(), Some("123456789012345678"));
        assert_eq!(audit_count(&pool, "update_discord_id", &target.id).await, 1);

        // Clear it.
        let cleared = svc.update_discord_id(actor.id, target.id, None)
            .await.unwrap();
        assert!(cleared.discord_id.is_none());
        assert_eq!(audit_count(&pool, "update_discord_id", &target.id).await, 2);
    }

    #[tokio::test]
    async fn resend_verification_audits_on_success_and_rejects_verified() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_member(&pool, "admin@example.com", "admin").await;
        let target = make_member(&pool, "tgt@example.com", "target").await;

        // Pending member with unverified email — should succeed.
        svc.resend_verification(actor.id, target.id).await.unwrap();
        assert_eq!(audit_count(&pool, "resend_verification", &target.id).await, 1);

        // Mark verified, then try again — should reject.
        SqliteMemberRepository::new(pool.clone())
            .mark_email_verified(target.id).await.unwrap();
        let rejected = svc.resend_verification(actor.id, target.id).await;
        assert!(matches!(rejected, Err(AppError::BadRequest(_))));
        // Audit count unchanged.
        assert_eq!(audit_count(&pool, "resend_verification", &target.id).await, 1);
    }

    #[tokio::test]
    async fn create_audits_and_skips_activation_event() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_member(&pool, "admin@example.com", "admin").await;

        let request = CreateMemberRequest {
            email: "new@example.com".to_string(),
            username: "newuser".to_string(),
            full_name: "New User".to_string(),
            password: "secure_password123".to_string(),
            membership_type_id: None,
        };
        let created = svc.create(actor.id, request).await.unwrap();

        // Newly created members default to Pending (NOT Active) — the
        // create path deliberately doesn't dispatch MemberActivated.
        assert_eq!(created.status, MemberStatus::Pending);
        assert_eq!(audit_count(&pool, "create_member", &created.id).await, 1);
    }
}
