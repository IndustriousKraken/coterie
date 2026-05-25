# bulk-member-csv-import Specification

## Purpose
TBD - created by archiving change a13-bulk-member-csv-import. Update Purpose after archive.
## Requirements
### Requirement: Admins can bulk-create members from a CSV upload

The system SHALL expose `POST /portal/admin/members/import` accepting `multipart/form-data` with a CSV file in a `file` field. The endpoint SHALL be gated by `require_admin_redirect`. Maximum file size SHALL be 5 MB.

The CSV's header row MUST include the columns `email`, `username`, `full_name`, `membership_type_slug`. Optional columns: `status` (default `Pending`), `notes`, `discord_id`. Other columns SHALL be silently ignored.

Each data row SHALL be processed independently. A failing row SHALL NOT prevent subsequent rows from succeeding.

#### Scenario: Successful import of N members

- **WHEN** an admin uploads a CSV with 10 valid rows
- **THEN** 10 new `members` rows SHALL be created with `status = Pending` (or the row's specified status); each gets its own `audit_logs` row with `action = "import_member"`

#### Scenario: Partial-failure import

- **WHEN** an admin uploads a CSV where 7 rows are valid and 3 have duplicate emails
- **THEN** 7 members SHALL be created; the response SHALL list the 3 failures with row index, the offending email, and a reason (e.g., "Email already exists")

#### Scenario: Missing required column aborts the batch

- **WHEN** the uploaded CSV lacks the `email` column in its header
- **THEN** the handler SHALL return an error response identifying the missing column; no rows SHALL be created

#### Scenario: Unknown membership_type_slug fails the row, not the batch

- **WHEN** a row's `membership_type_slug` doesn't match any active row in `membership_types`
- **THEN** that row SHALL be reported as a failure with the slug in the reason; other rows continue

#### Scenario: Imported members have no password

- **WHEN** a row creates a new member
- **THEN** the resulting `members.password_hash` SHALL be a sentinel (matching the existing "no password yet" pattern in `MemberRepository::create`, if one exists; otherwise the row is unusable for login until the member sets a password via password-reset)

### Requirement: Import does not overwrite existing members

The import path SHALL be INSERT-only. A row whose `email` OR `username` matches an existing member SHALL be reported as a failure; the existing member SHALL NOT be modified.

#### Scenario: Duplicate email is a failure, not an update

- **WHEN** a row's email matches an existing member's email
- **THEN** the row SHALL be reported as failed; the existing member's data SHALL be unchanged

### Requirement: Import emits per-row and aggregate audit rows

The import SHALL emit one `audit_logs` row per successfully created member (`action = "import_member"`) AND one aggregate row at the end of the batch (`action = "import_members_batch"`). Both carry the importing admin's `actor_id`.

#### Scenario: Per-row audit row identifies the new member

- **WHEN** a row is successfully imported
- **THEN** an `audit_logs` row SHALL be inserted with `action = "import_member"`, `entity_id = <new member uuid>`, and `new_value = Some(email)`

#### Scenario: Aggregate audit row summarizes the batch

- **WHEN** a bulk import completes
- **THEN** an `audit_logs` row SHALL be inserted with `action = "import_members_batch"`, `entity_id = "*"`, and `new_value` summarizing the counts (e.g., `"file=members.csv,succeeded=42,failed=3"`)

### Requirement: Importer accepts billing-migration optional columns

The CSV importer SHALL recognize five additional optional columns:

- `dues_paid_until` (ISO 8601 timestamp or `YYYY-MM-DD` date): seeds the member's paid-through date.
- `stripe_customer_id` (string): the Stripe `cus_*` id this member maps to.
- `stripe_subscription_id` (string): the Stripe `sub_*` id for an active subscription.
- `joined_at` (ISO 8601 timestamp or `YYYY-MM-DD` date): seeds the historical join date.
- `email_verified_at` (ISO 8601 timestamp or `YYYY-MM-DD` date): marks the email as verified at import time; suppresses the verification-email send.

All five SHALL be optional. An empty cell parses as `None`. Rows omitting all five behave identically to the pre-change behavior.

#### Scenario: Empty optional cells preserve current defaults

- **WHEN** an import CSV omits the new columns entirely OR provides empty cells for them
- **THEN** the imported member is created with `dues_paid_until = NULL`, `stripe_customer_id = NULL`, `stripe_subscription_id = NULL`, `joined_at = NOW()`, and `email_verified_at = NULL`; the verification email is sent — matching the existing behavior

#### Scenario: Date-only timestamps are accepted

- **WHEN** an import cell contains `2024-03-15` (no time component)
- **THEN** the parser interprets it as `2024-03-15T00:00:00Z` and the row succeeds

#### Scenario: Malformed timestamp produces a per-row failure

- **WHEN** an import cell contains `not-a-date` for a timestamp column
- **THEN** that row fails with reason `"Could not parse <field>: 'not-a-date'"`; subsequent rows are unaffected

### Requirement: billing_mode is inferred from stripe_subscription_id presence

If the imported row carries `stripe_subscription_id`, the resulting member SHALL have `billing_mode = StripeSubscription`. If it doesn't, `billing_mode` defaults to `Manual` (the current behavior). The importer SHALL NOT accept an explicit `billing_mode` column.

Rationale: avoids inconsistency where an operator might set `billing_mode = Manual` while also supplying a `stripe_subscription_id`. The data IS the signal.

#### Scenario: Subscription ID triggers StripeSubscription mode

- **WHEN** an import row supplies both `stripe_customer_id` and `stripe_subscription_id`
- **THEN** the created member has `billing_mode = StripeSubscription`, with both ID fields populated

#### Scenario: Customer-only triggers Manual mode

- **WHEN** an import row supplies `stripe_customer_id` but not `stripe_subscription_id`
- **THEN** the created member has `billing_mode = Manual` (the customer ID is retained for future card-save flows but no subscription is being observed)

### Requirement: subscription_id without customer_id fails the row

A row with `stripe_subscription_id` set but `stripe_customer_id` empty is malformed — a Stripe subscription always has a customer. The importer SHALL fail such a row with a clear reason; other rows in the batch are unaffected.

#### Scenario: Subscription without customer is rejected

- **WHEN** an import row supplies `stripe_subscription_id = sub_ABC` but leaves `stripe_customer_id` empty
- **THEN** the row fails with reason `"Stripe subscription_id present without customer_id"`; the import summary reports it among the per-row failures

### Requirement: email_verified_at present skips verification email

If an imported row supplies `email_verified_at`, the member SHALL be created with that verification timestamp AND the verification email SHALL NOT be sent. If the row omits `email_verified_at`, the verification email SHALL be sent as before.

#### Scenario: Pre-verified member skips email

- **WHEN** an import row supplies `email_verified_at = 2024-01-01T00:00:00Z`
- **THEN** the created member's `email_verified_at` is set; no email is queued; the import-result summary still counts the row as succeeded

#### Scenario: No email_verified_at sends verification

- **WHEN** an import row omits `email_verified_at`
- **THEN** the verification email is queued for the new member, matching the current behavior

