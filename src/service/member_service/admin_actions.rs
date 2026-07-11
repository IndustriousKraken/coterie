//! Admin quick actions from the member detail page: send a password
//! reset on the member's behalf, and delete a member without financial
//! history. See the `admin-member-quick-actions` change / the
//! `admin-members` capability spec.

use uuid::Uuid;

use crate::{
    auth,
    email::{
        self,
        templates::{ResetHtml, ResetText},
    },
    error::{AppError, Result},
};

use super::MemberService;

impl MemberService {
    /// Issue the member the same single-use, one-hour reset token +
    /// email the self-service forgot-password flow issues. Does not
    /// touch the member's password, status, or sessions. Audited.
    pub async fn send_password_reset(&self, actor_id: Uuid, member_id: Uuid) -> Result<()> {
        let member = self
            .member_repo
            .find_by_id(member_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Member not found".to_string()))?;

        let created = auth::email_tokens::create_password_reset_token(
            &self.db_pool,
            member.id,
            chrono::Duration::hours(1),
        )
        .await?;

        let reset_url = format!(
            "{}/reset-password?token={}",
            self.base_url.trim_end_matches('/'),
            created.token,
        );
        let org_name = self
            .settings_service
            .get_value("org.name")
            .await
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Coterie".to_string());

        let html = ResetHtml {
            full_name: &member.full_name,
            org_name: &org_name,
            reset_url: &reset_url,
        };
        let text = ResetText {
            full_name: &member.full_name,
            org_name: &org_name,
            reset_url: &reset_url,
        };
        let message = email::message_from_templates(
            member.email.clone(),
            format!("Reset your {} password", org_name),
            &html,
            &text,
        )?;
        self.email_sender.send(&message).await?;

        self.audit_service
            .log(
                Some(actor_id),
                "send_password_reset",
                "member",
                &member_id.to_string(),
                None,
                Some(&member.email),
                None,
            )
            .await;

        Ok(())
    }

    /// Guarded hard delete for members WITHOUT financial history —
    /// cleanup for test/spam/typo signups. Payments are a ledger:
    /// members with any payment row are rejected toward suspend/expire.
    /// Self-deletion and deleting the last administrator are rejected
    /// (the latter would lock every operator out AND re-arm the
    /// unauthenticated /setup page on restart).
    pub async fn delete(&self, actor_id: Uuid, member_id: Uuid) -> Result<()> {
        let member = self
            .member_repo
            .find_by_id(member_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Member not found".to_string()))?;

        if member_id == actor_id {
            return Err(AppError::BadRequest(
                "You cannot delete your own account.".to_string(),
            ));
        }
        if member.is_admin && self.member_repo.count_admins().await? <= 1 {
            return Err(AppError::BadRequest(
                "Cannot delete the last administrator. Grant another member admin \
                 access first."
                    .to_string(),
            ));
        }

        let payment_rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM payments WHERE member_id = ?")
                .bind(member_id.to_string())
                .fetch_one(&self.db_pool)
                .await
                .map_err(AppError::Database)?;
        if payment_rows > 0 {
            return Err(AppError::BadRequest(
                "This member has payment history, which is never deleted. Suspend or \
                 expire the membership instead."
                    .to_string(),
            ));
        }

        // Profile, sessions, saved cards, scheduled payments, tokens and
        // attendance cascade. Anything else still referencing the member
        // (authored events/announcements, audit entries as actor) makes
        // the DELETE fail its FK checks — map that to guidance instead
        // of a 500.
        match self.member_repo.delete(member_id).await {
            Ok(()) => {}
            Err(AppError::Database(e))
                if e.to_string().to_lowercase().contains("foreign key") =>
            {
                return Err(AppError::BadRequest(
                    "This member is referenced by other records (events, announcements \
                     or audit history) and cannot be deleted. Suspend or expire the \
                     membership instead."
                        .to_string(),
                ));
            }
            Err(e) => return Err(e),
        }

        self.audit_service
            .log(
                Some(actor_id),
                "delete_member",
                "member",
                &member_id.to_string(),
                Some(&format!(
                    "{} <{}> ({})",
                    member.username, member.email, member.full_name
                )),
                None,
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
        error::AppError,
        repository::{MemberRepository, SqliteMemberRepository},
    };

    #[tokio::test]
    async fn send_password_reset_creates_token_and_audits() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_member(&pool, "admin@example.com", "admin").await;
        let target = make_member(&pool, "tgt@example.com", "target").await;

        svc.send_password_reset(actor.id, target.id).await.unwrap();

        let tokens: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM password_reset_tokens WHERE member_id = ?")
                .bind(target.id.to_string())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(tokens, 1, "reset token created");
        assert_eq!(
            audit_count(&pool, "send_password_reset", &target.id).await,
            1
        );
    }

    #[tokio::test]
    async fn delete_removes_paymentless_member_and_audits() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_member(&pool, "admin@example.com", "admin").await;
        let target = make_member(&pool, "tgt@example.com", "target").await;
        let target_id = target.id;

        svc.delete(actor.id, target_id).await.unwrap();

        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM members WHERE id = ?")
            .bind(target_id.to_string())
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(remaining, 0, "member row removed");
        assert_eq!(audit_count(&pool, "delete_member", &target_id).await, 1);
    }

    #[tokio::test]
    async fn delete_rejects_member_with_payment_history() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_member(&pool, "admin@example.com", "admin").await;
        let target = make_member(&pool, "tgt@example.com", "target").await;

        sqlx::query(
            "INSERT INTO payments (id, member_id, amount_cents, currency, status, \
             payment_method, description, payment_type, created_at, updated_at) \
             VALUES (?, ?, 4500, 'USD', 'Completed', 'Stripe', 'dues', 'membership', \
             datetime('now'), datetime('now'))",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(target.id.to_string())
        .execute(&pool)
        .await
        .unwrap();

        let err = svc.delete(actor.id, target.id).await;
        assert!(matches!(err, Err(AppError::BadRequest(_))));
        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM members WHERE id = ?")
            .bind(target.id.to_string())
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(remaining, 1, "member with ledger rows survives");
    }

    #[tokio::test]
    async fn delete_rejects_self_and_last_admin() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let repo = SqliteMemberRepository::new(pool.clone());
        let actor = make_member(&pool, "admin@example.com", "admin").await;
        let only_admin = make_member(&pool, "boss@example.com", "boss").await;
        repo.set_admin(only_admin.id, true).await.unwrap();

        let self_err = svc.delete(actor.id, actor.id).await;
        assert!(matches!(self_err, Err(AppError::BadRequest(_))));

        let last_admin_err = svc.delete(actor.id, only_admin.id).await;
        assert!(matches!(last_admin_err, Err(AppError::BadRequest(_))));
        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM members WHERE id = ?")
            .bind(only_admin.id.to_string())
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(remaining, 1, "last admin survives");
    }
}
