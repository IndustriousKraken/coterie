## Why

`a13-bulk-member-csv-import` was scoped for fresh-bootstrap imports — its required columns are `email, username, full_name, membership_type_slug` and its optional columns are `status, notes, discord_id`. That covers the "import a list of names" case.

What it doesn't cover is the **migration from an existing billing system** scenario — which is exactly what's about to happen with the neontemple.com production deploy. Members are currently being billed by MemberPress (a WordPress plugin) via Stripe subscriptions. To migrate them into Coterie without disrupting the existing billing relationship, the importer needs to accept:

- `dues_paid_until` — when their current paid-through-date expires. Without this, every imported member looks newly-joined with no dues paid and the billing runner treats them as expired.
- `stripe_customer_id` — so Coterie can take over charging the existing card-on-file when the member organically migrates (e.g., updates their card).
- `stripe_subscription_id` — so Coterie knows which Stripe subscription is theirs and can cancel it during `migrate_to_coterie_managed`.
- `joined_at` (nice-to-have) — for historical accuracy on the member-detail page and receipts.
- `email_verified_at` (nice-to-have) — so MemberPress members aren't asked to re-verify an email address they've used for years.

If `stripe_subscription_id` is present, the import SHALL also set `billing_mode = StripeSubscription` automatically (rather than the default `Manual`), so Coterie correctly observes the existing Stripe-managed billing relationship until it's organically migrated.

## What Changes

- **Extend `ImportRow`** (in `src/service/member_service.rs`) with five optional fields:
  - `dues_paid_until: Option<DateTime<Utc>>`
  - `stripe_customer_id: Option<String>`
  - `stripe_subscription_id: Option<String>`
  - `joined_at: Option<DateTime<Utc>>`
  - `email_verified_at: Option<DateTime<Utc>>`
- **Extend `parse_import_csv`** in `src/web/portal/admin/members/bulk.rs` to recognize these as optional columns in the CSV header. Empty cell = `None`. ISO 8601 parse for the timestamps. Unknown columns continue to be silently ignored per the existing behavior.
- **Extend `MemberService::bulk_import`** to thread the new fields into `CreateMemberRequest`. Currently `CreateMemberRequest` doesn't have these fields — extend it with the same optional fields, and update the repo's `create` to honor them when present (falling back to current defaults when `None`).
- **`billing_mode` inference**: if `stripe_subscription_id.is_some()`, the imported member gets `billing_mode = StripeSubscription`. Otherwise the existing default (`Manual` or whatever the repo currently sets) applies. No explicit `billing_mode` column in the CSV — the subscription ID's presence is the signal.
- **Validation additions**:
  - If `dues_paid_until` is in the past, accept it (legacy-data scenarios may have expired members being imported); just create them with the past date. The billing runner will mark them Expired on its next tick.
  - If `stripe_subscription_id.is_some()` but `stripe_customer_id.is_none()`, that's an inconsistent row — fail it with reason "Stripe subscription_id present without customer_id."
- **No new template work** — the existing import form accepts any CSV the parser understands. Update the format-reminder text on `templates/admin/member_import.html` to list the new optional columns.
- **Out of scope**: bulk update of existing members (the import remains INSERT-only); credential migration (passwords/TOTP secrets cannot be ported); reconciliation of mismatched dues_paid_until (a sanity-check pass against the WP data is an operator's responsibility, not the importer's).

## Capabilities

### New Capabilities

(None — extends existing capability.)

### Modified Capabilities
- `bulk-member-csv-import`: optional column set expands. Behavioral semantics for the new fields documented in the delta spec.

## Impact

- **Code**:
  - `src/service/member_service.rs::ImportRow` — five new optional fields.
  - `src/service/member_service.rs::bulk_import` — thread new fields into `CreateMemberRequest`; add the inconsistency check.
  - `src/domain/member.rs::CreateMemberRequest` — five new optional fields (mirror of `ImportRow`'s additions).
  - `src/repository/member_repository.rs::create` — honor the new fields when `Some`; preserve current defaults when `None`.
  - `src/web/portal/admin/members/bulk.rs::parse_import_csv` — read the new optional columns.
  - `templates/admin/member_import.html` — update format-reminder text.
- **Wire shape**: the CSV import endpoint accepts a superset of what it accepted before. Existing CSVs (without the new columns) continue to work unchanged.
- **Tests**:
  - Update existing import tests to assert that empty-optional rows continue to work.
  - Add a test that an imported row with `stripe_subscription_id` results in `billing_mode = StripeSubscription` on the created member.
  - Add a test for the inconsistency check (subscription_id without customer_id → row failure).
  - Add a test for the timestamp-parsing failure mode (malformed ISO date → row failure with clear reason).
- **Risk**: medium. The change touches the member-creation path. Mitigation: existing import tests catch regressions on the default path; new tests cover the new behavior.
- **Production timing**: This change is required pre-launch. The neontemple.com migration depends on it.
