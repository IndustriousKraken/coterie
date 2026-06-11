## ADDED Requirements

### Requirement: Test mode uses a separate database file

When the wizard's "test mode or live mode?" prompt is answered "test," the resulting Coterie instance SHALL be configured with `DATABASE_URL` pointing at `sqlite:///var/lib/coterie/coterie-test.db?mode=rwc` (instead of `coterie.db`). Test charges, test members, test audit rows, and any other state generated during the verification phase SHALL live in this file.

The live database file `coterie.db` SHALL NOT exist or be touched during test-mode operation. It is created fresh by the switchover subcommand.

#### Scenario: Wizard in test mode creates coterie-test.db

- **WHEN** the wizard runs with test mode selected
- **THEN** `/var/lib/coterie/coterie-test.db` SHALL be created with migrations applied; `/var/lib/coterie/coterie.db` SHALL NOT exist; `.env`'s `COTERIE__DATABASE__URL` SHALL reference `coterie-test.db`

#### Scenario: Test data is fully isolated

- **WHEN** test charges are made during test-mode operation
- **THEN** the resulting member rows, payment rows, audit log entries SHALL be in `coterie-test.db`; the live database is unaffected (because it doesn't exist yet)

### Requirement: coterie-provision switch-stripe-to-live transitions from test to live mode atomically

The `coterie-provision` Rust binary (introduced by `a24`) SHALL gain a `switch-stripe-to-live` subcommand that handles the one-shot transition from test mode to live mode. The subcommand SHALL:

1. Refuse if `.env` already shows `pk_live_*` credentials.
2. Refuse if `/var/lib/coterie/coterie-test.db` doesn't exist.
3. Load live Stripe credentials from `/opt/coterie/.env.live` if present; otherwise prompt for them (interactively or via `--no-prompt` env-var pathway).
4. Validate credentials are well-formed (`pk_live_*`, `sk_live_*`, `whsec_*`) via the existing `validate_prefix` helper.
5. Validate the secret key by calling `https://api.stripe.com/v1/balance` (via the `StripeApi` trait) with it as basic auth. Abort BEFORE any destructive operation if the API rejects.
6. Stop the `coterie` service via the `SystemCommand` trait.
7. Create a fresh `/var/lib/coterie/coterie.db` and ensure migrations are applied.
8. Copy the admin row(s) from `coterie-test.db` to `coterie.db` via `ATTACH DATABASE ... INSERT SELECT`.
9. Archive `coterie-test.db` to `coterie-test-archive-YYYYMMDD-HHMMSS.db` (or delete if `--discard-test-db` was passed).
10. Atomically rewrite `.env` with live credentials and the new `DATABASE_URL` (write to `.env.new`, rename to `.env`).
11. Remove `/opt/coterie/.env.live` if it existed.
12. Start the `coterie` service.
13. Smoke-test `GET http://127.0.0.1:8080/health` returns 200.
14. Print a reminder about registering the live-mode webhook in Stripe's dashboard.

All side-effecting operations SHALL go through the `SystemCommand`, `FileSystem`, and `StripeApi` traits so the subcommand is fully testable via `cargo test` with fake implementations.

#### Scenario: Successful test-to-live switchover

- **WHEN** an operator runs `coterie-provision switch-stripe-to-live` on an instance currently in test mode
- **THEN** after completion: `.env` contains live credentials; `DATABASE_URL` references `coterie.db`; `coterie.db` exists with the admin row preserved; `coterie-test.db` is renamed to an archive name; `.env.live` (if it existed) is removed; the service is running and reachable; the operator's existing admin password works for login

#### Scenario: Refuse to switch when already in live mode

- **WHEN** `coterie-provision switch-stripe-to-live` is invoked on an instance whose `.env` already has `pk_live_*`
- **THEN** the subcommand SHALL exit non-zero with a clear message; no modifications SHALL occur

#### Scenario: Refuse to switch when no test DB exists

- **WHEN** `coterie-provision switch-stripe-to-live` is invoked on an instance that doesn't have a `coterie-test.db` file
- **THEN** the subcommand SHALL exit non-zero ("instance is not in test mode; nothing to switch from"); no modifications

#### Scenario: Stripe API rejects the live secret key

- **WHEN** the supplied live secret key fails the `/v1/balance` validation (as determined by the `StripeApi` trait — `FakeStripeApi` in tests, `RealStripeApi` in prod)
- **THEN** the subcommand SHALL abort BEFORE stopping the service or modifying `.env`; the test-mode instance SHALL remain running

#### Scenario: Integration test covers the happy path via fakes

- **WHEN** the autocoder runs `cargo test -p coterie-provision`
- **THEN** an integration test SHALL exercise `switch-stripe-to-live` end-to-end against `FakeSystem`, `FakeFs`, and `FakeStripeApi`, asserting on the expected command sequence, the .env contents (via golden snapshot), and the archive/removal of test artifacts

### Requirement: Admin row migrates across the DB swap

The switchover subcommand SHALL preserve the operator's admin row(s) from `coterie-test.db` into the new `coterie.db` using SQLite's `ATTACH DATABASE` mechanism. The hashed password, email, username, and `is_admin` flag SHALL all migrate verbatim. The operator's existing credentials SHALL work for login against the new live database without re-entering the password.

#### Scenario: Admin login works after switchover

- **WHEN** an operator logs in via `/login` immediately after the switchover completes
- **THEN** the credentials they used during test-mode operation SHALL authenticate successfully against the new live database

#### Scenario: Multiple admins migrate together

- **WHEN** the test database has multiple admin rows (e.g., the operator added a second admin via the UI during testing)
- **THEN** all rows with `is_admin = 1` SHALL be copied to the live database; the subcommand SHALL NOT silently drop any

### Requirement: Live credentials may be pre-loaded at wizard time

The wizard MAY optionally collect live Stripe credentials at the same time it collects test credentials, storing them in `/opt/coterie/.env.live` (chmod 0640, owned by `coterie`). The switchover subcommand SHALL detect this file and use its contents instead of prompting. This is convenience-only — operators without live credentials at wizard time can defer; the switchover prompts at run time.

After consumption, the switchover SHALL remove `.env.live` so live credentials don't linger in two files on disk.

#### Scenario: Wizard stashes live creds, switchover uses them without prompting

- **WHEN** the wizard runs in test mode and the operator supplies both test AND live credentials
- **THEN** test creds go into `.env`; live creds go into `/opt/coterie/.env.live` with mode 0640
- **WHEN** the operator later runs the switchover
- **THEN** the subcommand SHALL read live creds from `.env.live`, skip the credential prompts, and remove `.env.live` after the .env rewrite completes

#### Scenario: Wizard skips live creds, switchover prompts later

- **WHEN** the wizard runs in test mode and the operator chooses not to supply live credentials
- **THEN** no `.env.live` file is created
- **WHEN** the operator later runs the switchover
- **THEN** the subcommand SHALL prompt for each live credential at run time (or accept them via env/flag in non-interactive mode)
