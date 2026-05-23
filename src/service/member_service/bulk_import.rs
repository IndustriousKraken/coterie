//! Bulk member import from pre-parsed CSV rows. Extracted into its
//! own file because the single method weighs ~315 lines on its own.
//! Per-row failures don't abort the batch; the summary carries the
//! full failure list with 1-based row indexes.

use uuid::Uuid;

use crate::{
    domain::{BillingMode, CreateMemberRequest, MemberStatus, UpdateMemberRequest},
    error::{AppError, Result},
};

use super::{BulkImportSummary, ImportFailure, ImportRow, MemberService};

impl MemberService {
    /// Import a batch of pre-parsed rows. Per-row validation failures
    /// (bad email, duplicate, unknown membership type) are accumulated
    /// into the summary's `failures` list; one bad row does not abort
    /// the batch. Each successful row creates a member and emits an
    /// `import_member` audit row; the batch emits one final
    /// `import_members_batch` aggregate audit row carrying the
    /// success/failure counts.
    ///
    /// Imported members never get a real password — `create` synthesizes
    /// one from a random secret here so the row is unusable for login
    /// until the member completes a password reset. This matches the
    /// `bulk-member-csv-import` spec: the operator activates members
    /// later, and the password-reset flow handles credentialing.
    pub async fn bulk_import(
        &self,
        actor_id: Uuid,
        file_name: &str,
        rows: Vec<ImportRow>,
    ) -> Result<BulkImportSummary> {
        use rand::RngCore;

        let mut summary = BulkImportSummary {
            succeeded: 0,
            failed: 0,
            failures: Vec::new(),
            created_member_ids: Vec::new(),
        };

        // Pre-load active membership types once so each row's slug
        // lookup is an in-memory hash hit, not a DB round trip. Inactive
        // types fail the row (they shouldn't be assignable on import any
        // more than they are on the manual-create form).
        let active_types = self
            .membership_type_service
            .list(false)
            .await
            .unwrap_or_default();

        for (idx, row) in rows.into_iter().enumerate() {
            let row_index = idx + 1;
            let email = row.email.trim().to_string();
            let username = row.username.trim().to_string();
            let full_name = row.full_name.trim().to_string();
            let slug = row.membership_type_slug.trim().to_string();

            // Parser-surfaced per-row failure (e.g., a malformed
            // timestamp cell). Fail the row first so the parse error
            // reaches the operator instead of a downstream "invalid
            // email" or similar masking message.
            if let Some(reason) = &row.parse_error {
                summary.failed += 1;
                summary.failures.push(ImportFailure {
                    row_index,
                    email: Some(email.clone()).filter(|s| !s.is_empty()),
                    reason: reason.clone(),
                });
                continue;
            }

            // Row-level validation. Each branch records a failure and
            // continues to the next row.
            if email.is_empty() || !email.contains('@') {
                summary.failed += 1;
                summary.failures.push(ImportFailure {
                    row_index,
                    email: Some(email.clone()).filter(|s| !s.is_empty()),
                    reason: "Invalid email format".to_string(),
                });
                continue;
            }
            if username.is_empty() {
                summary.failed += 1;
                summary.failures.push(ImportFailure {
                    row_index,
                    email: Some(email.clone()),
                    reason: "Username is required".to_string(),
                });
                continue;
            }
            if full_name.is_empty() {
                summary.failed += 1;
                summary.failures.push(ImportFailure {
                    row_index,
                    email: Some(email.clone()),
                    reason: "Full name is required".to_string(),
                });
                continue;
            }

            let membership_type = match active_types.iter().find(|t| t.slug == slug && t.is_active)
            {
                Some(mt) => mt,
                None => {
                    summary.failed += 1;
                    summary.failures.push(ImportFailure {
                        row_index,
                        email: Some(email.clone()),
                        reason: format!("Unknown membership_type_slug: '{}'", slug),
                    });
                    continue;
                }
            };

            // Billing-migration inconsistency: a Stripe subscription
            // always has a customer, so a row carrying a sub_id but no
            // customer_id is malformed. Catch this BEFORE create so no
            // member row is written for the bad row.
            if row
                .stripe_subscription_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_some()
                && row
                    .stripe_customer_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .is_none()
            {
                summary.failed += 1;
                summary.failures.push(ImportFailure {
                    row_index,
                    email: Some(email.clone()),
                    reason: "Stripe subscription_id present without customer_id".to_string(),
                });
                continue;
            }

            // Duplicate detection — INSERT-only semantics, no upsert.
            // We process rows sequentially so a prior row's insert is
            // already visible to this check.
            match self.member_repo.find_by_email(&email).await {
                Ok(Some(_)) => {
                    summary.failed += 1;
                    summary.failures.push(ImportFailure {
                        row_index,
                        email: Some(email.clone()),
                        reason: "Email already exists".to_string(),
                    });
                    continue;
                }
                Ok(None) => {}
                Err(e) => {
                    summary.failed += 1;
                    summary.failures.push(ImportFailure {
                        row_index,
                        email: Some(email.clone()),
                        reason: format!("Database error: {}", e),
                    });
                    continue;
                }
            }
            match self.member_repo.find_by_username(&username).await {
                Ok(Some(_)) => {
                    summary.failed += 1;
                    summary.failures.push(ImportFailure {
                        row_index,
                        email: Some(email.clone()),
                        reason: "Username already exists".to_string(),
                    });
                    continue;
                }
                Ok(None) => {}
                Err(e) => {
                    summary.failed += 1;
                    summary.failures.push(ImportFailure {
                        row_index,
                        email: Some(email.clone()),
                        reason: format!("Database error: {}", e),
                    });
                    continue;
                }
            }

            // Random sentinel password — unusable for login. Members
            // claim their account through password-reset (the existing
            // forgot-password flow accepts any registered email).
            let mut bytes = [0u8; 24];
            rand::thread_rng().fill_bytes(&mut bytes);
            let sentinel_password = format!("import-no-password-{}", hex::encode(bytes),);

            // Normalize the optional billing-migration strings: trim
            // and drop empties so a blank CSV cell behaves identically
            // to an omitted column.
            let stripe_customer_id = row
                .stripe_customer_id
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let stripe_subscription_id = row
                .stripe_subscription_id
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());

            let create_request = CreateMemberRequest {
                email: email.clone(),
                username: username.clone(),
                full_name: full_name.clone(),
                password: sentinel_password,
                membership_type_id: Some(membership_type.id),
                dues_paid_until: row.dues_paid_until,
                stripe_customer_id: stripe_customer_id.clone(),
                stripe_subscription_id: stripe_subscription_id.clone(),
                joined_at: row.joined_at,
                email_verified_at: row.email_verified_at,
            };

            let member = match self.member_repo.create(create_request).await {
                Ok(m) => m,
                Err(e) => {
                    // Likely a UNIQUE constraint violation that slipped
                    // past the pre-checks (e.g., case-insensitive
                    // collisions), or a real DB error. Either way, the
                    // row fails and the batch keeps going.
                    let reason = match &e {
                        AppError::Database(db_err) => {
                            let msg = db_err.to_string();
                            if msg.contains("UNIQUE") && msg.contains("email") {
                                "Email already exists".to_string()
                            } else if msg.contains("UNIQUE") && msg.contains("username") {
                                "Username already exists".to_string()
                            } else if msg.contains("UNIQUE") {
                                "Email or username already exists".to_string()
                            } else {
                                format!("Database error: {}", db_err)
                            }
                        }
                        other => other.to_string(),
                    };
                    summary.failed += 1;
                    summary.failures.push(ImportFailure {
                        row_index,
                        email: Some(email.clone()),
                        reason,
                    });
                    continue;
                }
            };

            // Infer `billing_mode` from sub-id presence (design D2).
            // A row carrying a Stripe subscription means Coterie is
            // observing that subscription, not the default Manual mode.
            // The inconsistency check above guarantees that if we have
            // a sub_id we also have a customer_id, so this branch is
            // only reached for a well-formed Stripe-migrated row.
            if let Some(sub_id) = stripe_subscription_id.as_deref() {
                if let Err(e) = self
                    .member_repo
                    .set_billing_mode(member.id, BillingMode::StripeSubscription, Some(sub_id))
                    .await
                {
                    tracing::error!(
                        "Bulk import: created {} but billing_mode update failed: {}",
                        member.id,
                        e,
                    );
                }
            }

            // Apply optional fields. Status default is Pending (the
            // repo's default); only call update when the row asked for
            // something different or carries notes/discord_id.
            let status_override = row.status.filter(|s| *s != MemberStatus::Pending);
            if status_override.is_some() || row.notes.is_some() {
                let update = UpdateMemberRequest {
                    status: status_override,
                    notes: row.notes.as_ref().map(|s| s.trim().to_string()),
                    ..Default::default()
                };
                if let Err(e) = self.member_repo.update(member.id, update).await {
                    // The member exists; the follow-up update failed.
                    // Log + count as success since the member row is in
                    // the DB. Audit captures the create.
                    tracing::error!(
                        "Bulk import: created {} but follow-up update failed: {}",
                        member.id,
                        e,
                    );
                }
            }
            if let Some(discord_id) = row
                .discord_id
                .as_ref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
            {
                if let Err(e) = self
                    .member_repo
                    .update_discord_id(member.id, Some(discord_id))
                    .await
                {
                    tracing::error!(
                        "Bulk import: created {} but discord_id update failed: {}",
                        member.id,
                        e,
                    );
                }
            }

            self.audit_service
                .log(
                    Some(actor_id),
                    "import_member",
                    "member",
                    &member.id.to_string(),
                    None,
                    Some(&member.email),
                    None,
                )
                .await;

            summary.succeeded += 1;
            summary.created_member_ids.push(member.id);
        }

        // Aggregate batch row, regardless of partial failures. Matches
        // the `audit-logging` capability's aggregate-entity convention
        // (entity_id = "*" for cross-entity batch operations).
        let summary_str = format!(
            "file={},succeeded={},failed={}",
            file_name, summary.succeeded, summary.failed,
        );
        self.audit_service
            .log(
                Some(actor_id),
                "import_members_batch",
                "member",
                "*",
                None,
                Some(&summary_str),
                None,
            )
            .await;

        Ok(summary)
    }
}
