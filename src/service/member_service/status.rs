//! Member status transitions: `activate`, `suspend`, `expire_now`.
//! Each method handles the full side-effect chain (repo update →
//! session invalidation → audit → integration dispatch → email).

use uuid::Uuid;

use crate::{
    domain::{Member, MemberStatus, UpdateMemberRequest},
    error::Result,
    integrations::IntegrationEvent,
};

use super::MemberService;

impl MemberService {
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
                member.id,
                e,
            );
        }

        self.audit_service
            .log(
                Some(actor_id),
                "activate_member",
                "member",
                &member_id.to_string(),
                None,
                Some(&member.email),
                None,
            )
            .await;

        // Notify integrations (Discord role sync, future Unifi
        // access provisioning, etc.). Fire-and-forget — individual
        // failures are logged inside each impl.
        self.integration_manager
            .handle_event(IntegrationEvent::MemberActivated(member.clone()))
            .await;

        if let Err(e) = self.send_welcome_email(&member).await {
            tracing::error!(
                "Member {} activated but welcome email failed: {}",
                member.id,
                e,
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
                member.id,
                e,
            );
        }

        if let Some(old) = old_member {
            self.integration_manager
                .handle_event(IntegrationEvent::MemberUpdated {
                    old,
                    new: member.clone(),
                })
                .await;
        }

        self.audit_service
            .log(
                Some(actor_id),
                "suspend_member",
                "member",
                &member_id.to_string(),
                None,
                Some(&member.email),
                None,
            )
            .await;

        Ok(member)
    }

    /// Backdate `dues_paid_until` to yesterday and flip Active→Expired,
    /// invalidate sessions so the member sees the change immediately,
    /// audit, and dispatch `MemberUpdated`. Session-invalidation
    /// failure is logged but not fatal — middleware re-validates per
    /// request and bounces them anyway.
    pub async fn expire_now(&self, actor_id: Uuid, member_id: Uuid) -> Result<Member> {
        use crate::error::AppError;

        let old_member = self
            .member_repo
            .find_by_id(member_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Member not found".to_string()))?;

        self.member_repo.expire_dues_now(member_id).await?;

        if let Err(e) = self.auth_service.invalidate_all_sessions(member_id).await {
            tracing::error!(
                "Expired dues for member {} but failed to invalidate sessions: {}",
                member_id,
                e,
            );
        }

        self.audit_service
            .log(
                Some(actor_id),
                "expire_member_now",
                "member",
                &member_id.to_string(),
                None,
                None,
                None,
            )
            .await;

        self.dispatch_member_updated(member_id, old_member).await
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::*;
    use crate::domain::MemberStatus;
    use uuid::Uuid;

    #[tokio::test]
    async fn activate_emits_full_chain() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_member(&pool, "admin@example.com", "admin").await;
        let target = make_member(&pool, "tgt@example.com", "target").await;

        // Mint a session so we can prove invalidate_all_sessions ran.
        let (_session, _token) = svc
            .auth_service
            .create_session(target.id, 24)
            .await
            .unwrap();
        let sessions_before: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM sessions WHERE member_id = ?")
                .bind(target.id.to_string())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(sessions_before.0, 1);

        let result = svc.activate(actor.id, target.id).await.unwrap();

        // Repo touched — status is now Active.
        assert_eq!(result.status, MemberStatus::Active);

        // Sessions invalidated.
        let sessions_after: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM sessions WHERE member_id = ?")
                .bind(target.id.to_string())
                .fetch_one(&pool)
                .await
                .unwrap();
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
        let (_s, _t) = svc
            .auth_service
            .create_session(target.id, 24)
            .await
            .unwrap();

        let result = svc.suspend(actor.id, target.id).await.unwrap();

        assert_eq!(result.status, MemberStatus::Suspended);
        let sessions_after: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM sessions WHERE member_id = ?")
                .bind(target.id.to_string())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(sessions_after.0, 0);
        assert_eq!(audit_count(&pool, "suspend_member", &target.id).await, 1);
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
        let (_s, _t) = svc
            .auth_service
            .create_session(target.id, 24)
            .await
            .unwrap();

        let result = svc.expire_now(actor.id, target.id).await.unwrap();

        // expire_dues_now backdates dues_paid_until and flips Active→Expired.
        assert_eq!(result.status, MemberStatus::Expired);
        let sessions_after: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM sessions WHERE member_id = ?")
                .bind(target.id.to_string())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(sessions_after.0, 0);
        assert_eq!(audit_count(&pool, "expire_member_now", &target.id).await, 1);
    }
}
