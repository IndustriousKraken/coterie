# admin-events Specification

## MODIFIED Requirements

### Requirement: Tests of the recurring-event materializer use anchors relative to runtime

Tests asserting on `RecurringEventService::create_series_with_initial_materialization` (or the materializer's output more generally) SHALL compute their input anchors and `until_date` values relative to `Utc::now()` at runtime. Fixed-calendar timestamps SHALL NOT be used as test inputs to the materializer.

This rule applies BOTH to standalone test files under `tests/` AND to inline `#[cfg(test)] mod tests` blocks inside `src/` files (such as `src/service/event_admin_service.rs`). Wherever a test exercises the materializer, the inputs SHALL be runtime-relative.

The reason: the materializer's horizon is `now + 12 months`. A fixed-calendar anchor drifts further into the past as wall-clock time advances, changing the gap between the anchor and the horizon. Tests that assert occurrence counts (with any tolerance) inevitably break as the gap widens. Tests that constrain via a fixed-calendar `until_date` work until "now" passes that date, at which point the materializer's effective horizon resolves to a past timestamp and produces an empty occurrence set.

Relative-anchor helpers (e.g., `next_tuesday_anchor()` returning the next Tuesday after `Utc::now() + 1 day`) keep the test inputs in the same temporal position regardless of when the suite runs.

**Runtime-relative is necessary but not sufficient for horizon-extension tests.** A test that asserts a horizon extension *produced* occurrences (`extend_horizon(...)` returning a non-zero count, or new occurrence rows appearing) SHALL additionally derive both its `until_date` and its extension target from `DEFAULT_HORIZON` rather than from absolute day counts, and SHALL leave an extension window wide enough to contain at least one occurrence of the rule under test on **every** weekday the suite might run on.

The reason is that `create_series_with_initial_materialization` fills up to `min(now + DEFAULT_HORIZON, until_date)` and `extend_horizon` caps its target at `until_date`. An `until_date` at or just past `now + DEFAULT_HORIZON` therefore leaves the extension nothing — or almost nothing — to do, no matter how far out the extension target is set. A window narrower than the recurrence interval contains an occurrence only on some days of the week, which produces a test that passes on the day it is written and fails later on an unrelated commit.

Absolute day counts SHALL NOT be used to position these bounds even when they are computed from `Utc::now()`, because they silently stop lining up if `DEFAULT_HORIZON` changes. Deriving from the constant keeps the window correct by construction.

**Diagnostic signature:** an occurrence-count assertion that passes locally and fails in CI on some days — or that starts failing on a commit that touched nothing related — SHALL be treated as a horizon-window defect in the test's bounds, NOT as flaky infrastructure to be retried or as a regression in the materializer.

#### Scenario: Test anchor is computed from runtime, not hardcoded

- **WHEN** a contributor writes a test that calls `create_series_with_initial_materialization` and asserts an occurrence count or `materialized_through` value
- **THEN** the anchor SHALL be computed from `Utc::now()` (e.g., via a helper that finds the next occurrence-eligible weekday at a chosen time) and any dependent `until_date` SHALL be computed as a relative offset from that anchor

#### Scenario: Hardcoded calendar timestamps in materializer tests are a defect

- **WHEN** a contributor inspects a recurring-event test file or any `src/` file with `#[cfg(test)] mod tests`
- **THEN** instances of `Utc.with_ymd_and_hms(<year>, <month>, <day>, ...)` used as materializer inputs SHALL be treated as defects to be replaced with relative-anchor helpers; the rule is "no fixed-calendar inputs to the materializer in tests, regardless of where the test lives"

#### Scenario: Inline test modules in src/ follow the same rule

- **WHEN** an inline `#[cfg(test)] mod tests` block inside a service file (e.g., `src/service/event_admin_service.rs`) exercises the materializer or the service that wraps it
- **THEN** the test SHALL use runtime-relative anchors. The helpers MAY be duplicated per-file rather than shared until a third caller appears; premature extraction to a shared `src/service/test_helpers.rs` is not required

#### Scenario: An extension test derives its bounds from DEFAULT_HORIZON

- **WHEN** a contributor writes a test asserting that `extend_horizon` materialized new occurrences
- **THEN** the series `until_date` and the extension target SHALL both be expressed as offsets from `DEFAULT_HORIZON` (e.g. `Utc::now() + DEFAULT_HORIZON + Duration::days(90)` and `... + Duration::days(60)`), NOT as absolute day counts

#### Scenario: An extension window narrower than the recurrence interval is a defect

- **WHEN** a test's `until_date` sits at or barely beyond `now + DEFAULT_HORIZON`, leaving an extension window shorter than the interval of the rule under test
- **THEN** that test SHALL be treated as defective even if it currently passes, because it asserts a non-zero occurrence count against a window that contains an occurrence only on some weekdays

#### Scenario: A weekday-dependent failure is diagnosed as a window bug

- **WHEN** an occurrence-count assertion fails in CI on some days and passes on others, with no related code change
- **THEN** the failure SHALL be investigated as a horizon-window defect in the test's bounds and SHALL NOT be dismissed as infrastructure flakiness or retried until green
