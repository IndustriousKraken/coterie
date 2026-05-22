## Why

`a24` ships a provisioning wizard that takes a fresh Debian box from clean to "Coterie running with Stripe live keys." That works for operators who've already verified Stripe wiring elsewhere (staging, dev, the team's prior experience). It DOESN'T address the operator who wants to verify-before-committing — the "anxious admin" deploying for the first time, the new-build verification pass before a major release, the disaster-recovery operator who restored from backup and wants to confirm everything works before flipping back to live.

The fix is to let the wizard optionally start in **Stripe test mode** with a **separate database file** (`coterie-test.db`), then provide a **one-shot switchover script** that transitions to live mode cleanly: swap credentials, swap to a fresh live DB, migrate the admin row across, restart.

Several use cases this enables:

1. **First-deploy verification**: operator runs a few test charges through real Stripe (using `4242 4242 4242 4242`), watches them appear in the test-mode dashboard, sees the audit trail in Coterie, then switches to live mode confident the wiring works.
2. **Permanent test instance**: operator deploys `test.coterie.example.com` and never runs the switchover — the box stays in test mode forever, useful for testing new builds without risk.
3. **Major-version verification**: when a release touches payment code, spin up a fresh instance with the new build, verify against test-mode Stripe, then upgrade prod.
4. **Disaster-recovery verification**: restored from backup → start in test mode → verify Stripe still works against the restored data → switch back to live.

The model rests on the fact that Stripe's test and live modes are fully isolated: separate credential triples (pk + sk + webhook secret), separate dashboard configurations, separate webhook endpoints (each with their own signing secret). The wizard collects test credentials initially; the switchover collects live credentials at the moment they're needed.

## What Changes

- **Extend `a24`'s wizard** with a "test mode or live mode?" prompt (default: live, matches `a24`'s shipped behavior). If test:
  - Collect Stripe TEST credentials (`pk_test_*`, `sk_test_*`, test webhook signing secret).
  - Set `DATABASE_URL` in `.env` to point at `coterie-test.db` instead of `coterie.db`.
  - Print a verification checklist after the wizard finishes (which flows to test manually, how to confirm each succeeded).
  - Optionally collect LIVE credentials too if the operator has them ready — store in `/opt/coterie/.env.live` (chmod 0640, owned by `coterie`) for the switchover script to pick up later without re-prompting.
- **New script `deploy/switch-stripe-to-live.sh`** that:
  - Refuses to run if `.env` already shows `pk_live_*` (idempotent — can't double-switch).
  - Refuses to run if `/opt/coterie/coterie-test.db` doesn't exist (operator must be in test mode currently).
  - Loads live credentials from `/opt/coterie/.env.live` if present; otherwise prompts for them.
  - Validates each credential matches the expected shape (`pk_live_*`, `sk_live_*`, `whsec_*`).
  - Optionally calls Stripe's `/v1/balance` endpoint to confirm the secret key is accepted before committing the switch.
  - Stops Coterie.
  - Creates a fresh `coterie.db` and runs migrations.
  - Copies the admin row from `coterie-test.db` to `coterie.db` via `ATTACH DATABASE` + `INSERT SELECT` — operator doesn't need to re-enter the admin password.
  - Optionally archives `coterie-test.db` to `coterie-test-archive-YYYYMMDD.db` (so the test data is preserved as a record if anyone needs it later) or discards it.
  - Rewrites `.env` atomically (writes to `.env.new`, mv to `.env`): swap `STRIPE__*` keys, swap `DATABASE_URL`.
  - Starts Coterie.
  - Smoke-tests `curl http://127.0.0.1:8080/health` returns 200 (not /setup redirect, since the admin row was preserved).
  - Reminds the operator to verify the live-mode webhook endpoint is registered in the Stripe dashboard.
- **No Coterie code changes.** The binary doesn't know about "test mode" vs "live mode" beyond what's in `.env`. The mode distinction is entirely a wizard/script convention.
- **Documentation**: update `STRIPE-SETUP.md` to cover the test-mode workflow and reference the switchover script.

## Capabilities

### New Capabilities
- `stripe-test-then-live`: a deployment pattern where Coterie starts in test mode against a separate database, the operator manually verifies Stripe wiring, then a single command swaps to live mode with a fresh production database and the existing admin preserved.

### Modified Capabilities
- `provisioning-wizard`: gains the test/live mode prompt and the verification-checklist output. Existing live-mode-only behavior is unchanged when the operator selects "live."

## Impact

- **Code**:
  - Modifications to `deploy/provision.sh` (the wizard from `a24`): new mode prompt, conditional credential collection, alternate `DATABASE_URL` setting, optional live-credential storage in `.env.live`, post-wizard verification checklist.
  - New `deploy/switch-stripe-to-live.sh` (~150-200 lines).
  - Updates to `STRIPE-SETUP.md` for the test-mode walkthrough.
  - No changes to Coterie's binary or any of the runtime code.
- **Wire shape**: zero runtime change. Same URLs, same handlers, same webhook routing. The only externally-observable change is which Stripe credentials are accepted (test sets work in test mode, live sets work in live mode — Stripe's existing behavior).
- **Tests**: the wizard and switchover script are bash; testing strategy is `--dry-run` + manual smoke on a fresh Debian box. The credential-swap logic in the switchover script can be unit-tested via shellcheck + bash test framework (`bats`) for the parsing pieces, but most validation is end-to-end.
- **Risk**: low. The switchover script's most dangerous step is the .env rewrite + DB swap; both happen with explicit atomic operations (write-to-new + rename) and idempotency checks prevent accidental double-switch.
- **Dependency**: depends on `a24` (the wizard) being shipped first. The switchover script ALSO depends on `a23` (`create_admin` binary) being present in the install, though it doesn't directly invoke create_admin — it copies the existing admin row across DBs via SQL.
