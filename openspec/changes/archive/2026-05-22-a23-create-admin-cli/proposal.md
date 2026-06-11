## Why

`/setup` is unauthenticated and reachable to anyone who can reach the server until the first admin exists. That's a real security window during deployment: between Coterie's `systemctl start` and the operator visiting `/setup` to claim the org, anyone on the internet who knows (or guesses) the URL can hijack the deployment by registering as the first admin.

Today this window is small because the operator visits /setup immediately after starting the service. But it's not zero, and it gets larger as soon as you imagine:

- A provisioning wizard that starts the service, then needs minutes to write Caddyfile / wait for DNS / register Stripe webhook before the operator reaches /setup
- A misconfigured firewall that leaves the service exposed before the operator notices
- Any failure that delays the operator's path-to-setup-page

The fix is to support bootstrapping the first admin via a CLI command, BEFORE the service ever serves a request. A provisioning wizard creates the admin via this CLI, THEN starts the service. From the moment Coterie binds port 8080, an admin already exists; the setup-redirect middleware forwards normally and /setup is unreachable.

This change is a prerequisite for the upcoming `provisioning-wizard` work — it's the trustworthy primitive the wizard depends on. It's also independently useful: operators doing disaster-recovery from a snapshot want a way to re-bootstrap an admin without exposing /setup again.

## What Changes

- **New binary**: `src/bin/create_admin.rs`. Mirrors the existing `seed` binary's structure (clap-based CLI, same DB connection logic as the main coterie binary).
- **CLI surface**:
  ```
  Usage: create_admin --email <EMAIL> --username <USERNAME> --full-name <NAME> [--password <PASS> | --password-file <PATH>]
  ```
  - `--email`, `--username`, `--full-name` are required.
  - `--password` is a flag; `--password-file` reads the password from a file (safer for scripted use — env vars are visible in `ps`, files have proper permissions).
  - Exactly one of `--password` or `--password-file` is required.
- **Behavior**:
  1. Load configuration (same `Settings::new()` call as the main binary).
  2. Connect to the database; run migrations to ensure schema is up to date (idempotent — already-applied migrations are skipped).
  3. Check whether any admin already exists. If yes, refuse with exit code 2 and a clear message ("Admin already exists; refusing to create another via CLI. Use the portal admin UI instead."). This makes the binary idempotent-safe — running it twice during a re-provision doesn't create a second admin.
  4. Hash the password using the same `AuthService` code path the manual setup wizard uses (argon2; same parameters; same output format).
  5. Insert the admin row with `status = Active`, `is_admin = true`, `email_verified_at = NOW()`. Roughly equivalent to "the user already verified their email and was activated by an existing admin" — which is the right semantic for a bootstrap admin.
  6. Exit 0 with a brief success message.
- **`GET /setup` defense-in-depth**: the existing `/setup` GET handler should redirect to `/login` if an admin already exists. The middleware currently redirects unsigned-in requests TO /setup; once an admin exists, /setup itself becomes a dead-end (POST already refuses; this change makes GET refuse too, by redirecting to /login). Closes the "operator reaches /setup before noticing the admin exists" UX confusion.

## Capabilities

### New Capabilities
- `bootstrap-admin-cli`: a CLI-only path to create the first admin, used by provisioning automation to close the /setup security window.

### Modified Capabilities
- `routing-architecture`: /setup behavior tightened — the GET handler also redirects (not just the POST) when an admin already exists.

## Impact

- **Code**:
  - New `src/bin/create_admin.rs` (~150 lines, including clap setup, error handling, idempotency check, password hashing call, DB insert).
  - Small change to `Cargo.toml` adding the `[[bin]] name = "create_admin"` block.
  - Small change to the existing `/setup` GET handler in `src/web/templates/setup.rs` to check admin existence and redirect to `/login` if present.
  - The provisioning wizard (separate change) calls this binary as a step.
  - The release tarball already ships every binary in `target/release/`, so `create_admin` flows to prod automatically once the binary is added.
- **Wire shape**: no HTTP shape change. The `/setup` GET handler's redirect-on-admin-exists is a UX/security improvement, not a behavioral wire change in any way that breaks documented contracts.
- **Tests**:
  - Unit test for the create_admin CLI: in-memory DB, runs migrations, calls the binary's logic with synthesized args, asserts an admin row is created.
  - Unit test for the refuse-when-admin-exists path.
  - Unit test for password hashing roundtrip: hash via create_admin, verify via AuthService.
  - Test for the GET /setup → /login redirect when admin exists.
- **Risk**: low. The DB schema doesn't change; we're adding a new way to populate an existing table. The `is_admin = true` insert path is exercised today via the /setup form; the create_admin binary just routes around the HTTP layer.
- **Dependency**: independent of every other queued change. The provisioning-wizard change depends on this.
