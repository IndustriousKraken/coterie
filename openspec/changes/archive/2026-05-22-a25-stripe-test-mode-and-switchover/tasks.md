## 1. Extend the wizard with test/live mode

- [x] 1.1 In `deploy/provision.sh` (after the existing Stripe-enable prompt), add a new prompt: "Stripe mode: [test/live]?" with default `live`. Env-var override: `COTERIE_PROVISION_STRIPE_MODE=test|live`.
- [x] 1.2 If test mode: prompt for `pk_test_*`, `sk_test_*`, test `whsec_*`. Validate each starts with the expected prefix; refuse to continue if any prefix is wrong.
- [x] 1.3 If test mode: prompt "Do you also have live credentials to pre-load for later switchover?" with default `no`. If yes, prompt for `pk_live_*`, `sk_live_*`, live `whsec_*`. Validate prefixes.
- [x] 1.4 If test mode: set `COTERIE__DATABASE__URL=sqlite:///var/lib/coterie/coterie-test.db?mode=rwc` in the generated `.env`.
- [x] 1.5 If test mode AND live creds were pre-loaded: write them to `/opt/coterie/.env.live` (chmod 0640, owned by `coterie`). Format: same `COTERIE__STRIPE__*=...` lines a regular `.env` would have, but only the Stripe-live fields.

## 2. Test-mode verification checklist output

- [x] 2.1 After the wizard completes in test mode (just before the final summary), print the verification checklist described in design.md D8. Include: test card number, suggested verification flows, the command to switch to live mode.
- [x] 2.2 In live mode, the checklist is NOT printed (matches `a24` baseline output).

## 3. Write switch-stripe-to-live.sh

- [x] 3.1 Create `deploy/switch-stripe-to-live.sh` with `#!/usr/bin/env bash`, `set -euo pipefail`, and an ERR trap.
- [x] 3.2 Parse args: `--discard-test-db` (default is archive), `--yes` (skip confirmation), `--help`.
- [x] 3.3 Refuse if `$EUID -ne 0`.
- [x] 3.4 Idempotency check #1: read `/opt/coterie/.env`. If it contains `pk_live_`, exit 0 with "Already in live mode; nothing to do."
- [x] 3.5 Idempotency check #2: if `/var/lib/coterie/coterie-test.db` doesn't exist, exit non-zero with "Not in test mode; no test DB to migrate from."
- [x] 3.6 Load live credentials: if `/opt/coterie/.env.live` exists, source it. Otherwise prompt the operator interactively for `pk_live_*`, `sk_live_*`, `whsec_*` (twice for the secrets, with masking).
- [x] 3.7 Validate prefixes (`pk_live_`, `sk_live_`, `whsec_`); refuse if any mismatch.
- [x] 3.8 Optional API check: `curl -sf -u "$LIVE_SK:" https://api.stripe.com/v1/balance > /dev/null`. If non-zero exit, abort with "Stripe rejected the live secret key — aborting before any modifications."
- [x] 3.9 Confirmation prompt (unless `--yes`): show what's about to happen — service stop, DB swap (archive vs discard per flag), .env rewrite, service restart. Get y/N.

## 4. Service stop + DB migration

- [x] 4.1 `systemctl stop coterie`. If this fails, abort — something's wrong with the service config.
- [x] 4.2 Migrations setup: invoke `/opt/coterie/coterie --help` is not the right move (coterie is a server, no --help mode). Instead, briefly start coterie with `COTERIE__DATABASE__URL=sqlite:///var/lib/coterie/coterie.db?mode=rwc` set in the environment, wait for it to log "Server listening" (or just sleep 3 seconds), then stop it. This runs the migrations against the new DB. Alternative: use `sqlite3 /var/lib/coterie/coterie.db < migrations/*.sql` directly. The cleanest approach: invoke `/opt/coterie/create_admin --help` with the new DB URL — if create_admin runs migrations as part of its startup (per `a23`), this populates the schema without actually creating an admin.

  (NOTE during implementation: pick whichever method is cleanest. The simplest is to copy the entire test DB schema rather than relying on Coterie's startup to run migrations. A `sqlite3 -cmd ".schema" coterie-test.db | sqlite3 coterie.db` could replicate the schema, but it's safer to actually run the migrations from a known-good source. Detail to figure out at implementation time.)
- [x] 4.3 ATTACH the test DB and INSERT admin rows:
  ```sql
  ATTACH DATABASE '/var/lib/coterie/coterie-test.db' AS test;
  INSERT INTO members SELECT * FROM test.members WHERE is_admin = 1;
  DETACH DATABASE test;
  ```
  Use `sqlite3 /var/lib/coterie/coterie.db <<SQL ... SQL` heredoc.
- [x] 4.4 Archive or discard the test DB:
  - Default: `mv coterie-test.db coterie-test-archive-$(date +%Y%m%d-%H%M%S).db`
  - With `--discard-test-db`: `rm coterie-test.db`

## 5. .env rewrite

- [x] 5.1 Read `/opt/coterie/.env` into a variable.
- [x] 5.2 Substitute the Stripe credential lines: `COTERIE__STRIPE__PUBLISHABLE_KEY=pk_test_…` → `=pk_live_…`. Same for SECRET_KEY and WEBHOOK_SECRET.
- [x] 5.3 Substitute the DATABASE_URL line: `coterie-test.db` → `coterie.db`.
- [x] 5.4 Write the new content to `/opt/coterie/.env.new` with permissions 0640, ownership `coterie:coterie`.
- [x] 5.5 `mv /opt/coterie/.env.new /opt/coterie/.env` (atomic rename).
- [x] 5.6 If `/opt/coterie/.env.live` exists, archive or discard it (it served its purpose; living plaintext live creds are a security risk to leave around). Default: `rm /opt/coterie/.env.live`.

## 6. Service start + smoke test

- [x] 6.1 `systemctl start coterie`. Wait for it to come up (poll `systemctl is-active coterie` up to 30 seconds).
- [x] 6.2 If service doesn't reach `active` state, dump `journalctl -u coterie -n 50 --no-pager` and exit non-zero. The .env is now live mode but the service isn't running — operator needs to investigate, but no rollback path (the test DB is archived).
- [x] 6.3 `curl -sf http://127.0.0.1:8080/health` — expect 200 with JSON. If the response is a 303 to /setup, something went wrong with admin migration; dump diagnostics.

## 7. Final summary + webhook reminder

- [x] 7.1 Print the success summary including the webhook-registration reminder per design.md D9.

## 8. STRIPE-SETUP.md updates

- [x] 8.1 Add a new section to `deploy/STRIPE-SETUP.md` describing the test-mode-first workflow: how to enable in the wizard, the verification flows to run, when to use `switch-stripe-to-live.sh`.
- [x] 8.2 Note that live and test modes have SEPARATE webhook configurations in the Stripe dashboard — both need to be registered if both modes will be used (even sequentially).

## 9. Tests / validation

- [x] 9.1 `bash -n deploy/switch-stripe-to-live.sh` — syntax check.
- [x] 9.2 `shellcheck` both `provision.sh` (revised) and `switch-stripe-to-live.sh`; address warnings.
- [x] 9.3 Manual smoke on a fresh Debian VM:
   - Run the wizard in test mode with test Stripe creds.
   - Verify `/var/lib/coterie/coterie-test.db` exists; `coterie.db` does not.
   - Make a test donation via the portal using `4242 4242 4242 4242`.
   - Confirm the test charge appears in Stripe's test-mode dashboard.
   - Confirm Coterie's logs show the webhook event.
   - Run `switch-stripe-to-live.sh`.
   - Confirm `.env` now references `coterie.db` and `pk_live_*`.
   - Log in with the admin credentials from the test-mode wizard run — should succeed against the new live DB.
   - Make a small live test charge to confirm the live wiring works.
- [x] 9.4 Test idempotency: try to run `switch-stripe-to-live.sh` again after the first switch. Should exit 0 with "already in live mode."

## 10. Verify the spec

- [x] 10.1 Confirm the delta specs match the implemented behavior.
