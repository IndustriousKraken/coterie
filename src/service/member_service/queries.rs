//! Small read-only / audit-only helpers: `audit_export` records an
//! aggregate audit row for a roster CSV export; `membership_type_name`
//! resolves a member's type to its display name for HTMX fragment
//! rendering after a status change.

use uuid::Uuid;

use crate::{domain::Member, error::Result};

use super::MemberService;

impl MemberService {
    /// Audit an admin's CSV export of the member roster. The handler
    /// has already pulled the rows and assembled the response — this
    /// method only emits the `export_members` audit entry so abuse is
    /// traceable. `entity_id = "*"` marks the action as aggregate
    /// rather than tied to a single member row; `new_value` carries
    /// the filter summary (e.g., `status=Active`) plus the row count.
    pub async fn audit_export(
        &self,
        actor_id: Uuid,
        filter_summary: &str,
        row_count: usize,
    ) -> Result<()> {
        let new_value = if filter_summary.is_empty() {
            format!("count={}", row_count)
        } else {
            format!("{},count={}", filter_summary, row_count)
        };
        self.audit_service
            .log(
                Some(actor_id),
                "export_members",
                "member",
                "*",
                None,
                Some(&new_value),
                None,
            )
            .await;
        Ok(())
    }

    /// Look up a membership type's display name. Used by the
    /// member-row HTMX fragment after a status change. Returns
    /// `(unknown)` if the lookup fails or the type isn't found —
    /// callers render this in a flash row, not a critical surface.
    pub async fn membership_type_name(&self, member: &Member) -> String {
        self.membership_type_service
            .get(member.membership_type_id)
            .await
            .ok()
            .flatten()
            .map(|mt| mt.name)
            .unwrap_or_else(|| "(unknown)".to_string())
    }
}
