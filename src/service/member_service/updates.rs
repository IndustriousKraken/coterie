//! Profile-field updates: `update` (generic), `update_discord_id`,
//! `resend_verification`. The first two audit and dispatch
//! `MemberUpdated`; `resend_verification` audits only on a successful
//! email send and rejects already-verified members.

use uuid::Uuid;

use crate::{
    auth,
    domain::{Member, UpdateMemberRequest},
    email::{
        self,
        templates::{VerifyHtml, VerifyText},
    },
    error::{AppError, Result},
    integrations::IntegrationEvent,
};

use super::MemberService;

impl MemberService {
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

        // Admin flag: handled here (not in the generic repo update) so
        // the guard and its dedicated audit trail can't be bypassed.
        // Runs BEFORE the generic update so the MemberUpdated event
        // payload below carries the new flag.
        if let (Some(want_admin), Some(old)) = (request.is_admin, old_member.as_ref()) {
            if old.is_admin != want_admin {
                if !want_admin && self.member_repo.count_admins().await? <= 1 {
                    // Zero admins = every operator locked out AND the
                    // unauthenticated /setup page re-arms on restart.
                    return Err(AppError::BadRequest(
                        "Cannot revoke the last administrator. Grant another member \
                         admin access first."
                            .to_string(),
                    ));
                }
                self.member_repo.set_admin(member_id, want_admin).await?;
                self.audit_service
                    .log(
                        Some(actor_id),
                        if want_admin {
                            "grant_admin"
                        } else {
                            "revoke_admin"
                        },
                        "member",
                        &member_id.to_string(),
                        Some(if want_admin { "false" } else { "true" }),
                        Some(if want_admin { "true" } else { "false" }),
                        None,
                    )
                    .await;
            }
        }

        let new_member = self.member_repo.update(member_id, request).await?;

        self.audit_service
            .log(
                Some(actor_id),
                "update_member",
                "member",
                &member_id.to_string(),
                None,
                None,
                None,
            )
            .await;

        if let Some(old) = old_member {
            self.integration_manager
                .handle_event(IntegrationEvent::MemberUpdated {
                    old,
                    new: new_member.clone(),
                })
                .await;
        }

        Ok(new_member)
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

        let trimmed = discord_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if let Some(s) = trimmed {
            if !is_valid_snowflake(s) {
                return Err(AppError::BadRequest(
                    "Discord ID must be 17–20 digits (snowflake format). \
                     Right-click the user in Discord with Developer Mode on → Copy User ID."
                        .to_string(),
                ));
            }
        }

        let old_member = self
            .member_repo
            .find_by_id(member_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Member not found".to_string()))?;

        self.member_repo
            .update_discord_id(member_id, trimmed)
            .await?;

        self.audit_service
            .log(
                Some(actor_id),
                "update_discord_id",
                "member",
                &member_id.to_string(),
                old_member.discord_id.as_deref(),
                trimmed,
                None,
            )
            .await;

        self.dispatch_member_updated(member_id, old_member).await
    }

    /// Regenerate a verification token for an unverified member and
    /// email them the fresh link. Invalidates any previously
    /// outstanding tokens so an old email can't be used. Already-
    /// verified members are rejected.
    pub async fn resend_verification(&self, actor_id: Uuid, member_id: Uuid) -> Result<()> {
        let member = self
            .member_repo
            .find_by_id(member_id)
            .await?
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
        if let Err(e) =
            auth::email_tokens::invalidate_verification_tokens_for_member(&self.db_pool, member_id)
                .await
        {
            tracing::warn!(
                "Resending verification for {} but couldn't invalidate previous tokens: {}",
                member_id,
                e,
            );
        }

        let created = auth::email_tokens::create_verification_token(
            &self.db_pool,
            member_id,
            chrono::Duration::hours(24),
        )
        .await?;

        let org_name = self
            .settings_service
            .get_value("org.name")
            .await
            .ok()
            .filter(|s| !s.is_empty())
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

        self.audit_service
            .log(
                Some(actor_id),
                "resend_verification",
                "member",
                &member_id.to_string(),
                None,
                Some(&member.email),
                None,
            )
            .await;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::*;
    use crate::{
        domain::UpdateMemberRequest,
        error::AppError,
        repository::{MemberRepository, SqliteMemberRepository},
    };

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
    async fn grant_admin_via_update_sets_flag_and_audits() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_member(&pool, "admin@example.com", "admin").await;
        let target = make_member(&pool, "tgt@example.com", "target").await;

        let result = svc
            .update(
                actor.id,
                target.id,
                UpdateMemberRequest {
                    full_name: Some(target.full_name.clone()),
                    is_admin: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(result.is_admin, "flag must be set");
        assert_eq!(audit_count(&pool, "grant_admin", &target.id).await, 1);
        assert_eq!(audit_count(&pool, "revoke_admin", &target.id).await, 0);
    }

    #[tokio::test]
    async fn revoke_admin_guards_the_last_administrator() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let repo = SqliteMemberRepository::new(pool.clone());
        let actor = make_member(&pool, "admin@example.com", "admin").await;
        let second = make_member(&pool, "two@example.com", "two").await;
        repo.set_admin(actor.id, true).await.unwrap();

        // actor is the ONLY admin: revoking them must be rejected.
        let err = svc
            .update(
                actor.id,
                actor.id,
                UpdateMemberRequest {
                    is_admin: Some(false),
                    ..Default::default()
                },
            )
            .await;
        assert!(matches!(err, Err(AppError::BadRequest(_))));
        let still = repo.find_by_id(actor.id).await.unwrap().unwrap();
        assert!(still.is_admin, "last admin must keep the flag");

        // With a second admin, revoking succeeds and audits.
        repo.set_admin(second.id, true).await.unwrap();
        let revoked = svc
            .update(
                actor.id,
                second.id,
                UpdateMemberRequest {
                    is_admin: Some(false),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(!revoked.is_admin);
        assert_eq!(audit_count(&pool, "revoke_admin", &second.id).await, 1);
    }

    #[tokio::test]
    async fn notes_text_never_grants_admin() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_member(&pool, "admin@example.com", "admin").await;
        let target = make_member(&pool, "tgt@example.com", "target").await;

        // The historical hint claimed putting "ADMIN" in notes grants
        // privileges. It never did, and must never start to.
        let result = svc
            .update(
                actor.id,
                target.id,
                UpdateMemberRequest {
                    notes: Some("ADMIN".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(!result.is_admin, "notes text must not affect adminness");
        assert_eq!(audit_count(&pool, "grant_admin", &target.id).await, 0);
    }

    #[tokio::test]
    async fn unchanged_admin_flag_writes_no_admin_audit() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let repo = SqliteMemberRepository::new(pool.clone());
        let actor = make_member(&pool, "admin@example.com", "admin").await;
        let target = make_member(&pool, "tgt@example.com", "target").await;
        repo.set_admin(target.id, true).await.unwrap();

        svc.update(
            actor.id,
            target.id,
            UpdateMemberRequest {
                is_admin: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(
            audit_count(&pool, "grant_admin", &target.id).await,
            0,
            "no-op flag must not audit"
        );
    }

    #[tokio::test]
    async fn update_discord_id_validates_snowflake() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_member(&pool, "admin@example.com", "admin").await;
        let target = make_member(&pool, "tgt@example.com", "target").await;

        let bad = svc
            .update_discord_id(actor.id, target.id, Some("user#1234".to_string()))
            .await;
        assert!(matches!(bad, Err(AppError::BadRequest(_))));

        let ok = svc
            .update_discord_id(actor.id, target.id, Some("123456789012345678".to_string()))
            .await
            .unwrap();
        assert_eq!(ok.discord_id.as_deref(), Some("123456789012345678"));
        assert_eq!(audit_count(&pool, "update_discord_id", &target.id).await, 1);

        // Clear it.
        let cleared = svc
            .update_discord_id(actor.id, target.id, None)
            .await
            .unwrap();
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
        assert_eq!(
            audit_count(&pool, "resend_verification", &target.id).await,
            1
        );

        // Mark verified, then try again — should reject.
        SqliteMemberRepository::new(pool.clone())
            .mark_email_verified(target.id)
            .await
            .unwrap();
        let rejected = svc.resend_verification(actor.id, target.id).await;
        assert!(matches!(rejected, Err(AppError::BadRequest(_))));
        // Audit count unchanged.
        assert_eq!(
            audit_count(&pool, "resend_verification", &target.id).await,
            1
        );
    }
}
