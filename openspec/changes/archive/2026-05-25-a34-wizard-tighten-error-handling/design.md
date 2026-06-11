## Context

`a24` shipped a working wizard but its error-path discipline was uneven. Three concrete gaps surfaced in post-merge review:

1. The `FileSystem` trait's `chown`/`chmod`/`create_dir_all` methods return `Result<()>`, but several call sites discard the result via `.ok()` or by ignoring the returned value entirely. The trait was designed for testability; the call-site discipline didn't match.
2. The `run_allow_codes` SystemCommand helper takes a whitelist of "non-error" exit codes. `bootstrap_admin` uses it with `&[0, 2]` for create_admin but then matches only `0 vs _`, treating any non-zero allowed code as "admin exists." It's an underspecified contract that the autocoder filled in with a permissive default.
3. The smoke-test step is a single HTTP call. The systemd-active signal is necessary but not sufficient — the HTTP listener binds asynchronously after the process starts.

This change tightens all three without changing the wizard's happy-path behavior.

## Goals / Non-Goals

**Goals:**
- Filesystem errors propagate with context naming the path and operation.
- `bootstrap_admin` distinguishes create_admin's exit codes explicitly; unknown codes fail loudly.
- `smoke_test` tolerates 1–30 seconds of startup latency before declaring failure.
- All three fixes are testable in the existing `FakeFs`/`FakeSystem` harness; no new test infrastructure required.

**Non-Goals:**
- Refactoring the `FileSystem` or `SystemCommand` trait shape. The traits are correct; the call sites were sloppy.
- Adding rollback / cleanup on error. If the wizard fails partway, the operator runs `uninstall.sh` and re-provisions (existing recovery path).
- Health-check on more than `/health`. We don't probe `/portal` or other endpoints; `/health` is the canonical liveness signal.
- Changing the `D4` design doc's "RAII guard" wording. That's documentation drift, not a runtime concern.

## Decisions

### D1. Error propagation via `?` + `anyhow::Context`

Every call site that calls a fallible `FileSystem` or `SystemCommand` method uses `?` to propagate. Where context isn't obvious from the call, wrap with `.with_context(|| format!("chown {} to coterie:coterie", path.display()))` or similar. The wizard's existing top-level `anyhow` error printer produces a clean chain.

Specifically:
- `RealFs::chown` propagates its inner errors (don't `.ok()`).
- `RealFs::chmod` same.
- `RealFs::create_dir_all` same.
- `Executor::render_and_write_env` propagates the chown/chmod that follows the write.
- `write_caddyfile` propagates the log-dir mkdir + chown.

If a future caller has a *deliberate* reason to ignore a failure (e.g., best-effort cleanup), they use `let _ = ...` with an inline comment explaining why. `.ok()` should not appear in this crate.

### D2. Exit-code matching is exhaustive

`bootstrap_admin` matches explicitly:

```rust
match status.code() {
    Some(0) => info!("admin created"),
    Some(2) => info!("admin already exists; skipping"),
    Some(other) => bail!("create_admin exited unexpectedly with code {other}"),
    None => bail!("create_admin terminated by signal"),
}
```

The `run_allow_codes` whitelist becomes `&[0, 2]` (unchanged) — the helper still treats both as "ran cleanly"; bootstrap_admin then makes the semantic distinction. If `create_admin` ever grows new documented exit codes, this match is the one place to update.

### D3. Smoke-test retry budget

`smoke_test` becomes a polling loop:

```rust
let deadline = Instant::now() + Duration::from_secs(30);
let mut last_error: Option<String> = None;
loop {
    match sys.run("curl", &["-fsSL", "http://127.0.0.1:8080/health"]) {
        Ok(out) if out.status.success() => return Ok(()),
        Ok(out) => last_error = Some(format!("status={}, stdout={}", out.status, str::from_utf8(&out.stdout).unwrap_or("<binary>"))),
        Err(e) => last_error = Some(format!("{}", e)),
    }
    if Instant::now() >= deadline { break; }
    sleep(Duration::from_secs(1));
}
bail!("smoke test failed after 30s: {}", last_error.unwrap_or("no response".to_string()));
```

The `SystemCommand` trait is unchanged. Tests provide a `FakeSystem` configured to return error for the first N invocations of `curl /health` then success on the (N+1)th. The wizard reaches success.

If the service is genuinely broken, the wizard fails after the budget exhausts with the most recent error string in the bail message — operator sees what was wrong.

### D4. Tests

Three new integration tests in `tests/install_flow.rs` (or its peer):

- `chown_failure_aborts_wizard` — `FakeFs::chown` set to fail on `.env`. Wizard returns Err; error chain contains "chown" and ".env".
- `unexpected_create_admin_code_aborts` — `FakeSystem` returns exit code 3 for the create_admin invocation. Wizard returns Err with "unexpectedly with code 3" in the chain.
- `smoke_test_retries_through_startup` — `FakeSystem` returns connection-refused for the first 2 curl /health calls, then a 200 body. Wizard succeeds; assert `FakeSystem` recorded ≥3 curl calls.

## Risks / Trade-offs

- **Risk**: tightening error propagation surfaces failures that the wizard previously hid. → That's the goal, but: an operator might see a wizard that *used* to "succeed" now fail because chown couldn't change ownership on an unusual filesystem (e.g., a CIFS mount). → Mitigation: the error chain says exactly what failed and where; the operator fixes the underlying issue and reruns (the wizard is idempotent per a24's spec).
- **Risk**: smoke-test retry hides genuinely broken services for an extra 29 seconds. → 29 seconds is a small budget; if the service is dead, it'll be dead 30 seconds later too. The retry only hides transient binding latency.
- **Trade-off**: the smoke-test loop adds ~30 lines (vs. the current single call). Worth it given how often early-startup race conditions bite operators.

## Migration Plan

Single PR.

1. Update `RealFs::chown/chmod/create_dir_all` to propagate errors.
2. Update the three identified call sites to use `?`.
3. Rewrite `bootstrap_admin`'s exit-code match.
4. Rewrite `smoke_test` as a polling loop.
5. Add the three integration tests.
6. `cargo test`, `cargo clippy --deny warnings`, `cargo fmt --check` — autocoder validation.
