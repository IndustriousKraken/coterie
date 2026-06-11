//! Single-member creation and the shared `send_welcome_email` helper.
//! `bulk_import` is large enough to live in its own sibling module
//! ([`super::bulk_import`]); `send_welcome_email` is `pub(super)` so
//! both this module and [`super::status::activate`] can reach it.

use uuid::Uuid;

use crate::{
    domain::{CreateMemberRequest, Member},
    email::{
        self,
        templates::{WelcomeHtml, WelcomeText},
    },
    error::Result,
};

use super::MemberService;

impl MemberService {
    /// Create a new member via the admin form. Persists the row,
    /// sends the welcome email (log+swallow on failure), audits.
    /// Does NOT dispatch `MemberActivated` — newly-created members
    /// start `Pending` by repo default; the activation event fires
    /// on the later `activate` call.
    pub async fn create(&self, actor_id: Uuid, request: CreateMemberRequest) -> Result<Member> {
        let member = self.member_repo.create(request).await?;

        if let Err(e) = self.send_welcome_email(&member).await {
            tracing::error!(
                "Created member {} but welcome email failed: {}",
                member.id,
                e,
            );
        }

        self.audit_service
            .log(
                Some(actor_id),
                "create_member",
                "member",
                &member.id.to_string(),
                None,
                Some(&member.email),
                None,
            )
            .await;

        Ok(member)
    }

    /// Send the welcome email after admin-driven activate or create.
    /// Pulls org name + Discord invite from settings.
    pub(super) async fn send_welcome_email(&self, member: &Member) -> Result<()> {
        let portal_url = format!("{}/portal/dashboard", self.base_url.trim_end_matches('/'),);
        let org_name = self
            .settings_service
            .get_value("org.name")
            .await
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Coterie".to_string());

        // Pull the Discord invite URL from settings if the operator has
        // configured one. None → the welcome email omits the Discord
        // section entirely. Empty string is treated the same as None.
        let discord_invite = self
            .settings_service
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
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::*;
    use crate::domain::{CreateMemberRequest, MemberStatus};

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
            ..Default::default()
        };
        let created = svc.create(actor.id, request).await.unwrap();

        // Newly created members default to Pending (NOT Active) — the
        // create path deliberately doesn't dispatch MemberActivated.
        assert_eq!(created.status, MemberStatus::Pending);
        assert_eq!(audit_count(&pool, "create_member", &created.id).await, 1);
    }
}
