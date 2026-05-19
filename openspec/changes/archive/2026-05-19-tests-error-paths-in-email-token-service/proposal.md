## Why

`EmailTokenService` (`src/auth/email_tokens.rs`) is the security-critical
backend for email-verification and password-reset tokens. Its three public
methods — `create`, `consume`, `invalidate_for_member` — plus `cleanup_expired`
implement the single-use, time-limited token contract spelled out in
`openspec/specs/email-tokens/spec.md`.

`grep -l "EmailTokenService\|email_tokens" tests/*.rs` returns **zero**
matches. None of the following error / edge paths are exercised:

- **Atomic single-use redemption** (`consume` lines 71–102): the atomic
  UPDATE … RETURNING gate that ensures a second redemption returns `None`.
- **Expired-token rejection** (`consume`, `expires_at > ?` clause): a row
  past `expires_at` must not redeem.
- **Cross-purpose isolation** (line 33–39, `verification()` vs.
  `password_reset()` constructors): a token issued by one constructor must
  not redeem through the other (the two table names are the only thing
  keeping these flows apart in code).
- **`invalidate_for_member`** (line 107–119): outstanding tokens for the
  member are marked consumed; an already-consumed token's `consumed_at`
  is not bumped.
- **`cleanup_expired`** (line 123–131): returns the number of rows deleted;
  consume-time expiry enforcement is independent of cleanup.

The spec calls these invariants out explicitly with `#### Scenario:` entries
("Token redeems exactly once", "Expired token cannot redeem", "Reset token
cannot redeem at verification endpoint", "Other outstanding reset tokens are
killed after a successful reset"), but there is no test code locking any of
them in. A regression that loosened the `WHERE` clause or swapped the table
constants would silently break security guarantees.

## What Changes

Add a new integration-test file `tests/email_tokens_test.rs` that constructs
both `EmailTokenService::verification(pool)` and
`EmailTokenService::password_reset(pool)` against an in-memory SQLite +
migrations harness (mirroring `tests/totp_test.rs`) and adds the following
tests:

- `consume_redeems_exactly_once` — `create()` → `consume()` returns
  `Some(member_id)`; second `consume()` of the same plaintext returns `None`.
- `consume_rejects_expired_token` — insert a row directly via
  `sqlx::query` with `expires_at` in the past; assert `consume` returns
  `None` even though the row exists and `consumed_at IS NULL`.
- `consume_rejects_unknown_token` — assert `consume("not-a-real-token")`
  returns `Ok(None)`.
- `verification_and_reset_use_separate_tables` — mint a token via
  `password_reset(pool)`; assert that `verification(pool).consume(token)`
  returns `None`, and vice versa.
- `invalidate_for_member_marks_outstanding_consumed` — mint two tokens for
  the same member; call `invalidate_for_member`; assert both subsequent
  `consume` calls return `None`.
- `cleanup_expired_deletes_only_expired_rows` — mint one fresh token, insert
  one already-expired row directly; assert `cleanup_expired` returns 1 and
  that the fresh token still consumes successfully.
- `created_token_is_sha256_of_plaintext` — assert the `token_hash` column
  for a created row equals `sha2::Sha256(plaintext)` (locks in the storage
  invariant from the "Tokens are stored as SHA-256 hashes" requirement).

## Impact

- New file: `tests/email_tokens_test.rs`.
- No production-code changes.
- The scenarios under `openspec/specs/email-tokens/spec.md` requirements
  ("Tokens are single-use via atomic consume", "Expired tokens cannot
  redeem", "Cross-purpose protection is enforced by separate tables",
  "Successful consume invalidates other outstanding tokens", "Background
  cleanup prunes expired rows") gain locked-in test coverage. See
  `specs/email-tokens/spec.md` in this change for added scenario phrasing.
