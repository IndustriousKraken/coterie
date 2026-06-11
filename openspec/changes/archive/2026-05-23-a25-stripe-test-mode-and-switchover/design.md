## Context

Stripe maintains two completely isolated modes: test (`pk_test_*` / `sk_test_*`) and live (`pk_live_*` / `sk_live_*`). Each has its own webhook configurations with their own signing secrets (`whsec_*`). A single URL can be registered as a webhook endpoint in BOTH modes simultaneously — Stripe routes events by which mode generated them, and Coterie verifies the signature against whatever `whsec` is currently in `.env`.

This isolation makes "test then switch to live" clean: only one credential triple is active at a time in Coterie's `.env`. Switching modes means swapping the three values atomically. Coterie itself doesn't need a "test mode" concept; it just reads whatever credentials are configured.

The new piece this change introduces is the **separate database file during test mode**. Without it, test charges accumulate in the same DB that will eventually serve live customers. Separating means:
- Test data (members, payments, audit rows) lives in `coterie-test.db`
- When switching to live, that DB is archived and a fresh `coterie.db` starts from migrations
- The admin row is copied across so the operator's identity persists

This is the cleanest possible "verify, then go live" workflow: the operator runs real flows through real Stripe (in test mode), sees results in the test-mode dashboard, then a single command transitions to production with no test artifacts polluting the live data.

The implementation lives in the `coterie-provision` Rust crate introduced by `a24`. Adding a second subcommand to that crate (alongside `install`) keeps all deploy-lifecycle tooling in one binary and reuses `a24`'s trait abstractions and prefix validators.

## Goals / Non-Goals

**Goals:**
- Wizard supports test mode as an alternative to live mode at provision time.
- Test mode uses a separate database file (`coterie-test.db`).
- Switchover subcommand transitions to live mode in one shot: stops service, swaps creds + DB, preserves admin, restarts.
- Operator never re-enters the admin password (admin row migrates across DBs).
- Operator never has to manually edit `.env`.
- Switchover is idempotent: running twice on a live instance refuses to do anything.
- Switchover logic is fully testable in the autocoder sandbox via `cargo test` against fake trait implementations.

**Non-Goals:**
- Switching FROM live back TO test. The migration is one-way by design — if an operator needs a test instance after going live, they provision a new instance.
- Coterie runtime code changes. The mode distinction is entirely a wizard/subcommand convention; Coterie just reads `.env`.
- Automated verification of test scenarios (creating synthetic charges, asserting outcomes). The verification is manual — operator runs real flows through the UI.
- Migrating non-admin data from test DB to live DB. The live DB starts empty by intent.
- Multi-mode deployments (running test and live simultaneously on the same box). Single mode at a time.

## Decisions

### D1. Six credentials total, but collected in stages

The wizard collects TEST credentials (when test mode is chosen). It optionally also collects LIVE credentials if the operator has them ready — those get stashed in `/opt/coterie/.env.live` (chmod 0640) for the switchover subcommand to read. If the operator doesn't have live creds yet, the switchover subcommand prompts at the time it's run.

This deferred-collection model means the wizard doesn't force operators to have live Stripe keys at provision time. It also limits the time window during which live credentials sit on disk in plaintext (only if the operator pre-loads them via the wizard).

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

The Rust implementation uses `rusqlite` (or shells out to the `sqlite3` CLI — implementer's call, with a slight preference for `rusqlite` for better error handling). Either way, the SQL above is the contract.

This copies the row verbatim, including the hashed password. Operator's existing credentials work against the new DB. Schemas are guaranteed to match because both DBs ran the same migrations from the same Coterie version.

If the test DB has multiple admin rows (unlikely but possible if the operator manually created additional admins via the UI during testing), all of them migrate. The switchover doesn't try to filter — operators who want a clean slate can manually delete extras after.

### D4. Idempotency via two refusal paths

`switch-stripe-to-live` refuses to run in two scenarios:

1. **`.env` already has `pk_live_*`** — instance is already in live mode; switchover is meaningless or destructive.
2. **`coterie-test.db` doesn't exist** — instance was never in test mode, so there's nothing to migrate from.

Both checks happen at the top of the subcommand before any modifications. If either fires, the subcommand exits non-zero with a clear message.

### D5. Test DB lifecycle: archive by default

The switchover subcommand renames `coterie-test.db` to `coterie-test-archive-YYYYMMDD-HHMMSS.db` rather than deleting. Operators may want to retain it for compliance, debugging, or "what did we test against" records.

Operators who don't want the archive can pass `--discard-test-db`.

### D6. .env rewrite is atomic

The switchover doesn't edit `.env` in place. It:

1. Reads the current `.env` into memory.
2. Builds the new `.env` content with substitutions: test creds → live creds, `coterie-test.db` → `coterie.db`. The substitution logic lives in a pure function `pub fn rewrite_env(current: &str, live_creds: &LiveCreds) -> String` with golden tests.
3. Writes to `/opt/coterie/.env.new` via the `FileSystem` trait with mode 0640.
4. `rename` `/opt/coterie/.env.new` → `/opt/coterie/.env` (atomic on the same filesystem).

This means a partial failure mid-write doesn't leave `.env` corrupt. The service is still stopped at this point, so worst case we have a stale-but-valid `.env` and the operator can retry.

After the rewrite succeeds, the subcommand removes `/opt/coterie/.env.live` if it existed (don't leave live creds in two places).

### D7. Stripe `/v1/balance` smoke test before committing

Before the destructive operations, the subcommand calls `https://api.stripe.com/v1/balance` with the LIVE secret key as basic auth. If Stripe returns 200, the key is valid. If 401/403, the key is wrong or revoked; abort before doing anything destructive.

This catches the common "operator pasted the wrong secret key" error before it becomes a service-down incident.

```rust
pub trait StripeApi {
    fn check_balance(&self, secret_key: &Secret<String>) -> Result<()>;
}

pub struct RealStripeApi {
    client: reqwest::blocking::Client,
}

impl StripeApi for RealStripeApi {
    fn check_balance(&self, secret_key: &Secret<String>) -> Result<()> {
        let response = self.client
            .get("https://api.stripe.com/v1/balance")
            .basic_auth(secret_key.expose_secret(), Some(""))
            .send()?
            .error_for_status()?;
        Ok(())
    }
}
```

Tests use a `FakeStripeApi` that returns canned success/failure responses based on the secret key contents.

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

  sudo coterie-provision switch-stripe-to-live

This will: stop Coterie, archive coterie-test.db, create a fresh
coterie.db, copy your admin row across, prompt for (or load) your
live Stripe credentials, rewrite .env, and start Coterie back up.
```

The text lives in a `const` string in the wizard. Test asserts the exact text gets printed when test mode is selected.

### D9. Webhook endpoint reminder at switchover

Live and test modes have SEPARATE webhook configurations in the Stripe dashboard. If the operator only registered the test-mode webhook during the wizard, switching to live mode means Stripe has no webhook configured for live events — every live charge would happen but Coterie would never hear about it.

The switchover prints a clear reminder at the end:

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

### D10. Testability — reuse a24's trait scaffolding

The switchover subcommand uses the same `SystemCommand`, `FileSystem`, and (new) `StripeApi` traits introduced by `a24`. Integration tests in `tests/switch_to_live.rs` drive the full subcommand against fakes and assert on:

- Refusal paths (already-live, no-test-db) exit non-zero without touching anything.
- Happy path: expected commands in expected order (`systemctl stop`, sqlite operations, `systemctl start`), correct filesystem mutations (.env contents per golden snapshot, archive file present, .env.live removed).
- Stripe API smoke test failure aborts before service stop.
- The DB migration SQL is correctly emitted.

The `cargo test` suite is the autocoder's validation gate. VM-level smoke (real test charges, real switchover) is operator-side.

## Risks / Trade-offs

- **Risk**: operator forgets to register the live-mode webhook in Stripe's live-mode dashboard. → Mitigation: D9 reminder + the wizard's STRIPE-SETUP.md cross-link. Could be made more robust by having the switchover call Stripe's API to LIST registered webhooks and warn if none point at this URL — but that's complexity for a future iteration.
- **Risk**: `.env.live` sitting on disk is a credential leak vector. → Mitigation: chmod 0640, owned by `coterie` (same as `.env`). If the operator chooses to defer providing live creds until switchover time, no `.env.live` is created. The file is removed immediately after consumption.
- **Risk**: archived test DBs accumulate over time. → Acceptable: operators clean these up manually if they care. They're small files (no traffic happened in test mode at any real scale).
- **Risk**: the SQL migration assumes both DBs have identical schemas. If the operator runs the switchover after upgrading Coterie mid-test-phase (new migrations applied to coterie-test.db but coterie.db is fresh and gets the same migrations on first connection), this still works. But if the schemas drift for any reason, INSERT SELECT will fail loudly. → Mitigation: the switchover runs migrations on coterie.db BEFORE the INSERT, so both DBs share the same schema baseline.
- **Trade-off**: the switchover is one-way. To go back to test mode after going live, the operator provisions a new instance. Acceptable for the workflows we're designing for.

## Migration Plan

Single PR. Depends on `a24` shipping first (the `coterie-provision` binary must exist).

1. Add the "test mode / live mode" prompt to the `install` subcommand in the `coterie-provision` crate. Default: live (matches `a24`'s current behavior).
2. If test mode chosen: collect test credentials, set `DATABASE_URL` to point at `coterie-test.db`, optionally collect live credentials and write to `.env.live` via the `FileSystem` trait.
3. After the wizard completes in test mode, print the verification checklist (per D8).
4. Implement the `switch-stripe-to-live` subcommand in a new module `src/switch_to_live.rs`:
   - CLI flag: `--discard-test-db` (default is archive), `--yes` (skip confirmation), `--no-prompt` (require env/flags only).
   - Idempotency checks (D4).
   - Load live creds from `.env.live` (parsed via a small helper) or prompt.
   - Validate creds prefixes (reuse `a24`'s `validate_prefix`).
   - Stripe API smoke test via `StripeApi` trait (D7).
   - Stop coterie via `SystemCommand`.
   - Create fresh `coterie.db`, run migrations (either by briefly invoking the coterie binary with the new DB URL, or by running the sqlx migration SQL directly via `rusqlite`).
   - SQL admin migration (D3) via `rusqlite` or `sqlite3` subprocess.
   - Archive coterie-test.db (or discard if flag set) via `FileSystem`.
   - Atomic .env rewrite (D6).
   - Remove .env.live if it existed.
   - Start coterie via `SystemCommand`.
   - Smoke test + print webhook reminder.
5. Add the new `StripeApi` trait (+ Real and Fake implementations) to the crate.
6. Update `STRIPE-SETUP.md` to describe the test-mode workflow and reference the switchover subcommand.
7. `cargo test`, `cargo clippy --deny warnings`, `cargo fmt --check` — autocoder validation gates.

## Operator handoff (PR description content, post-autocoder)

The autocoder cannot make real Stripe charges or run the switchover against a live VM. Before merging, the operator runs these manually on a fresh Debian 13 VM:

- Run the wizard in test mode with test Stripe credentials. Confirm `/var/lib/coterie/coterie-test.db` exists; `coterie.db` does not. Make a test donation via the portal using `4242 4242 4242 4242`. Confirm the charge appears in Stripe's test-mode dashboard and Coterie's logs show the webhook event.
- Run `coterie-provision switch-stripe-to-live`. Confirm `.env` now references `coterie.db` and `pk_live_*`. Log in with the admin credentials from the test-mode run — should authenticate against the new live DB. Make a small live test charge to confirm the live wiring works.
- Run `coterie-provision switch-stripe-to-live` a second time. Should exit 0 with "already in live mode" — verifies idempotency.

If any of those fail, the PR doesn't merge.
