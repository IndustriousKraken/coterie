> **Partial-completion note (added when a34 was reopened):**
> The current code already has the `RealFs` trait methods propagating errors (1.1) and the exhaustive `bootstrap_admin` exit-code match (2.1, 2.2). Those checkboxes are pre-ticked.
> The call sites (1.2, 1.3) and the smoke-test retry (section 3) are still missing — the wizard still has three `fs.chown(...).ok()` drops (at `install.rs:971` for `.env`, `:1007` for `.env.live`, `:1088` for the Caddy log dir) that silently swallow ownership failures, and `smoke_test` is still a single `curl /health` with no retry. Pick up from there.

## 1. Propagate filesystem errors

- [x] 1.1 In `deploy/coterie-provision/src/fs_ops.rs`: change `RealFs::chown` to propagate the inner error instead of `.ok()` / `Ok(())`-on-failure. Same for `RealFs::chmod` and `RealFs::create_dir_all`. *(Done — all three propagate via `with_context` / `bail!`.)*
- [x] 1.2 In `deploy/coterie-provision/src/install.rs`: in `Executor::render_and_write_env`, change every `fs.chown(...).ok()` / `fs.chmod(...).ok()` / equivalent to `fs.chown(...).with_context(|| format!("..."))?`. *(Two sites: `install.rs:971` chown on `.env`, `install.rs:1007` chown on `.env.live`.)*
- [x] 1.3 Same for `write_caddyfile` and any other call site (grep for `\.ok\(\)` in `src/install.rs` to find them all). *(One remaining site: `install.rs:1088` chown on `/var/log/caddy`. The other `.ok()` matches at lines 631 and 1029 are on `std::env::var` and `tmp.flush` — intentional, leave them.)* Also fixed `switch_to_live.rs:257` (same `.ok()` chown-drop pattern, covered by spec requirement 1).
- [x] 1.4 Add `use anyhow::Context;` where needed. *(Already present in `install.rs:1` and `switch_to_live.rs:27`.)*
- [x] 1.5 Verify: `grep -rn '\.ok()' deploy/coterie-provision/src/` — any remaining matches SHALL be on `Result::ok` for Option conversion (or similar intentional uses), NOT for discarding `FileSystem`/`SystemCommand` results. *(Remaining matches: `install.rs:648` is `std::env::var(...).ok()` (Result→Option, intentional); `install.rs:1025` is `tmp.flush().ok()` on `std::io::Result` for a tempfile we're about to shred-and-drop, not a `FileSystem`/`SystemCommand` method.)*

## 2. Tighten bootstrap_admin exit-code matching

- [x] 2.1 In `install.rs::bootstrap_admin`, replace the current `0 => ok, _ => already_exists` match with an exhaustive match on the exit status. *(Done — `bootstrap_admin` now calls `self.sys.run` directly and matches on `out.status`: `0` → created, `2` → already exists, `-1` → terminated by signal, `other` → "create_admin exited unexpectedly with code {other}". The previous `run_allow_codes(&[0, 2])` path swallowed code 3+ into a generic "failed" message, which couldn't satisfy the spec scenario requiring `unexpectedly` in the error.)*
- [x] 2.2 Keep `run_allow_codes`'s whitelist as `&[0, 2]` so the call still succeeds for both codes; `bootstrap_admin` is the layer that makes the semantic distinction. *(`bootstrap_admin` is now the only caller and it bypasses `run_allow_codes` entirely. The helper had no other callers, so it was removed rather than left as dead code. The semantic distinction now lives in `bootstrap_admin`'s direct match on `out.status`.)*

## 3. Add smoke_test retry

- [x] 3.1 In `install.rs::smoke_test` (or wherever the `/health` check lives), replace the single curl call with a polling loop. *(Done — `smoke_test` polls `curl -fsSL http://127.0.0.1:8080/health` once per `smoke_test_interval` until either a 2xx (Ok with `success()`) or the deadline. The last failure (status+stdout+stderr or anyhow error string) is bubbled into the final `Err` so the operator can see what went wrong.)*
- [x] 3.2 For test-friendliness, consider injecting the sleep duration via a `&dyn Clock` or a constant that tests can override. *(Done — production defaults live as `pub(crate) const SMOKE_TEST_INTERVAL: Duration = Duration::from_secs(1)` and `SMOKE_TEST_BUDGET: Duration = Duration::from_secs(30)`. `InstallArgs` exposes two `#[doc(hidden)]` `Option<Duration>` overrides (`smoke_test_interval`, `smoke_test_budget`) that integration tests set to 1ms / 50ms so the failure-budget test runs in well under a second.)*

## 4. Tests

- [x] 4.1 Add `chown_failure_aborts_wizard` to `tests/install_flow.rs`. *(Done — uses new `FakeFs::fail_chown_on(...)` helper; asserts the formatted `anyhow` chain contains both `chown` and `.env`.)*
- [x] 4.2 Add `unexpected_create_admin_code_aborts`. *(Done — uses new `FakeSystem::respond_to_cmd(...)` helper since create_admin's args include a dynamic tempfile path. Asserts the error contains `unexpectedly` and `3`.)*
- [x] 4.3 Add `smoke_test_retries_through_startup`. *(Done — uses new `FakeSystem::respond_to_sequence(...)` helper. Asserts ≥3 recorded curl calls and that the wizard returns Ok once the 3rd response succeeds.)*
- [x] 4.4 Add `smoke_test_fails_after_budget_with_last_error`. *(Done — every curl returns exit 22 with "500" stderr. Asserts the error message contains `500` and `smoke test failed`, and that the total wall-clock time stays under 5s with the test-budget override.)*

## 5. Validation

- [x] 5.1 `cargo test -p coterie-provision` — all tests pass, including the new ones. *(89 passing across lib, bin, install_flow, switch_to_live, caddyfile.)*
- [x] 5.2 `cargo clippy -p coterie-provision -- --deny warnings` — clean. *(Ran with `--all-targets --features test-support` so the test-only items are covered too; zero warnings.)*
- [x] 5.3 `cargo fmt --check` — clean.
- [x] 5.4 Final grep: `grep -rn '\.ok()' deploy/coterie-provision/src/install.rs` returns zero matches that drop a `Result` from a `FileSystem` or `SystemCommand` method. *(Remaining matches at lines 648 and 1025 are `std::env::var` → Option and `std::io::Write::flush` on a tempfile; neither is a `FileSystem` or `SystemCommand` method.)*
