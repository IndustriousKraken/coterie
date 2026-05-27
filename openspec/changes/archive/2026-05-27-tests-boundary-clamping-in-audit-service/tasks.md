## 1. Add boundary-clamping tests for `AuditService`

- [x] 1.1 `prune_older_than_clamps_below_one_day` — seed the table
  with one fresh row (`created_at = now()`). Call
  `audit_service.prune_older_than(0)` (a value below the lower clamp
  bound). Assert `Ok(0)` (the clamp lifted it to `1` day, and a row
  inserted "now" is not older than 1 day). Crucially, assert that
  `SELECT COUNT(*) FROM audit_logs` is still `1` — the row was NOT
  wiped.
- [x] 1.2 `prune_older_than_clamps_above_3650` — call
  `prune_older_than(i64::MAX)`. Assert `Ok(0)` (clamped to 3650 days;
  no row is that old). Assert the call returns instead of erroring
  out from SQL arithmetic overflow.
- [x] 1.3 `recent_clamps_limit_below_one` — seed the table with three
  rows. Call `audit_service.recent(0)`. Assert `Ok(rows)` is returned
  with `rows.len() == 1` (the lower clamp lifted limit to 1).
- [x] 1.4 `recent_clamps_limit_above_500` — seed the table with 600
  rows in a single transaction. Call `audit_service.recent(10_000)`.
  Assert `Ok(rows)` is returned with `rows.len() == 500` (the upper
  clamp held the LIMIT at 500).
