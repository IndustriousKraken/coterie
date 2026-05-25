> **Partial-completion note (added when a34 was reopened):**
> The current code already has the `RealFs` trait methods propagating errors (1.1) and the exhaustive `bootstrap_admin` exit-code match (2.1, 2.2). Those checkboxes are pre-ticked.
> The call sites (1.2, 1.3) and the smoke-test retry (section 3) are still missing — the wizard still has three `fs.chown(...).ok()` drops (at `install.rs:971` for `.env`, `:1007` for `.env.live`, `:1088` for the Caddy log dir) that silently swallow ownership failures, and `smoke_test` is still a single `curl /health` with no retry. Pick up from there.

## 1. Propagate filesystem errors

- [x] 1.1 In `deploy/coterie-provision/src/fs_ops.rs`: change `RealFs::chown` to propagate the inner error instead of `.ok()` / `Ok(())`-on-failure. Same for `RealFs::chmod` and `RealFs::create_dir_all`. *(Done — all three propagate via `with_context` / `bail!`.)*
- [ ] 1.2 In `deploy/coterie-provision/src/install.rs`: in `Executor::render_and_write_env`, change every `fs.chown(...).ok()` / `fs.chmod(...).ok()` / equivalent to `fs.chown(...).with_context(|| format!("..."))?`. *(Two sites: `install.rs:971` chown on `.env`, `install.rs:1007` chown on `.env.live`.)*
- [ ] 1.3 Same for `write_caddyfile` and any other call site (grep for `\.ok\(\)` in `src/install.rs` to find them all). *(One remaining site: `install.rs:1088` chown on `/var/log/caddy`. The other `.ok()` matches at lines 631 and 1029 are on `std::env::var` and `tmp.flush` — intentional, leave them.)*
- [ ] 1.4 Add `use anyhow::Context;` where needed.
- [ ] 1.5 Verify: `grep -rn '\.ok()' deploy/coterie-provision/src/` — any remaining matches SHALL be on `Result::ok` for Option conversion (or similar intentional uses), NOT for discarding `FileSystem`/`SystemCommand` results.

## 2. Tighten bootstrap_admin exit-code matching

- [x] 2.1 In `install.rs::bootstrap_admin`, replace the current `0 => ok, _ => already_exists` match with an exhaustive match on the exit status. *(Done — see `bootstrap_admin` match arms: `Ok(0)`, `Ok(2)`, `Ok(other) => Err("unexpected exit code")`, `Err(e) => Err(e)`. The "terminated by signal" case is folded into `Err(e)` since `run_allow_codes` returns Err for signal termination.)*
- [x] 2.2 Keep `run_allow_codes`'s whitelist as `&[0, 2]` so the call still succeeds for both codes; `bootstrap_admin` is the layer that makes the semantic distinction. *(Done.)*

## 3. Add smoke_test retry

- [ ] 3.1 In `install.rs::smoke_test` (or wherever the `/health` check lives), replace the single curl call with a polling loop:
  ```rust
  use std::time::{Duration, Instant};
  use std::thread::sleep;

  let deadline = Instant::now() + Duration::from_secs(30);
  let mut last_error: Option<String> = None;
  loop {
      match sys.run("curl", &["-fsSL", "http://127.0.0.1:8080/health"]) {
          Ok(out) if out.status.success() => return Ok(()),
          Ok(out) => last_error = Some(format!(
              "status={:?}, stdout={}",
              out.status,
              std::str::from_utf8(&out.stdout).unwrap_or("<binary>"),
          )),
          Err(e) => last_error = Some(format!("{}", e)),
      }
      if Instant::now() >= deadline { break; }
      sleep(Duration::from_secs(1));
  }
  bail!("smoke test failed after 30s: {}", last_error.unwrap_or_else(|| "no response".into()));
  ```
- [ ] 3.2 For test-friendliness, consider injecting the sleep duration via a `&dyn Clock` or a constant that tests can override. Otherwise the smoke_test tests will literally sleep for 30 seconds on failure — too slow. Simplest: make the per-iteration sleep duration a `pub(crate) const SMOKE_TEST_INTERVAL: Duration` that tests can shadow via a `cfg(test)` const, OR pass it as an argument with a sensible production default.

## 4. Tests

- [ ] 4.1 Add `chown_failure_aborts_wizard` to `tests/install_flow.rs`. Configure `FakeFs::chown` to return an error on the `.env` chown. Assert `run(...)` returns `Err`; assert the error chain includes `"chown"` and `".env"`.
- [ ] 4.2 Add `unexpected_create_admin_code_aborts`. Configure `FakeSystem` to return exit code 3 for the create_admin invocation. Assert `Err`; assert error message contains `"unexpectedly"` and `"3"`.
- [ ] 4.3 Add `smoke_test_retries_through_startup`. Configure `FakeSystem` so the first 2 calls to `curl /health` return failure (e.g., status code 7 = connection refused) and the 3rd returns success. Assert the wizard succeeds; assert `FakeSystem` recorded ≥3 curl calls.
- [ ] 4.4 (Optional but recommended) Add `smoke_test_fails_after_budget_with_last_error`. Configure `FakeSystem` to return a 500 every call. Assert `Err`; assert error message includes `"500"`. Use the test-friendly sleep override (task 3.2) so this test runs in well under 30 seconds.

## 5. Validation

- [ ] 5.1 `cargo test -p coterie-provision` — all tests pass, including the new ones.
- [ ] 5.2 `cargo clippy -p coterie-provision -- --deny warnings` — clean.
- [ ] 5.3 `cargo fmt --check` — clean.
- [ ] 5.4 Final grep: `grep -rn '\.ok()' deploy/coterie-provision/src/install.rs` returns zero matches that drop a `Result` from a `FileSystem` or `SystemCommand` method.
