## Context

Coterie has two binaries today: `coterie` (the server) and `seed` (data seeder for development). The `seed` binary is the established pattern for "a separate CLI that talks to the same DB as the server" — it uses clap, loads the same `Settings`, connects to the same SQLite file, and runs operations the operator wants to perform without going through the HTTP layer.

`create_admin` follows that pattern exactly. It's a third binary using the same shape: clap for args, `Settings::new()` for config, `SqlitePoolOptions::connect_with(SqliteConnectOptions::create_if_missing(true))` for the pool (matching the recent main.rs fix), `sqlx::migrate!()` to ensure schema, then a focused operation (insert admin row).

The security motivation is real and well-documented in the `routing-architecture` spec already — `/setup` is exempt from auth because there's no session to bind to before the first admin exists. That necessary exemption opens the hijack window. Closing it with a CLI bootstrap is the standard pattern in operator-deployed apps (Gitea has `gitea admin user create`, Discourse has `rake admin:create`, Mastodon has `tootctl accounts create`).

## Goals / Non-Goals

**Goals:**
- A standalone binary that can create the first admin without HTTP.
- Idempotency: re-running with an admin already present is a no-op with a clear exit code, not a duplicate insert.
- Same password-hashing path as the manual setup form — passwords created via CLI are interchangeable with passwords created via /setup, including for future rehashing on argon2 parameter bumps.
- GET /setup respects the "admin exists" state — redirects to /login instead of rendering a stale form.
- The wizard can rely on this binary as a stable seam.

**Non-Goals:**
- Creating non-admin members via CLI. The bulk-import flow exists for that; this binary is bootstrap-only.
- Subsequent admin promotion ("turn this existing user into an admin"). That's an admin-portal action, not a bootstrap concern.
- Resetting an admin's password from the CLI. Separate concern (`reset_admin_password` could be a future sibling binary if needed for disaster recovery).
- Removing the /setup form entirely. It's useful for the manual-deploy path, which stays supported.

## Decisions

### D1. Separate binary, not a subcommand of `coterie`

Pattern parity with `seed`. The `coterie` binary stays focused on "start the server"; auxiliary operations get their own binaries. Means the wizard's command line stays readable:

```
/opt/coterie/create_admin --email=... --username=... --full-name=... --password-file=/path/to/pw
```

…vs. the alternative of `/opt/coterie/coterie create-admin --email=...` which would require restructuring main.rs's argument parsing.

If the binary count grows beyond ~3-4, the calculus might flip toward a unified CLI. Today, 3 is fine.

### D2. Password via file, not env or arg

`--password "secret"` shows up in `ps aux` for anyone with the box's process list (typically any local user). `--password-file /tmp/pw` keeps the secret on disk where filesystem permissions can protect it. The wizard creates the file with `chmod 0600`, runs `create_admin`, then `shred -u`s the file.

We support `--password "..."` too for testing convenience, but the wizard uses `--password-file`. Exactly one must be specified; supplying both is a usage error.

### D3. Idempotency by refusal, not by upsert

Re-running with an admin already present exits with code 2 and a message — does NOT replace, update, or create-another. Rationale: the CLI's job is bootstrap. Upsert semantics here would be surprising (and dangerous: a typo'd email could create a second admin if upsert keyed on email, and key-conflict behavior depends on which field you treat as the natural key).

Operators who genuinely need to add a second admin do so through the portal's admin UI, where the path is intentional and auditable. Operators who need to "reset" an admin do so via password-reset email or a separate `reset_admin_password` binary if we ever build one.

### D4. Skip the verification-email send

The setup wizard's POST handler today doesn't send a verification email (the bootstrap admin is implicitly verified). create_admin matches this — sets `email_verified_at = NOW()` and doesn't trigger any email send. The wizard collects an email address that the operator controls; we trust it.

### D5. Run migrations as part of the command

create_admin's first DB operation runs `sqlx::migrate!()` to ensure the schema is in place. This means the binary works on a fresh DB (provisioning wizard scenario) AND on an existing DB (re-provision scenario). sqlx's migration runner is idempotent — already-applied migrations are skipped.

Without this, the wizard would have to start the coterie service once just to run migrations, then stop it, then create_admin, then start it again. The migrations-from-create_admin path lets the wizard create the admin BEFORE the service ever starts.

### D6. GET /setup redirects to /login when admin exists

Today the /setup route is reachable as a GET handler that renders the setup form regardless of whether an admin exists. The POST checks via `setup_lock` + `check_admin_exists` and refuses. But the GET still renders the form — confusing UX if an operator stumbles onto /setup after bootstrap (they'd see a form, fill it in, submit, get an error).

Change: GET /setup checks `check_admin_exists` (or consults `admin_exists_observed` from `AppState`). If true, return `Redirect::to("/login")`. The form is only rendered when bootstrap is actually pending.

This is mostly a UX fix; the security property already holds because POST refuses. But the GET behavior should match the POST's reality.

### D7. The wizard, not the binary, validates the inputs

The binary validates required args are present and that exactly one of `--password` / `--password-file` is specified. But it does NOT validate that the email looks like an email, or the password meets a strength minimum, or anything beyond "these are non-empty strings." The wizard's prompt logic is where input validation belongs — let the operator see input-time feedback, not after launching the binary.

The repo-level password validator (the one used by the signup endpoint, ensuring minimum length, etc.) is invoked by the binary as a hard floor — passwords below the technical minimum are refused even if the wizard somehow let them through. That's a backstop, not the primary validation surface.

## Risks / Trade-offs

- **Risk**: an attacker who can SSH to the box uses `create_admin` to make themselves the org admin. → They already have root on the box. They can do worse via SQL. Not a new attack vector.
- **Risk**: the wizard fails after create_admin runs but before the service starts — operator restarts wizard, get-prompted-for-password-again confusion. → Mitigation: design the wizard to detect "admin already exists" and skip the create step. The binary's refusal-with-clear-exit-code is the signal the wizard reads.
- **Trade-off**: another binary in the build. Marginal increase in CI time (the binary shares the cargo dep graph with coterie itself, so most compile work is already done). Acceptable.
- **Trade-off**: the password ends up in a temp file on the wizard's box. Mitigation: chmod 0600, shred after use. Better than `--password "..."` in process list.

## Migration Plan

Single PR.

1. Add `[[bin]] name = "create_admin" path = "src/bin/create_admin.rs"` to `Cargo.toml`.
2. Create `src/bin/create_admin.rs`:
   - Clap definition for the four args (`--email`, `--username`, `--full-name`, `--password` or `--password-file`).
   - `Settings::new()`, DB connect with `create_if_missing(true)`, run migrations.
   - Idempotency check via `SELECT 1 FROM members WHERE is_admin = 1 LIMIT 1`.
   - Hash password using existing `AuthService` helper (extract a `hash_password` free function from AuthService if not already public; reuse the same argon2 parameters).
   - Insert via existing `MemberService::create` machinery if convenient, or a direct SQL insert (probably direct — we want to set `is_admin`, `status`, `email_verified_at` in one row with no side effects).
3. Modify `src/web/templates/setup.rs` GET handler to redirect to `/login` if `check_admin_exists(&state).await` returns true (or consult `state.admin_exists_observed`).
4. Add tests:
   - Happy path: empty DB, run create_admin, assert one admin row with the expected fields and a verifiable password hash.
   - Refuse-on-existing: pre-seed an admin, run create_admin, assert exit code 2 and no second insert.
   - Password-via-file: write password to tmpfile, pass via `--password-file`, assert the resulting hash verifies against that password.
   - GET /setup redirects when admin exists.
5. `cargo build --bins --release` + `cargo test --features test-utils`.
