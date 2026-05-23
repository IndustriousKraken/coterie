//! Shared integration-event dispatch helper. Re-fetches the member
//! after a mutation and fires `MemberUpdated { old, new }` to
//! integrations, returning the new member. Called from `dues.rs`,
//! `updates.rs`, and `status.rs` (the `expire_now` path).

use uuid::Uuid;

use crate::{
    domain::Member,
    error::{AppError, Result},
    integrations::IntegrationEvent,
};

use super::MemberService;

impl MemberService {
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
    pub(super) async fn dispatch_member_updated(
        &self,
        member_id: Uuid,
        old: Member,
    ) -> Result<Member> {
        let new = self
            .member_repo
            .find_by_id(member_id)
            .await?
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "Member {} vanished between update and refetch",
                    member_id,
                ))
            })?;
        self.integration_manager
            .handle_event(IntegrationEvent::MemberUpdated {
                old,
                new: new.clone(),
            })
            .await;
        Ok(new)
    }
}
