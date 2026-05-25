## Why

`a24` shipped the `coterie-provision` wizard. Code review on the merged PR surfaced three real bugs in error-handling correctness:

1. **Silently-swallowed filesystem errors.** `RealFs::chown`, `Executor::render_and_write_env`, and `write_caddyfile` use `.ok()` (or otherwise ignore) `Result`s from chown/chmod/create_dir_all. A chown failure on `.env` leaves the file owned by `root`; the wizard reports success; systemd then fails to read `.env` as user `coterie` and the operator stares at journalctl wondering why. Exactly the kind of silent-failure footgun the wizard was supposed to prevent.

2. **`bootstrap_admin` exit-code matching is too loose.** `run_allow_codes` accepts a whitelist (e.g., `&[0, 2]` for create_admin where 2 = "admin already exists"). But `bootstrap_admin` then matches only `0 => ok, _ => already_exists`. If create_admin ever returns code 3 (e.g., a future "validation failed"), it gets silently treated as "admin exists" — the wizard succeeds and the admin doesn't exist.

3. **`smoke_test` has no retry / grace period.** After `systemctl start coterie` and `is-active` reports active, the HTTP server may still be 1–3 seconds from binding port 8080 (sqlx pool init, migrations on first connection, address binding). The wizard's `curl /health` fires once; a transient connection-refused fails the wizard even though the service is healthy moments later.

This change fixes all three. Small surface, mechanical fixes; one Rust crate touched.

## What Changes

- `RealFs::chown`, `RealFs::chmod`, `RealFs::create_dir_all` propagate errors via `?` with `anyhow::Context` annotating which path + which operation failed. Test-side `FakeFs` already records calls; tests assert that error propagation works when configured to fail.
- Every wizard call site that previously dropped these results (`Executor::render_and_write_env`, `write_caddyfile`, the Caddy log-dir setup step) propagates via `?` and adds context.
- `bootstrap_admin` matches `create_admin`'s exit code explicitly: `0 => ok`, `2 => already_exists` (log + skip), `other => bail!("create_admin exited unexpectedly with code {other}")`. Drop the catch-all "any non-zero = admin exists" interpretation.
- `smoke_test` polls `/health` for up to 30 seconds with a 1-second sleep between attempts. The first 2xx response succeeds. Only after the full 30-second budget exhausts without a 2xx does the wizard fail; that failure includes the most recent response status + body in the error context.
- Tests cover: a `FakeFs::chown` configured to return an error fails the wizard; a `FakeSystem` returning `create_admin` exit code 3 fails the wizard with the unexpected-code message; a `FakeSystem` returning connection-refused on the first two `/health` calls then 200 succeeds.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `provisioning-wizard` — gains explicit requirements about error propagation from filesystem operations, exit-code matching precision, and health-check retry behavior. These are additions to the existing capability, not changes to previously-shipped behavior.

## Impact

- **Code**: ~50 lines net change in `deploy/coterie-provision/`. Three error-path fixes + one new retry loop + tests.
- **Wire shape**: zero change. Successful wizard runs produce identical output to before. Failure modes get clearer.
- **Tests**: add ~3 new integration tests against `FakeFs`/`FakeSystem`. Existing tests continue to pass.
- **Risk**: low. The fixes only change behavior in error paths that previously didn't fail-fast; the happy path is unchanged.
- **Dependency**: none in the queue. `a24` is already shipped/merged; this builds on it.
