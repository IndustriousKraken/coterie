# Setup wizard swallows the first-admin activation error and can lock the org out

## Summary

The first-run setup handler discards the `Result` of the call that promotes
the freshly-created admin to `Active`. If only that one step fails (while the
preceding `create` and the following `set_admin` succeed), the handler still
arms the admin-exists cache, returns `200 { success: true }`, and redirects the
operator to `/login` — but the first admin is left with `status = Pending` and
`is_admin = 1`. A `Pending` member is rejected by **every** authenticated
middleware tier, so the brand-new admin can never log in, and because
`is_admin = 1` is now set, setup cannot be retried. The organization is
permanently locked out of the instance it just provisioned, with no in-app
recovery path.

## Source location

`src/web/templates/setup.rs:188-190`:

```rust
    // Promote to Active with bypass_dues
    let update_request = UpdateMemberRequest {
        status: Some(MemberStatus::Active),
        bypass_dues: Some(true),
        ..Default::default()
    };

    if let Err(e) = member_repo.update(member.id, update_request).await {
        tracing::error!("Failed to activate admin user: {}", e);
    }   // <-- error logged and SWALLOWED; execution continues
```

Contrast with the very next step, `set_admin` (`src/web/templates/setup.rs:193-204`),
which treats an identical-criticality failure as fatal and returns
`500 INTERNAL_SERVER_ERROR`. The two adjacent critical steps handle their
errors inconsistently; the `update` swallow is an oversight, not a deliberate
soft-fail (unlike the `org.name` persistence at lines 216-231, which is
explicitly documented as soft-fail).

## Why this is harmful (trigger and impact)

1. During first-run `POST /setup`, `member_repo.create(...)` succeeds (member
   row created `Pending`, non-admin).
2. `member_repo.update(...)` (promote to `Active` + `bypass_dues`) fails — e.g.
   a transient `SQLITE_BUSY` while the `require_setup` middleware concurrently
   runs `SELECT 1 FROM members WHERE is_admin = 1`, an I/O hiccup, or any
   constraint error. The error is logged and swallowed.
3. `member_repo.set_admin(member.id, true)` succeeds → the row is now
   `status = Pending`, `is_admin = 1`.
4. `admin_exists_observed.store(true, ...)` arms the cache
   (`src/web/templates/setup.rs:210`) and the handler returns
   `200 { success: true, redirect: "/login" }`.

Resulting state is unrecoverable in-app:

- **The admin cannot authenticate.** Per the `auth-middleware-tiers`
  capability, `require_admin_redirect` (and `require_auth`/`require_auth_redirect`/
  `require_restorable`) admit only `Active`/`Honorary` (`Expired` for
  restorable). A `Pending` member is blocked by all of them, and the login
  handler rejects `Pending` before a session is even created. The
  `is_admin = 1` flag is irrelevant because the status gate runs first.
- **Setup cannot be retried.** `check_admin_exists` (`src/web/templates/setup.rs:251`)
  keys off `is_admin = 1`, so the in-lock re-check returns "Setup has already
  been completed" (`src/web/templates/setup.rs:140-150`), and the armed
  `admin_exists_observed` flag (sticky for the process lifetime per the
  `routing-architecture` cache requirement) stops the `require_setup`
  middleware from ever routing back to `/setup`.

Harm: silent failure that reports success while producing an unusable admin →
permanent organization lockout of a fresh install (recoverable only by direct
SQL against the database).

## Acceptance criteria (against existing specification)

This is a behavior-preserving correction of an unhandled error path; it changes
no observable contract on the success path and adds no new requirement. It makes
the code conform to requirements already in canon:

- **`routing-architecture` → Requirement "Setup-redirect check is
  process-cached after first positive observation":** the handler arms
  `admin_exists_observed` and returns success *"immediately after successfully
  creating the first admin."* After the fix, the cache is armed and a
  `success: true` response is returned **only** when the first admin is fully
  provisioned (`status = Active`, `is_admin = 1`). A failed activation aborts
  setup with `500` and does **not** arm the cache.
- **`auth-middleware-tiers` → Requirement "require_admin_redirect enforces
  Active/Honorary AND admin flag AND optional TOTP"** and its scenario that *"a
  setup hiccup does not lock every admin out":* the fix guarantees that a setup
  hiccup leaves a retryable, clean state — never a `Pending` + `is_admin = 1`
  row that no middleware tier will admit.

Concretely:

1. When `member_repo.update(...)` (the `Active` + `bypass_dues` promotion) fails
   during `POST /setup`, the handler MUST NOT continue to `set_admin`, MUST NOT
   arm `admin_exists_observed`, and MUST NOT return `success: true`; it returns
   `500` (mirroring the existing `set_admin` failure handling).
2. On any failure after the member row is created (`update` or `set_admin`),
   the partially-created member row is removed so the operator can retry `/setup`
   cleanly (the orphaned `Pending` row would otherwise block retry with a
   `UNIQUE` violation on email/username).
3. The success path is unchanged: a completed `POST /setup` yields a member with
   `status = Active`, `is_admin = 1`, `bypass_dues = true`, arms the cache, and
   redirects to `/login`.
