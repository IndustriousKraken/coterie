## Why

`a24` ships a provisioning wizard that takes a fresh Debian box from clean to "Coterie running with Stripe live keys." That works for operators who've already verified Stripe wiring elsewhere (staging, dev, the team's prior experience). It DOESN'T address the operator who wants to verify-before-committing — the "anxious admin" deploying for the first time, the new-build verification pass before a major release, the disaster-recovery operator who restored from backup and wants to confirm everything works before flipping back to live.

The fix is to let the wizard optionally start in **Stripe test mode** with a **separate database file** (`coterie-test.db`), then provide a **one-shot switchover subcommand** that transitions to live mode cleanly: swap credentials, swap to a fresh live DB, migrate the admin row across, restart.

Several use cases this enables:

1. **First-deploy verification**: operator runs a few test charges through real Stripe (using `4242 4242 4242 4242`), watches them appear in the test-mode dashboard, sees the audit trail in Coterie, then switches to live mode confident the wiring works.
2. **Permanent test instance**: operator deploys `test.coterie.example.com` and never runs the switchover — the box stays in test mode forever, useful for testing new builds without risk.
3. **Major-version verification**: when a release touches payment code, spin up a fresh instance with the new build, verify against test-mode Stripe, then upgrade prod.
4. **Disaster-recovery verification**: restored from backup → start in test mode → verify Stripe still works against the restored data → switch back to live.

The model rests on the fact that Stripe's test and live modes are fully isolated: separate credential triples (pk + sk + webhook secret), separate dashboard configurations, separate webhook endpoints (each with their own signing secret). The wizard collects test credentials initially; the switchover collects live credentials at the moment they're needed.

## What Changes

- **Extend `a24`'s wizard** (the `coterie-provision install` subcommand) with a "test mode or live mode?" prompt (default: live, matches `a24`'s shipped behavior). If test:
  - Collect Stripe TEST credentials (`pk_test_*`, `sk_test_*`, test webhook signing secret).
  - Set `DATABASE_URL` in `.env` to point at `coterie-test.db` instead of `coterie.db`.
  - Print a verification checklist after the wizard finishes (which flows to test manually, how to confirm each succeeded).
  - Optionally collect LIVE credentials too if the operator has them ready — store in `/opt/coterie/.env.live` (chmod 0640, owned by `coterie`) for the switchover to pick up later without re-prompting.
- **New `coterie-provision switch-stripe-to-live` subcommand** (added to the same Rust binary `a24` introduces) that:
  - Refuses to run if `.env` already shows `pk_live_*` (idempotent — can't double-switch).
  - Refuses to run if `/var/lib/coterie/coterie-test.db` doesn't exist (operator must be in test mode currently).
  - Loads live credentials from `/opt/coterie/.env.live` if present; otherwise prompts for them.
  - Validates each credential matches the expected prefix (`pk_live_`, `sk_live_`, `whsec_`).
  - Calls Stripe's `/v1/balance` endpoint to confirm the secret key is accepted before committing the switch.
  - Stops Coterie.
  - Creates a fresh `coterie.db` (sqlx migrations will run on first connection from the main coterie binary, or are explicitly run via a brief invocation — implementer's call).
  - Copies the admin row from `coterie-test.db` to `coterie.db` via `ATTACH DATABASE` + `INSERT SELECT` — operator doesn't need to re-enter the admin password.
  - Archives `coterie-test.db` to `coterie-test-archive-YYYYMMDD-HHMMSS.db` (so the test data is preserved as a record) or discards it with `--discard-test-db`.
  - Rewrites `.env` atomically (writes to `.env.new`, mv to `.env`): swap `STRIPE__*` keys, swap `DATABASE_URL`.
  - Removes `/opt/coterie/.env.live` after consuming it (don't leave live creds lying around in two files).
  - Starts Coterie.
  - Smoke-tests `GET http://127.0.0.1:8080/health` returns 200 (not /setup redirect, since the admin row was preserved).
  - Reminds the operator to verify the live-mode webhook endpoint is registered in the Stripe dashboard.
- **No Coterie code changes.** The main binary doesn't know about "test mode" vs "live mode" beyond what's in `.env`. The mode distinction is entirely a wizard/subcommand convention.
- **Documentation**: update `STRIPE-SETUP.md` to cover the test-mode workflow and reference the switchover subcommand.

## Capabilities

### New Capabilities
- `stripe-test-then-live`: a deployment pattern where Coterie starts in test mode against a separate database, the operator manually verifies Stripe wiring, then a single command swaps to live mode with a fresh production database and the existing admin preserved.

### Modified Capabilities
- `provisioning-wizard`: gains the test/live mode prompt and the verification-checklist output. Existing live-mode-only behavior is unchanged when the operator selects "live."

## Impact

- **Code**:
  - Modifications to the `coterie-provision` Rust crate from `a24`:
    - `install` subcommand: new mode prompt, conditional credential collection, alternate `DATABASE_URL` setting, optional live-credential storage in `.env.live`, post-wizard verification checklist.
    - New `switch-stripe-to-live` subcommand: ~500–800 LOC across the new module + tests, reusing `a24`'s trait abstractions (`SystemCommand`, `FileSystem`) and prefix validators.
  - Updates to `STRIPE-SETUP.md` for the test-mode walkthrough.
  - No changes to Coterie's runtime binary.
- **Wire shape**: zero runtime change. Same URLs, same handlers, same webhook routing. The only externally-observable change is which Stripe credentials are accepted (test sets work in test mode, live sets work in live mode — Stripe's existing behavior).
- **Tests**:
  - Pure-function modules (`.env` rewrite logic, archive-name generation, idempotency-state computation) covered by `cargo test`.
  - Integration tests drive `coterie-provision switch-stripe-to-live` end-to-end against `FakeSystem` and `FakeFs`, asserting on: command order (systemctl stop, sqlite operations, atomic rename, systemctl start), final filesystem state (.env contents, DB files present/absent), idempotency refusal paths.
  - Stripe `/v1/balance` smoke test is wrapped behind a `StripeApi` trait so tests use a fake.
  - VM-level end-to-end smoke (real test charges, then real switchover) is operator-side, documented in the PR template, not a task the autocoder claims to complete.
- **Risk**: low. The switchover's most dangerous step is the `.env` rewrite + DB swap; both happen with explicit atomic operations (write-to-new + rename) and idempotency checks prevent accidental double-switch.
- **Dependency**: depends on `a24` (the `coterie-provision` crate must exist) shipping first. Also depends on `a23` (`create_admin` binary) being present in the install, though the switchover doesn't directly invoke create_admin — it copies the existing admin row across DBs via SQL.
