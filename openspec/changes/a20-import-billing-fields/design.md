## Context

`a13-bulk-member-csv-import` shipped the original importer. Its `ImportRow` shape was deliberately narrow — required identity fields plus a few optional descriptive ones. The design assumed bootstrap-from-list, not migration-from-existing-billing.

The neontemple.com production deploy needs the latter case. Members in MemberPress have:
- An active Stripe subscription (customer + subscription IDs known to MP and Stripe).
- A `dues_paid_until` equivalent (MP calls it something else but the concept maps).
- A long history (`joined_at` is real data they care about).
- An already-verified email (don't make them re-verify).

The importer needs to accept all of that, in a way that the resulting Coterie member behaves correctly: shows as Active, has a real next-renewal date, and is connected to the existing Stripe billing so the `migrate_to_coterie_managed` flow can take over organically when the card rolls over.

## Goals / Non-Goals

**Goals:**
- The CSV importer accepts the five new optional columns: `dues_paid_until`, `stripe_customer_id`, `stripe_subscription_id`, `joined_at`, `email_verified_at`.
- `billing_mode = StripeSubscription` is inferred when `stripe_subscription_id` is present, so the imported member is correctly observed by the existing Stripe-webhook flow.
- Existing imports (without the new columns) continue to work byte-identically.
- Invalid combinations are caught per-row with clear reasons.

**Non-Goals:**
- Upsert semantics. The importer stays INSERT-only. A duplicate email is still a row failure.
- Importing credentials (password hashes, TOTP secrets). Members must use password-reset to claim their account in Coterie.
- Importing `is_admin`. Admin flag is set via the manual admin UI, not via bulk import.
- Bulk-update of existing members.
- A separate "migration mode" UI. The same import form handles both bootstrap and migration; the difference is just which columns the operator supplies.

## Decisions

### D1. New columns are optional

The existing required-vs-optional discipline holds. Operators bootstrapping a fresh org provide only the required columns; operators migrating from MP provide the additional ones. The two paths share one importer.

### D2. `billing_mode` is inferred, not specified

There's no `billing_mode` column in the CSV. The presence of `stripe_subscription_id` is the signal:

| `stripe_subscription_id` present? | Resulting `billing_mode` |
|-----------------------------------|--------------------------|
| Yes                               | `StripeSubscription`     |
| No                                | `Manual` (current default) |

The third mode, `CoterieManaged`, is unreachable from the importer — that mode requires a card to be saved into Coterie, which can't happen via CSV. Members reach `CoterieManaged` organically by saving a card after import.

Rationale: making `billing_mode` an explicit column risks operators setting it inconsistently with the IDs (e.g., `billing_mode=Manual` with `stripe_subscription_id` set). Inferring from data avoids the inconsistency class.

### D3. Inconsistency check: subscription_id requires customer_id

A Stripe subscription always has a customer. A row with `stripe_subscription_id` but no `stripe_customer_id` is malformed; reject it as a per-row failure. The reverse is fine — a `stripe_customer_id` without a subscription represents a member who has paid (and has a card on file) but isn't on auto-renew; they import as `Manual` mode with the customer_id retained.

### D4. Timestamps are ISO 8601 in UTC

The existing infrastructure uses `DateTime<Utc>` and ISO format. CSV cells like `2024-03-15T14:30:00Z` parse via `DateTime::parse_from_rfc3339(...)`. Operators who export from MemberPress need to ensure the dates are in UTC (or include a timezone offset).

A malformed timestamp is a per-row failure with a clear reason: "Could not parse <field>: '<cell value>'". Not an abort-the-batch error — operators may have a few bad rows in a 200-row CSV and want the good ones to succeed.

Date-only formats (`2024-03-15`) are also accepted, interpreted as `2024-03-15T00:00:00Z`. Convenience for operators whose source data doesn't include time.

### D5. `dues_paid_until` in the past is accepted

Operators may legitimately import expired members (e.g., to give them a path to restore via the dues-restoration flow). The importer doesn't second-guess: if the date is in the past, the row is created with that date. The billing runner marks them Expired on its next tick, same as it would for any expired member.

### D6. `email_verified_at` semantics

If supplied, the member is created with their email considered verified — the verification email is NOT sent on creation. If omitted, the verification email IS sent (same as today's flow).

For the MP migration, every member has an established email; verification would be a needless friction. So the importer's caller (the migration operator) supplies the field for all rows.

### D7. `CreateMemberRequest` parallel extension

`ImportRow`'s new fields are passed through `CreateMemberRequest` to the repository. This means `CreateMemberRequest` (used by the import path and by the admin "create member" form) also gains the optional fields. The admin form doesn't currently render inputs for them — it could later if there's a need — but the type accepts them so the import path doesn't need a divergent shape.

Alternative considered: add a separate `BulkImportRequest` shape that diverges from `CreateMemberRequest`. Rejected — duplicating the request shape means the repo needs two `create` variants, and the field-passing logic is identical anyway.

### D8. Repo `create` honors None vs Some

The existing repo `create` writes default values for fields the request doesn't supply (e.g., `dues_paid_until = NULL`, `joined_at = now()`). The change updates `create` so that `Some(value)` writes the supplied value and `None` writes the default. The schema's `NULL` semantics for these columns are unchanged.

## Risks / Trade-offs

- **Risk**: an operator imports rows with `stripe_subscription_id`s that reference subscriptions Stripe doesn't know about (typo, stale data, wrong account). When Coterie tries to react to a webhook for an unknown sub_id, nothing happens — the lookup returns no member. → **Mitigation**: documented limitation. A future change could add a "verify subscription exists" pre-flight check against Stripe, but that adds Stripe API calls during import and is out of scope here.
- **Risk**: timestamps in the wrong timezone (e.g., local time treated as UTC). → **Mitigation**: design D4 requires ISO 8601 with timezone (or date-only). Operator's responsibility.
- **Risk**: a Stripe webhook arrives for a subscription whose member was imported with the wrong `dues_paid_until`. The `handle_invoice_paid` flow extends dues from the existing value, so a too-far-future `dues_paid_until` results in dues extending past where they "should." → **Mitigation**: validation during the manual spot-check phase of the migration (see deploy plan Phase 4).
- **Trade-off**: `CreateMemberRequest` gains five Option fields, three of which the admin "create member" form never supplies. Mildly bloats that type for the import use case. Acceptable; alternative was a divergent shape with more duplication.

## Migration Plan

Single PR.

1. Extend `CreateMemberRequest` in `src/domain/member.rs` with the five new `Option` fields.
2. Update `MemberRepository::create` impl in `src/repository/member_repository.rs` to honor the new fields.
3. Extend `ImportRow` in `src/service/member_service.rs` with the five new `Option` fields. Update `MemberService::bulk_import` to thread them through into `CreateMemberRequest`. Add the inconsistency check (subscription without customer → failure).
4. Add the `billing_mode` inference: after constructing the member via `repo.create`, if `stripe_subscription_id.is_some()`, call `repo.set_billing_mode(new_member.id, BillingMode::StripeSubscription, Some(&sub_id))`.
5. Update `parse_import_csv` in `src/web/portal/admin/members/bulk.rs` to recognize the new column names and parse their values. Use `chrono::DateTime::parse_from_rfc3339` and fall back to date-only parsing (`NaiveDate::parse_from_str`).
6. Update `templates/admin/member_import.html` format-reminder block to list the new optional columns.
7. Update existing import tests to confirm the default path still works.
8. Add tests for the new behavior: `stripe_subscription_id` → `StripeSubscription` mode; subscription-without-customer → failure; malformed timestamp → row failure; `email_verified_at` supplied → no verification email queued.
9. `cargo build --all-targets --features test-utils` — clean.
10. `cargo test --features test-utils` — full suite passes.
