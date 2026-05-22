## Context

Stripe maintains two completely isolated modes: test (`pk_test_*` / `sk_test_*`) and live (`pk_live_*` / `sk_live_*`). Each has its own webhook configurations with their own signing secrets (`whsec_*`). A single URL can be registered as a webhook endpoint in BOTH modes simultaneously — Stripe routes events by which mode generated them, and Coterie verifies the signature against whatever `whsec` is currently in `.env`.

This isolation makes "test then switch to live" clean: only one credential triple is active at a time in Coterie's `.env`. Switching modes means swapping the three values atomically. Coterie itself doesn't need a "test mode" concept; it just reads whatever credentials are configured.

The new piece this change introduces is the **separate database file during test mode**. Without it, test charges accumulate in the same DB that will eventually serve live customers. Separating means:
- Test data (members, payments, audit rows) lives in `coterie-test.db`
- When switching to live, that DB is discarded (or archived) and a fresh `coterie.db` starts from migrations
- The admin row is copied across so the operator's identity persists

This is the cleanest possible "verify, then go live" workflow: the operator runs real flows through real Stripe (in test mode), sees results in the test-mode dashboard, then a single command transitions to production with no test artifacts polluting the live data.

## Goals / Non-Goals

**Goals:**
- Wizard supports test mode as an alternative to live mode at provision time.
- Test mode uses a separate database file (`coterie-test.db`).
- Switchover script transitions to live mode in one shot: stops service, swaps creds + DB, preserves admin, restarts.
- Operator never re-enters the admin password (admin row migrates across DBs).
- Operator never has to manually edit `.env`.
- Switchover is idempotent: running twice on a live instance refuses to do anything.

**Non-Goals:**
- Switching FROM live back TO test. The migration is one-way by design — if an operator needs a test instance after going live, they provision a new instance.
- Coterie code changes. The mode distinction is entirely a wizard/script convention; Coterie just reads `.env`.
- Automated verification of test scenarios (creating synthetic charges, asserting outcomes). The verification is manual — operator runs real flows through the UI. Automated scenarios are deferred to a future change (the `coterie verify-stripe` subcommand sketched earlier).
- Migrating non-admin data from test DB to live DB. The live DB starts empty by intent.
- Multi-mode deployments (running test and live simultaneously on the same box). Single mode at a time.

## Decisions

### D1. Six credentials total, but collected in stages

The wizard collects TEST credentials (when test mode is chosen). It optionally also collects LIVE credentials if the operator has them ready — those get stashed in `/opt/coterie/.env.live` (chmod 0640) for the switchover script to read. If the operator doesn't have live creds yet, the switchover script prompts at the time it's run.

This deferred-collection model means the wizard doesn't force operators to have live Stripe keys at provision time (which is common in real-world workflows where the org gets test keys first and applies for live API access later). It also limits the time window during which live credentials sit on disk in plaintext (only if the operator pre-loads them via the wizard).

### D2. Separate DB file, not test-data tagging

Considered: add an `is_test` column to relevant tables, run test charges in the live DB but tag them, sweep at switchover. Rejected because:
- Schema changes required (multiple tables: members, payments, audit_logs at minimum)
- Query filters needed everywhere ("don't show test rows in production UI") — code churn
- "Sweep test rows on switchover" has subtle correctness issues (what about audit rows referencing deleted test members?)

A separate DB file sidesteps all of that. Coterie's code stays unchanged; the mode distinction is invisible to it. The cost is one extra `.env` edit during switchover (the DATABASE_URL line).

### D3. Admin row migration via ATTACH DATABASE

The cleanest way to preserve the admin without re-prompting the password: use SQLite's `ATTACH DATABASE` to treat `coterie-test.db` as a second schema, then INSERT the admin row into the new `coterie.db`:

```sql
ATTACH DATABASE '/var/lib/coterie/coterie-test.db' AS test;
INSERT INTO members SELECT * FROM test.members WHERE is_admin = 1;
DETACH DATABASE test;
```

This copies the row verbatim, including the hashed password. Operator's existing credentials work against the new DB. Schemas are guaranteed to match because both DBs ran the same migrations from the same Coterie version.

If the test DB has multiple admin rows (unlikely but possible if the operator manually created additional admins via the UI during testing), all of them migrate. The switchover doesn't try to filter — operators who want a clean slate can manually delete extras after.

### D4. Idempotency via two refusal paths

`switch-stripe-to-live.sh` refuses to run in two scenarios:

1. **`.env` already has `pk_live_*`** — instance is already in live mode; switchover is meaningless or destructive.
2. **`coterie-test.db` doesn't exist** — instance was never in test mode, so there's nothing to migrate from.

Both checks happen at the top of the script before any modifications. If either fires, the script exits non-zero with a clear message.

### D5. Test DB lifecycle: archive by default

The switchover script renames `coterie-test.db` to `coterie-test-archive-YYYYMMDD-HHMMSS.db` rather than deleting. Operators may want to retain it for compliance, debugging, or "what did we test against" records.

Operators who don't want the archive can `rm` it after the switchover or pass `--discard-test-db` to the script.

### D6. .env rewrite is atomic

The switchover script doesn't edit `.env` in place. It:

1. Reads the current `.env` into memory (or a temp file).
2. Builds the new `.env` content with substitutions: test creds → live creds, `coterie-test.db` → `coterie.db`.
3. Writes to `/opt/coterie/.env.new` with the right permissions.
4. `mv /opt/coterie/.env.new /opt/coterie/.env` — atomic rename.

This means a partial failure mid-write doesn't leave `.env` corrupt. The service is still stopped at this point, so worst case we have a stale-but-valid `.env` and the operator can retry.

### D7. Stripe `/v1/balance` smoke test before committing

Optional but useful: before the `.env` rewrite, the script calls `https://api.stripe.com/v1/balance` with the LIVE secret key as basic auth. If Stripe returns 200, the key is valid. If 401/403, the key is wrong or revoked; abort before doing anything destructive.

This catches the common "operator pasted the wrong secret key" error before it becomes a service-down incident.

```sh
curl -sf -u "$LIVE_SK:" https://api.stripe.com/v1/balance > /dev/null
if [ $? -ne 0 ]; then
    die "Live secret key was rejected by Stripe. Aborting switchover."
fi
```

### D8. The wizard's test-mode verification checklist

After the wizard completes in test mode, print:

```
Coterie is running in Stripe TEST mode with separate test database
(/var/lib/coterie/coterie-test.db). Use this time to verify Stripe
wiring before switching to live.

Test card to use: 4242 4242 4242 4242, any future expiry, any 3-digit
CVC, any ZIP.

Suggested verification steps:

  [ ] Sign up a test member via your public site or directly via
      Coterie's signup form (if exposed).
  [ ] Make a test donation through /portal/donate (logged in as
      admin) or via the public donate flow.
  [ ] Confirm each test charge appears in your Stripe dashboard's
      TEST MODE payments view.
  [ ] Confirm `journalctl -u coterie` shows the webhook events
      arriving cleanly (look for "Webhook event received").
  [ ] Confirm the receipt email arrived at the address you used.

When satisfied, switch to live mode:

  sudo bash /opt/coterie/deploy/switch-stripe-to-live.sh

This will: stop Coterie, archive coterie-test.db, create a fresh
coterie.db, copy your admin row across, prompt for (or load) your
live Stripe credentials, rewrite .env, and start Coterie back up.
```

This is the documentation the operator needs at exactly the moment they need it. No need to flip back to a docs site.

### D9. Webhook endpoint reminder at switchover

Live and test modes have SEPARATE webhook configurations in the Stripe dashboard. If the operator only registered the test-mode webhook during the wizard, switching to live mode means Stripe has no webhook configured for live events — every live charge would happen but Coterie would never hear about it.

The switchover script prints a clear reminder at the end:

```
============================================================
Switched to Stripe LIVE mode.

IMPORTANT: verify the LIVE-mode webhook endpoint is registered in
your Stripe dashboard:

  Stripe dashboard → toggle to LIVE mode → Developers → Webhooks
  → confirm an endpoint exists for:
       https://coterie.example.com/api/payments/webhook/stripe
  → confirm the signing secret matches the whsec_ value you just
       supplied to this script

Without a live-mode webhook registered, real charges will go through
Stripe but Coterie will never hear about them — dues will never
extend, payments will never advance from Pending.
============================================================
```

## Risks / Trade-offs

- **Risk**: operator forgets to register the live-mode webhook in Stripe's live-mode dashboard. → Mitigation: D9 reminder + the wizard's STRIPE-SETUP.md cross-link. Could be made more robust by having the switchover script make a Stripe API call to LIST registered webhooks and warn if none point at this URL — but that's complexity for a future iteration.
- **Risk**: `.env.live` sitting on disk is a credential leak vector. → Mitigation: chmod 0640, owned by `coterie` (same as `.env`). If the operator chooses to defer providing live creds until switchover time, no `.env.live` is created. Best practice is to not pre-load if there's any doubt.
- **Risk**: archived test DBs accumulate over time. → Acceptable: operators clean these up manually if they care. They're small files (no traffic happened in test mode at any real scale).
- **Trade-off**: the switchover is one-way. To go back to test mode after going live, the operator provisions a new instance. Acceptable for the workflows we're designing for; reverse-switching would add complexity for a use case nobody's asked for.
- **Trade-off**: another bash script to maintain alongside `provision.sh` and `release-deploy.sh`. The deploy/ directory's shell-script surface is growing. Could be consolidated into a unified `coterie-deploy` Rust binary eventually, but that's a bigger project.

## Migration Plan

Single PR. Depends on `a24` shipping first.

1. Add the "test mode / live mode" prompt to `deploy/provision.sh`. Default: live (matches `a24`'s current behavior).
2. If test mode chosen: collect test credentials, set `DATABASE_URL` to point at `coterie-test.db`, optionally collect live credentials and write to `.env.live`.
3. After the wizard completes in test mode, print the verification checklist (per D8).
4. Write `deploy/switch-stripe-to-live.sh`:
   - Argument: `--discard-test-db` (optional, default is archive).
   - Idempotency checks (D4).
   - Load live creds from `.env.live` or prompt.
   - Validate creds shapes + optional Stripe API smoke test (D7).
   - Stop coterie.
   - SQL admin migration (D3).
   - Archive coterie-test.db (or discard if flag set).
   - Atomic .env rewrite (D6).
   - Start coterie.
   - Smoke test + print webhook reminder.
5. Update `STRIPE-SETUP.md` to describe the test-mode workflow and reference the switchover script.
6. Test on a fresh Debian VM: wizard in test mode, do a test donation, switch to live, confirm /portal still works and admin login still works.
