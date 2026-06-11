## 1. Extend the install subcommand with test/live mode

- [x] 1.1 In `src/install.rs` (from `a24`), after the existing Stripe-enable prompt, add a new mode prompt: "Stripe mode: [test/live]?" with default `live`. Env var: `COTERIE_PROVISION_STRIPE_MODE=test|live`. CLI flag: `--stripe-mode test|live`. Use `a24`'s `resolve` helper so all three input paths (flag/env/prompt) work uniformly.
- [x] 1.2 If test mode: prompt for `pk_test_*`, `sk_test_*`, test `whsec_*`. Validate each via `validate_prefix` (defined in `a24`'s `src/stripe_check.rs`). Refuse to continue if any prefix is wrong.
- [x] 1.3 If test mode: prompt "Do you also have live credentials to pre-load for later switchover?" (default `no`). If yes, prompt for `pk_live_*`, `sk_live_*`, live `whsec_*`. Validate prefixes.
- [x] 1.4 If test mode: when calling `render_env`, supply `DatabaseUrl::Test` (or equivalent) so the generated `.env` has `COTERIE__DATABASE__URL=sqlite:///var/lib/coterie/coterie-test.db?mode=rwc`.
- [x] 1.5 If test mode AND live creds were pre-loaded: write them to `/opt/coterie/.env.live` via the `FileSystem` trait. The file content is the same `COTERIE__STRIPE__*=...` lines a regular `.env` would have, but only the Stripe-live fields. Apply mode 0640 and ownership `coterie:coterie` via the trait methods.
- [x] 1.6 Update `render_env` (or add a new `render_env_live_overlay`) as needed to support the live-only subset. Add golden tests for the new shape.

## 2. Test-mode verification checklist output

- [x] 2.1 Define a `const TEST_MODE_CHECKLIST: &str = "..."` in `src/install.rs` (or a dedicated `src/checklist.rs`) containing the verification text from design.md D8.
- [x] 2.2 In test mode, after the wizard completes (just before the final summary), print the checklist to stdout. Ensure tests can capture it (either via a `Stdout` trait abstraction or by structuring the call so the print is a step that returns its output string in tests).
- [x] 2.3 In live mode, the checklist is NOT printed (matches `a24` baseline output).
- [x] 2.4 Test: integration test in test mode asserts the checklist text appears in captured stdout; in live mode, it does not appear.

## 3. New StripeApi trait

- [x] 3.1 Create `src/stripe_api.rs`. Define `pub trait StripeApi { fn check_balance(&self, secret_key: &Secret<String>) -> Result<()>; }`.
- [x] 3.2 Implement `RealStripeApi` using `reqwest::blocking::Client` (15-second timeout). Calls `https://api.stripe.com/v1/balance` with basic auth (secret key as username, empty password). Returns `Err` on non-2xx with a clear message.
- [x] 3.3 Implement `FakeStripeApi` (feature-gated for tests). Holds a `RefCell<Vec<...>>` of attempted keys + a configurable response policy ("accept all" by default; "reject" for failure-path tests).
- [x] 3.4 Add `StripeApi` as a third generic parameter to the install + switch flows where credential validation is desired (optional in `install`, mandatory in `switch-stripe-to-live`).

## 4. New switch-stripe-to-live subcommand skeleton

- [x] 4.1 In `src/main.rs`, replace the `todo!()` stub for the `SwitchStripeToLive` subcommand with a call into `src/switch_to_live.rs::run(...)`.
- [x] 4.2 Define CLI args for the subcommand in the clap derive struct: `--discard-test-db` (bool, default false), `--yes` (skip confirmation, bool), `--no-prompt` (require env/flags only, bool), pass-through credential flags (`--live-pk`, `--live-sk`, `--live-whsec`).
- [x] 4.3 Create `src/switch_to_live.rs`. Top-level: `pub fn run<S, F, A>(args: SwitchArgs, sys: &S, fs: &F, api: &A, prompts: &impl Prompter) -> Result<()>` where `S: SystemCommand`, `F: FileSystem`, `A: StripeApi`.

## 5. Idempotency + preflight

- [x] 5.1 Refuse if `$EUID != 0` (skip the check in tests via a configurable predicate).
- [x] 5.2 Read `.env` via the `FileSystem` trait. If it contains a line matching `pk_live_`, exit 0 with "Already in live mode; nothing to do." (Exit 0 because this is the "already done" case, not an error.)
- [x] 5.3 If `/var/lib/coterie/coterie-test.db` doesn't exist (per `FileSystem::exists`), exit non-zero with "Not in test mode; no test DB to migrate from."
- [x] 5.4 Tests: assert both refusal paths exit with the expected code, write nothing to `FakeFs`, and record zero commands on `FakeSystem`.

## 6. Load live credentials

- [x] 6.1 If `/opt/coterie/.env.live` exists, parse it. Use `dotenvy` crate (or a small hand-rolled parser: split lines on `=`, strip quotes, build `HashMap<String, String>`). Extract `COTERIE__STRIPE__PUBLISHABLE_KEY`, `COTERIE__STRIPE__SECRET_KEY`, `COTERIE__STRIPE__WEBHOOK_SECRET`.
- [x] 6.2 If `.env.live` doesn't exist, use the prompts/flag/env path (reuse `resolve` helper from `a24`).
- [x] 6.3 Validate each via `validate_prefix`. Refuse with a clear error if any prefix is wrong.
- [x] 6.4 Wrap secret values in `secrecy::Secret<String>` immediately on receipt; never log.

## 7. Stripe API smoke test

- [x] 7.1 Call `api.check_balance(&live_sk)`. If `Err`, abort with "Stripe rejected the live secret key — aborting before any modifications." No service stop, no .env mutation.
- [x] 7.2 Test (with `FakeStripeApi` configured to reject): assert the subcommand exits non-zero and the `FakeSystem` recorded zero commands.

## 8. Confirmation prompt (unless --yes)

- [x] 8.1 Print a summary of what's about to happen: service stop, DB creation, admin migration via ATTACH, archive (or discard per flag), .env rewrite, .env.live removal, service start, smoke test.
- [x] 8.2 Prompt for y/N (unless `--yes` was passed or `--no-prompt` is on, in which case proceed without confirmation). Default N.

## 9. Service stop + new DB setup

- [x] 9.1 `sys.run("systemctl", &["stop", "coterie"])` and check exit status. Abort if non-zero.
- [x] 9.2 Create `/var/lib/coterie/coterie.db` and ensure migrations are applied. Choose ONE approach (document the choice in code with a brief comment):
   - (a) Briefly invoke the main coterie binary with `COTERIE__DATABASE__URL=...coterie.db?mode=rwc`, wait for it to log "listening" or sleep ~3s, send SIGTERM. The binary's startup runs migrations.
   - (b) Use `rusqlite` to execute the contents of `migrations/*.sql` directly. The migration files ship in the release tarball.
   - (c) Copy schema from `coterie-test.db` via `sqlite3 -cmd ".schema" coterie-test.db | sqlite3 coterie.db`.
   Implementer picks based on what's cleanest. (a) is most aligned with how the rest of the codebase works; (b) is most direct.
- [x] 9.3 Tests: assert the chosen approach is exercised — for (a), check the `FakeSystem` recorded the coterie binary invocation; for (b)/(c), assert the right SQL/command sequence.

## 10. Admin row migration via ATTACH DATABASE

- [x] 10.1 Execute the SQL:
  ```sql
  ATTACH DATABASE '/var/lib/coterie/coterie-test.db' AS test;
  INSERT INTO members SELECT * FROM test.members WHERE is_admin = 1;
  DETACH DATABASE test;
  ```
  Preferred: `rusqlite::Connection::open(coterie.db)?.execute_batch(SQL)?`. (If the implementer chose `sqlite3` subprocess for step 9.2, use it here too for consistency.)
- [x] 10.2 Test: in the integration test, use real `tempfile`-based DB files (rusqlite operates on real paths). Pre-seed `coterie-test.db` with the schema + an admin row. After running the switchover step, assert `coterie.db` has the admin row.

## 11. Archive or discard the test DB

- [x] 11.1 Compute archive name: `coterie-test-archive-{timestamp}.db` where timestamp is `YYYYMMDD-HHMMSS` from `chrono::Local::now()`.
- [x] 11.2 If `args.discard_test_db`: `fs.remove_file("coterie-test.db")`. Else: `fs.rename("coterie-test.db", archive_path)`.
- [x] 11.3 Test: both branches covered.

## 12. Atomic .env rewrite

- [x] 12.1 Pure function `pub fn rewrite_env(current: &str, live_pk: &str, live_sk: &Secret<String>, live_whsec: &Secret<String>) -> String`. Substitutes the three `COTERIE__STRIPE__*` lines and the `COTERIE__DATABASE__URL` line (changing `coterie-test.db` → `coterie.db`). Other lines pass through untouched.
- [x] 12.2 Golden tests for `rewrite_env`: input fixture is a representative test-mode `.env`, output fixture is the same content with the four lines swapped.
- [x] 12.3 In the subcommand: read current `.env` via `fs.read_to_string`, compute new content, write to `.env.new` with mode 0640 and ownership `coterie:coterie` via `FileSystem` trait, then `fs.rename(".env.new", ".env")`.
- [x] 12.4 If `/opt/coterie/.env.live` exists, `fs.remove_file(...)` it.

## 13. Service restart + smoke test

- [x] 13.1 `sys.run("systemctl", &["start", "coterie"])`. Poll `systemctl is-active coterie` up to 30 seconds.
- [x] 13.2 If service doesn't reach active state, dump `journalctl -u coterie -n 50 --no-pager` via the trait + exit non-zero. (The .env is now live mode but the service isn't running — no rollback path; operator needs to debug.)
- [x] 13.3 Smoke test: HTTP GET `http://127.0.0.1:8080/health`. If 303 to /setup, dump diagnostics — admin migration failed.

## 14. Final summary + webhook reminder

- [x] 14.1 Print the success summary including the webhook-registration reminder per design.md D9. Read the portal domain from `.env` (just-rewritten content) to substitute into the URL.

## 15. STRIPE-SETUP.md updates

- [x] 15.1 Add a section describing the test-mode-first workflow: how to enable in the wizard, the verification flows to run, when to invoke `coterie-provision switch-stripe-to-live`.
- [x] 15.2 Note that live and test modes have SEPARATE webhook configurations in the Stripe dashboard — both need to be registered if both modes will be used (even sequentially).

## 16. Validation (autocoder-runnable)

- [x] 16.1 `cargo test -p coterie-provision` — all unit and integration tests pass, including the new switchover tests.
- [x] 16.2 `cargo clippy -p coterie-provision -- --deny warnings` — clean.
- [x] 16.3 `cargo fmt --check` — clean.
- [x] 16.4 `cargo build -p coterie-provision --target x86_64-unknown-linux-musl --release` — confirms the binary still builds for the release target after additions.
