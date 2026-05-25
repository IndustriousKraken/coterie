# provisioning-wizard Specification

## Purpose
TBD - created by archiving change a34-wizard-tighten-error-handling. Update Purpose after archive.
## Requirements
### Requirement: Filesystem operation errors propagate with context

Every call to `FileSystem::chown`, `FileSystem::chmod`, `FileSystem::create_dir_all`, `FileSystem::write`, or `FileSystem::rename` SHALL propagate its `Result` via `?`. If the failure mode is not obvious from the immediate call, the call SHALL be annotated with `anyhow::Context` naming the path and the operation.

`.ok()` SHALL NOT be used to discard `Result`s from `FileSystem` methods. A deliberate ignore SHALL use `let _ = ...` with an inline comment explaining why the failure is safe to drop.

The `RealFs` implementations of `chown`, `chmod`, and `create_dir_all` SHALL propagate their inner errors (not return `Ok(())` on inner failure).

#### Scenario: chown failure on .env fails the wizard with a clear error

- **WHEN** `FakeFs` is configured to return an error on `chown(/opt/coterie/.env, "coterie", "coterie")`
- **THEN** the wizard SHALL return `Err`; the error chain SHALL include the string `chown` and the path `.env`

#### Scenario: create_dir_all failure on /var/log/caddy fails the wizard

- **WHEN** `FakeFs` is configured to return an error on `create_dir_all(/var/log/caddy)`
- **THEN** the wizard SHALL return `Err`; the error chain SHALL include the path `/var/log/caddy`

#### Scenario: .ok() does not appear in the crate

- **WHEN** `grep -rn '\.ok()' deploy/coterie-provision/src/` is run
- **THEN** any matches SHALL be on `Option::ok()` (e.g., `Result::ok` for converting to Option) and NOT on dropping a `Result` from `FileSystem` or `SystemCommand` methods; the autocoder SHALL verify each remaining match is intentional

### Requirement: bootstrap_admin distinguishes create_admin exit codes explicitly

The wizard's `bootstrap_admin` step SHALL match on `create_admin`'s exit code exhaustively:

- `Some(0)` — admin created successfully; continue.
- `Some(2)` — admin already exists; log and skip (per `a23`'s exit-code contract).
- `Some(other)` — unexpected exit code; fail with a clear error including the code.
- `None` — process terminated by signal; fail with a clear error.

The catch-all "any non-zero is admin-exists" interpretation SHALL be removed. If `create_admin` grows new documented exit codes, this match is the one place to update.

#### Scenario: create_admin exit code 3 fails the wizard

- **WHEN** `FakeSystem` returns exit code 3 from the `create_admin` invocation
- **THEN** the wizard SHALL return `Err`; the error message SHALL contain `unexpectedly` and `3`

#### Scenario: create_admin terminated by signal fails the wizard

- **WHEN** `FakeSystem` returns `None` for the `create_admin` exit status (signal termination)
- **THEN** the wizard SHALL return `Err`; the error message SHALL contain `signal`

### Requirement: smoke_test retries within a 30-second budget

The wizard's `smoke_test` step SHALL poll `GET http://127.0.0.1:8080/health` up to once per second for up to 30 seconds. The first 2xx response SHALL succeed; the wizard continues to the exit summary.

If the budget exhausts without a 2xx response, the wizard SHALL fail with an error whose message includes the most recent attempt's status code or connection error string. This is so the operator sees *why* the smoke test failed (connection refused vs. 500 vs. 503), not just that it failed.

The retry budget SHALL NOT apply to other wizard steps. It is specific to the post-`systemctl start` HTTP probe, where the gap between systemd reporting `active` and the HTTP listener binding is the documented race condition.

#### Scenario: Wizard succeeds when health endpoint becomes available within the budget

- **WHEN** `FakeSystem` is configured so the first 2 calls to `curl /health` return connection-refused and the 3rd returns a 200 with JSON
- **THEN** the wizard SHALL succeed; `FakeSystem` SHALL record at least 3 calls to `curl /health`

#### Scenario: Wizard fails with the last error when the budget exhausts

- **WHEN** `FakeSystem` is configured so every call to `curl /health` returns a 500 response
- **THEN** the wizard SHALL return `Err` after the 30-second budget exhausts; the error message SHALL include `500` and the connection-refused / response-body context

#### Scenario: Retry budget is bounded

- **WHEN** the smoke_test is observed in any of the failure-path tests
- **THEN** the total elapsed time SHALL NOT exceed ~30 seconds (allow up to ~32s for scheduling slop); test harness uses a `FakeClock` or short-poll override to keep tests fast

