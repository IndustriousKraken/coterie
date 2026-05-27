## ADDED Requirements

### Requirement: AuditService::prune_older_than clamps retention_days into [1, 3650]

`AuditService::prune_older_than(retention_days)` SHALL clamp
`retention_days` into the inclusive range `[1, 3650]` before binding
it into the `DELETE` query. The lower bound is load-bearing: a value
of `0` or a negative value SHALL NOT delete rows created today,
because the SQL becomes `created_at < datetime('now', '-1 days')`
after clamping. The upper bound caps SQL date arithmetic at a
10-year window.

#### Scenario: prune_older_than(0) does not wipe today's rows

- **WHEN** the caller invokes `prune_older_than(0)` against a table
  containing a row whose `created_at` is `now()`
- **THEN** the method SHALL return `Ok(0)` AND the row count SHALL be
  unchanged; the clamp SHALL have prevented an `interval = 0`
  arithmetic that would otherwise delete the row

#### Scenario: prune_older_than(i64::MAX) returns cleanly

- **WHEN** the caller invokes `prune_older_than(i64::MAX)`
- **THEN** the method SHALL clamp the interval to 3650 days and
  return `Ok(_)`; the call SHALL NOT propagate a SQL arithmetic
  overflow error

### Requirement: AuditService::recent clamps limit into [1, 500]

`AuditService::recent(limit)` SHALL clamp `limit` into the inclusive
range `[1, 500]` before binding it into the `LIMIT` clause. The lower
bound prevents `LIMIT 0` from returning an empty set when the caller
passed a UI default of `0`; the upper bound caps the scan cost.

#### Scenario: recent(0) returns at most one row

- **WHEN** the caller invokes `recent(0)` against a non-empty
  `audit_logs` table
- **THEN** the returned `Vec<AuditEntry>` SHALL have length `1`
  (the most recent row), not `0`

#### Scenario: recent(10000) returns at most 500 rows

- **WHEN** the caller invokes `recent(10_000)` against a table
  containing 600 rows
- **THEN** the returned `Vec<AuditEntry>` SHALL have length `500`,
  with the 500 most recent rows by `created_at DESC`
