## ADDED Requirements

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
