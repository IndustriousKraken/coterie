# Tasks

Default behavior (upcoming-only, iCal) MUST stay byte-identical — the home page
and calendar subscriptions depend on it. Only an explicit valid range changes the
JSON result.

## 1. Query + handler

- [x] 1.1 `src/api/handlers/public.rs` — add `from: Option<String>` and
  `to: Option<String>` to `PublicEventsQuery` (RFC 3339 instants), documented in
  the `IntoParams`/`#[utoipa::path]` and `src/api/docs.rs`.
- [x] 1.2 In `list_events`, after `derive_utc_instants`: if `format=ical`, keep
  the existing upcoming filter (ignore range). Otherwise, parse `from`/`to`; treat
  the request as a **range request** only when BOTH parse as valid RFC 3339 AND
  `to > from` AND `to - from <= MAX_SPAN` (implementation constant, ~400 days).
- [x] 1.3 Range request → retain events whose `start_utc()` is in `[from, to)`
  (past included); sort ascending; apply `limit`. Non-range (default, or
  malformed/over-wide range) → the existing `start_time > now` upcoming filter,
  unchanged. Projection, members-only sanitization, and AdminOnly exclusion apply
  identically in both branches.

## 2. Tests

- [x] 2.1 `?from=&to=` spanning a past month with a past event → that event is in
  the JSON, correctly projected and (if members-only) sanitized.
- [x] 2.2 No range → a past event is still excluded (unchanged default).
- [x] 2.3 Malformed (`from` only), unparseable, or over-wide (> MAX_SPAN) range →
  falls back to the upcoming-only list, HTTP 200 (never an error).
- [x] 2.4 `?format=ical&from=&to=` → still the upcoming-only iCal (range ignored).
- [x] 2.5 Existing upcoming/sanitization/internal-field tests unchanged.

## 3. Verify

- [x] 3.1 `openspec validate public-events-past-via-range --strict` passes.
- [x] 3.2 `cargo test` (public-feed suites) green; `cargo clippy` clean.
