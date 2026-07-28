# The first-boot setup gate fails OPEN on a database error

## What is wrong

`POST /setup` is unauthenticated and CSRF-exempt. Its only gate is the
"has an admin already been created?" check. That check returns
**"no admin exists"** when the database query errors:

`src/web/templates/setup.rs:279-293`

```rust
async fn check_admin_exists(db_pool: &SqlitePool) -> bool {
    let result: Result<Option<(i64,)>, _> =
        sqlx::query_as("SELECT 1 as exists_flag FROM members WHERE is_admin = 1 LIMIT 1")
            .fetch_optional(db_pool)
            .await;

    match result {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(e) => {
            tracing::error!("Failed to check for admin: {}", e);
            false          // <-- fails OPEN
        }
    }
}
```

That value is the whole gate at `src/web/templates/setup.rs:141`:

```rust
let _setup_guard = setup_lock.lock().await;
if check_admin_exists(&db_pool).await {
    return ... "Setup has already been completed" ...;
}
```

So a query error is read as "this is a fresh install", and
`setup_handler` proceeds to create a member
(`src/web/templates/setup.rs:166`), promote it to `Active` with
`bypass_dues` (`:193`), and set `is_admin = 1` (`:208`) — with
attacker-supplied email, username, and password. The attacker then logs
in at `/login` as a full administrator of an already-provisioned
organization.

The identically-named helper in the middleware
(`src/api/middleware/setup.rs:66-81`) makes the **opposite** choice on the
same query and documents why: `Err(e) => { ...; true }` — "On error,
assume setup is not needed to avoid blocking". Two copies of one helper
with opposite failure semantics, and the security-critical copy is the
one that fails open.

`setup_handler` also never consults `AppState::admin_exists_observed`,
the process-cached "an admin has been seen" flag that
`setup_page` (`src/web/templates/setup.rs:60`) and the middleware both
read — so even a process that has already observed an admin will still
mint a second one if the DB query happens to error.

## Who triggers it, and how

Any unauthenticated caller who can get that one `SELECT` to return
`Err`. `sqlx` surfaces several routinely-reachable errors here:

- `sqlx::Error::PoolTimedOut` — the SQLite pool is small
  (`config.toml` ships `max_connections = 5`). An attacker floods the
  instance with concurrent requests until acquisition times out, then
  sends `POST /setup`. The later `create` / `update` / `set_admin` calls
  each acquire their own connection and succeed as the burst drains.
- `database is locked` / `SQLITE_BUSY` while another writer holds the
  write lock (a bulk member import, the nightly billing sweep, a backup).
- Any transient I/O error on the database file.

Request shape (no session, no CSRF token needed — `/setup` is on
`CSRF_EXEMPT_PATHS`, `src/api/middleware/security.rs:141`):

```
POST /setup
Content-Type: application/json

{"org_name":"x","email":"a@b.c","username":"a","full_name":"a",
 "password":"...","password_confirm":"..."}
```

## Harm

Full authentication and authorization bypass: an unauthenticated remote
caller obtains an `is_admin = 1`, `Active`, `bypass_dues` account on a
live instance. From there every admin surface is reachable — the member
directory, payment records and refunds, Stripe credentials, settings, and
the audit log.

## Acceptance criteria (against the EXISTING specification)

No spec delta. The fix makes the code conform to requirements that
already exist:

- `openspec/specs/routing-architecture/spec.md` — **Requirement: Single
  AppState shared across surfaces**, scenario *"First-boot setup is
  single-flight"*: when concurrent requests reach the setup-wizard
  handler, "only one SHALL succeed; the other SHALL observe the
  now-existing admin state". A wizard call that observes "no admin"
  because its query errored, and then creates a second admin, violates
  this.
- `openspec/specs/routing-architecture/spec.md` — **Requirement:
  Setup-redirect check is process-cached after first positive
  observation**: `admin_exists_observed` is sticky-once-true for the
  process lifetime. The setup handler must not create an admin after
  that flag is armed.
- `openspec/specs/bootstrap-admin-cli/spec.md` — **Requirement: A
  `create_admin` binary creates the first admin without HTTP**, scenario
  *"Refuse when admin already exists"*: the sibling admin-creation entry
  point refuses outright when any `is_admin = 1` row exists. The HTTP
  wizard is held to the same invariant.

Concretely, after the fix:

1. When the admin-existence query returns `Err`, `POST /setup` SHALL
   refuse (no member row created, no `is_admin` set) rather than
   proceeding. "Unknown" is treated as "an admin exists", matching
   `src/api/middleware/setup.rs`.
2. When `admin_exists_observed` is already `true`, `POST /setup` SHALL
   refuse without querying the database.
3. All behavior on a genuine fresh install (query returns `Ok(None)`,
   flag `false`) is unchanged: the wizard still creates the first admin,
   still arms the flag, still redirects to `/login`.
