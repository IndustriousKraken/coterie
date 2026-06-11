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
