## Why

`src/service/audit_service.rs` has two boundary-clamping invariants
that are critical to safe operation but have zero test coverage. The
file itself contains no `#[cfg(test)] mod tests` block, and grepping
`tests/` for `prune_older_than`, `prune`, or `retention` yields zero
hits.

- `prune_older_than(retention_days)` (line 96): `let days =
  retention_days.clamp(1, 3650);`. The inline comment says
  "refuse both absurdly short and absurdly long". The lower bound is
  load-bearing: without it, an operator (or a misconfigured `config.toml`)
  could call `prune_older_than(0)` or a negative value and silently
  delete the entire audit log. The upper bound is also load-bearing:
  `3650` days (10 years) caps the SQL `datetime('now', '-N days')`
  arithmetic from getting nonsense from a stray huge integer.
- `recent(limit)` (line 118): `bind(limit.clamp(1, 500))`. An admin
  page that submits `?limit=0` should still return at least 1 row,
  and `?limit=10000` should not run an unbounded scan.

The audit-logging capability spec currently has no requirement
locking either of these clamps in. A naive refactor (e.g., someone
"simplifies" the clamp away because `0 days` "shouldn't happen") could
land without a test catching the regression, and the result would be
a total audit wipe with no error visible to the caller.

## What Changes

Add an inline `#[cfg(test)] mod tests` block to
`src/service/audit_service.rs` with four `#[tokio::test]` cases that
prove the clamps hold at both ends for both methods.

## Impact

- `src/service/audit_service.rs` — add a new inline `#[cfg(test)] mod
  tests` block (the file currently has none). Tests use an in-memory
  SQLite pool with `migrations/` applied.
- No production code change. No existing tests are modified or
  deleted.
