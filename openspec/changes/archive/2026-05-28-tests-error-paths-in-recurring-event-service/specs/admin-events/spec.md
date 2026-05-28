## ADDED Requirements

### Requirement: RecurringEventService rejects invalid recurrence input with a 4xx

`RecurringEventService::create_series_with_initial_materialization` SHALL return `AppError::BadRequest` (NOT `Internal`, NOT a panic) when either the supplied `Recurrence` fails its own `validate()` (e.g., a `Weekly` rule with an empty `weekdays` set, or an `interval` of 0), OR the combination of `template.start_time`, the rule, and `until_date` produces zero occurrences before the cutoff (e.g., `until_date` set before `template.start_time`). In both cases no `event_series` row SHALL be inserted; the call SHALL be side-effect free with respect to persisted state.

#### Scenario: Empty weekly weekday set returns BadRequest, no row written

- **WHEN** an admin POSTs the new-recurring-event form with a weekly rule whose weekday selector was left empty
- **THEN** `create_series_with_initial_materialization` SHALL return `Err(AppError::BadRequest(_))` and `SELECT COUNT(*) FROM event_series` SHALL be unchanged

#### Scenario: until_date earlier than start_time returns BadRequest

- **WHEN** the form submits `template.start_time = T` with `until_date = T - 6 days` (the operator mis-typed the end date)
- **THEN** the method SHALL return `Err(AppError::BadRequest(msg))` where the message contains "no occurrences"; no `event_series` or `events` rows SHALL be inserted

### Requirement: compute_occurrence_start_time validates index domain and series state

`RecurringEventService::compute_occurrence_start_time(series, index)` SHALL reject `index < 1` with `AppError::BadRequest("occurrence_index must be >= 1")`, SHALL reject the case where the series has zero corresponding `events` rows (anchor inference impossible) with `AppError::Internal("cannot infer series anchor — no occurrences exist")`, and SHALL reject an `index` beyond the generator's internal 10_000-entry cap with `AppError::BadRequest` whose message contains "beyond the series's generated occurrences". Each failure mode SHALL produce a typed error rather than a panic or generic 500.

#### Scenario: Zero or negative occurrence_index is a 4xx

- **WHEN** `compute_occurrence_start_time` is called with `index = 0` (or any value `< 1`)
- **THEN** the method SHALL return `Err(AppError::BadRequest(msg))` where `msg.contains("occurrence_index must be >= 1")`

#### Scenario: Series with no occurrences cannot extrapolate

- **WHEN** the caller invokes `compute_occurrence_start_time` against a series whose every `events` row has been deleted out-of-band
- **THEN** the method SHALL return `Err(AppError::Internal(msg))` where `msg.contains("cannot infer series anchor")`

#### Scenario: occurrence_index past the generator cap is a 4xx

- **WHEN** the caller asks for `compute_occurrence_start_time(series, 20_000)` on a weekly series (well past the 10_000-entry cap inside `generate_occurrences`)
- **THEN** the method SHALL return `Err(AppError::BadRequest(msg))` where `msg.contains("beyond the series's generated occurrences")`
