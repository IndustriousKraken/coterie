## ADDED Requirements

### Requirement: A `create_admin` binary creates the first admin without HTTP

The system SHALL ship a `create_admin` binary at `target/release/create_admin` (and in the release tarball alongside `coterie` and `seed`). It SHALL accept the following arguments:

- `--email <EMAIL>` (required)
- `--username <USERNAME>` (required)
- `--full-name <NAME>` (required)
- `--password <PASSWORD>` OR `--password-file <PATH>` (exactly one required)

The binary SHALL:

1. Load configuration via `Settings::new()` (same as the coterie server binary).
2. Connect to the SQLite database via `SqliteConnectOptions::create_if_missing(true)` (matching the server's connect logic).
3. Run pending migrations via `sqlx::migrate!()`.
4. Refuse with exit code 2 and a clear stderr message if any admin already exists.
5. Hash the password using the same argon2 parameters as `AuthService` uses for /setup form submissions.
6. Insert a single member row with `status = Active`, `is_admin = true`, `email_verified_at = NOW()`, and the hashed password.
7. Exit 0 on success.

#### Scenario: Happy path on a fresh database

- **WHEN** `create_admin --email founder@org --username founder --full-name 'Founder Name' --password-file /tmp/pw` runs against a fresh, migrated DB
- **THEN** exit code SHALL be 0; a single `members` row SHALL exist with the supplied email, username, full_name, `is_admin = 1`, `status = "Active"`, and `email_verified_at` set to a recent timestamp; the password hash SHALL be verifiable via the existing `AuthService::verify_password` logic

#### Scenario: Refuse when admin already exists

- **WHEN** `create_admin` is invoked against a DB that already contains a row with `is_admin = 1`
- **THEN** exit code SHALL be 2; stderr SHALL contain a message like "Admin already exists; refusing to create another via CLI"; the database SHALL be unchanged

#### Scenario: Refuse when both password forms are supplied

- **WHEN** `create_admin` is invoked with both `--password` and `--password-file`
- **THEN** exit code SHALL be non-zero (clap's standard usage error, code 2); stderr SHALL identify the mutual exclusion

#### Scenario: Migrations run automatically

- **WHEN** `create_admin` is invoked against a DB file that doesn't exist yet
- **THEN** the SQLite file is created, migrations are applied to bring the schema to current, and then the admin insert proceeds (single command brings a totally fresh DB to "ready with one admin")

### Requirement: Password is read safely from a file

When `--password-file` is supplied, the binary SHALL read the entire file contents, strip trailing whitespace (including a trailing newline if present), and use the result as the password. The file SHALL NOT be deleted or modified by the binary — its lifecycle is the caller's responsibility.

The `--password <STRING>` form is supported for testing and ad-hoc use, but the recommended path for automation (the provisioning wizard, scripted re-deploys) is `--password-file` to avoid the password appearing in process listings.

#### Scenario: Password file with trailing newline

- **WHEN** the password file contains `"secret123\n"` (with a trailing newline as would happen from `echo` or a heredoc)
- **THEN** the resulting password hash SHALL match `"secret123"` (no trailing newline); verification with `"secret123"` SHALL succeed
