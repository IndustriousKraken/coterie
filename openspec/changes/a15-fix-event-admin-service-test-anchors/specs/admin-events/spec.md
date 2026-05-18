## MODIFIED Requirements

### Requirement: Tests of the recurring-event materializer use anchors relative to runtime

Tests asserting on `RecurringEventService::create_series_with_initial_materialization` (or the materializer's output more generally) SHALL compute their input anchors and `until_date` values relative to `Utc::now()` at runtime. Fixed-calendar timestamps SHALL NOT be used as test inputs to the materializer.

This rule applies BOTH to standalone test files under `tests/` AND to inline `#[cfg(test)] mod tests` blocks inside `src/` files (such as `src/service/event_admin_service.rs`). Wherever a test exercises the materializer, the inputs SHALL be runtime-relative.

The reason: the materializer's horizon is `now + 12 months`. A fixed-calendar anchor drifts further into the past as wall-clock time advances, changing the gap between the anchor and the horizon. Tests that assert occurrence counts (with any tolerance) inevitably break as the gap widens. Tests that constrain via a fixed-calendar `until_date` work until "now" passes that date, at which point the materializer's effective horizon resolves to a past timestamp and produces an empty occurrence set.

Relative-anchor helpers (e.g., `next_tuesday_anchor()` returning the next Tuesday after `Utc::now() + 1 day`) keep the test inputs in the same temporal position regardless of when the suite runs.

#### Scenario: Test anchor is computed from runtime, not hardcoded

- **WHEN** a contributor writes a test that calls `create_series_with_initial_materialization` and asserts an occurrence count or `materialized_through` value
- **THEN** the anchor SHALL be computed from `Utc::now()` (e.g., via a helper that finds the next occurrence-eligible weekday at a chosen time) and any dependent `until_date` SHALL be computed as a relative offset from that anchor

#### Scenario: Hardcoded calendar timestamps in materializer tests are a defect

- **WHEN** a contributor inspects a recurring-event test file or any `src/` file with `#[cfg(test)] mod tests`
- **THEN** instances of `Utc.with_ymd_and_hms(<year>, <month>, <day>, ...)` used as materializer inputs SHALL be treated as defects to be replaced with relative-anchor helpers; the rule is "no fixed-calendar inputs to the materializer in tests, regardless of where the test lives"

#### Scenario: Inline test modules in src/ follow the same rule

- **WHEN** an inline `#[cfg(test)] mod tests` block inside a service file (e.g., `src/service/event_admin_service.rs`) exercises the materializer or the service that wraps it
- **THEN** the test SHALL use runtime-relative anchors. The helpers MAY be duplicated per-file rather than shared until a third caller appears; premature extraction to a shared `src/service/test_helpers.rs` is not required
