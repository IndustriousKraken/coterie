# Event past/upcoming status compares the naive wall-clock to a UTC `now`

## Summary

An event's `start_time` is stored as a **naive local wall-clock** (paired with
the event's IANA `timezone`), and is carried in a `DateTime<Utc>` only as a
typing container — it is NOT a true UTC instant. The true instant is obtained
via `Event::start_utc()` (`src/domain/event.rs:56`), which resolves the
wall-clock through the event's zone.

Six read sites in the portal/admin surface compute an event's temporal status —
`is_past`, and the `upcoming`/`past` list filter — by comparing this **raw
wall-clock** directly against `now`, a true `DateTime<Utc>` (request time). This
mixes a naive local time with a real instant, so the boundary is wrong by the
organization's UTC offset. For the reference deployment (`America/New_York`,
offset −4/−5h) an event flips to "past" **~4–5 hours before it actually
starts**, and the "upcoming"/"past" filters mis-partition by the same margin.

This is the exact footgun the storage model warns about ("a frozen instant is
lost information"): the naive value must be resolved to an instant with
`start_utc()` before it can be compared to `now`. The already-correct paths do
exactly that — see the reminder query
(`src/repository/event_repository.rs:756`, `start_utc() > now && start_utc <= until`)
and the public feed (`src/api/handlers/public.rs:682`, `derive_utc_instants`
before the `> now` retain). These six sites simply skip the derivation.

Display is unaffected: the `.format(...)` lines that render the wall-clock are
correct and must stay as-is (see Acceptance criteria).

## Source location

All six compare the naive `start_time` to a true-instant `now`:

`src/web/portal/admin/events/single.rs:141-142` (list time-filter):

```rust
            match time_filter.as_str() {
                "upcoming" => e.start_time > now,   // wall-clock vs UTC now
                "past" => e.start_time <= now,      // wall-clock vs UTC now
                _ => true,
            }
```

`src/web/portal/admin/events/single.rs:186` (admin list row badge):

```rust
            is_past: e.start_time <= now,
```

`src/web/portal/admin/events/single.rs:311` (admin event-detail badge):

```rust
        is_past: event.start_time <= now,
```

`src/web/portal/admin/events/occurrences.rs:60` (series-occurrence badge):

```rust
            is_past: event.start_time <= now,
```

`src/web/portal/events.rs:84` (member-facing events list badge):

```rust
        let is_past = event.start_time < now;
```

In each, `now` is a true `DateTime<Utc>` (request-time `Utc::now()`), and
`start_time` is the naive wall-clock (`DateTime::from_naive_utc_and_offset(naive, Utc)`
on read). The correct operand is `…start_utc()`.

**Out of scope — do NOT change these:**

- The `.format(...)` renders of the wall-clock — `single.rs:178/298/299`,
  `occurrences.rs:58/124`, `events.rs:148-149`, `dashboard.rs:146/153`,
  `integrations/discord.rs:356`. These correctly display the stored wall-clock
  as the local time and are required to (see Acceptance criteria). `start_utc()`
  here would wrongly show a UTC-shifted time.
- `single.rs:153` list sort `a.start_time.cmp(&b.start_time)` — within a single
  organization every event shares one zone, so wall-clock order equals instant
  order; leaving it avoids needless churn. (Optional hardening only.)

## Why this is harmful (trigger and impact)

1. An event is scheduled for 7 PM in `America/New_York` (EST). It is stored as
   wall-clock `19:00` with zone `America/New_York`; its true instant is the
   next day `00:00Z`.
2. A portal or admin page renders at 2:30 PM Eastern that day. `now` is
   `19:30Z`; the raw `start_time` is `19:00Z`. `start_time <= now` is TRUE.
3. The event is labelled **"past"** — 4.5 hours before it starts. The admin
   "upcoming" filter **hides** it and the "past" filter **surfaces** it, both
   ~4.5h early. The member-facing events list shows the same event as already
   over while people are still planning to attend.

- **Trigger:** any portal/admin event view rendered within the org's UTC-offset
  window before an event's start (up to ~14h for the widest zones; ~4–5h for
  US Eastern). Present for every event, worst around the day of the event.
- **Harm:** incorrect past/upcoming status and filtering on the admin event
  list, admin event detail, series-occurrence rows, and the member events list.
  No data is corrupted and no reminder/public-feed timing is affected (those
  paths already derive) — the damage is misleading status to admins and members
  about whether an event has happened.

## Acceptance criteria (against existing specification)

This is a behavior-preserving correction that adds no requirement and changes no
rendered time. It conforms the code to the `event-timezone` capability already
in canon:

- **`event-timezone` → Requirement "Event times are stored as a local
  wall-clock plus an IANA zone":** the stored value is explicitly NOT a true
  instant ("a frozen instant is lost information"). Comparing the raw wall-clock
  to a UTC `now` treats it as an instant, contradicting this model. The fix
  resolves the instant first, as the model requires.
- **`event-timezone` → Requirement "UTC is derived at read time for public
  output":** establishes derivation (via the stored (wall-clock, zone) pair) as
  the canonical way to obtain an event's instant. A past/upcoming predicate is a
  read-time instant need and MUST use the same derivation. `Event::start_utc()`
  is that derivation.
- **`event-timezone` → Requirement "The admin surface uses the stored
  wall-clock directly" is PRESERVED, not violated.** That requirement governs
  *rendering and form round-trip* — "admin lists and detail views SHALL render
  the stored wall-clock," "no offset math" on the shown time. The fix touches
  only the `is_past`/`upcoming` **comparisons**, never a rendered time: every
  `.format(...)` display stays on the wall-clock, so the admin still sees and
  types the identical `19:00` in both directions. The scenario "Round-trip
  preserves the admin's wall-clock" continues to hold unchanged.

Concretely, after the fix:

1. Each of the six sites compares `…start_utc()` (the derived instant) to `now`,
   not the raw `start_time`. An event is "past" iff its true instant has passed,
   independent of the org's offset.
2. No rendered time changes: the wall-clock `.format(...)` outputs on the admin
   list, admin detail, occurrence rows, dashboard, member events list, and
   Discord post are byte-identical before and after.
3. The already-correct reminder and public-feed paths are untouched.

## Tasks

Behavior-preserving fix. Only the past/upcoming **comparisons** change; every
rendered wall-clock time MUST stay identical — treat any change to a displayed
time as a regression.

### 1. Derive the instant before comparing to `now`

- [x] 1.1 `src/web/portal/admin/events/single.rs:141-142` — change
  `e.start_time > now` → `e.start_utc() > now` and `e.start_time <= now` →
  `e.start_utc() <= now` in the `time_filter` match.
- [x] 1.2 `src/web/portal/admin/events/single.rs:186` — `is_past: e.start_time <= now`
  → `is_past: e.start_utc() <= now`.
- [x] 1.3 `src/web/portal/admin/events/single.rs:311` —
  `is_past: event.start_time <= now` → `is_past: event.start_utc() <= now`.
- [x] 1.4 `src/web/portal/admin/events/occurrences.rs:60` —
  `is_past: event.start_time <= now` → `is_past: event.start_utc() <= now`.
- [x] 1.5 `src/web/portal/events.rs:84` — `let is_past = event.start_time < now`
  → `let is_past = event.start_utc() < now`.
- [x] 1.6 Do NOT alter any `.format(...)` render line or `single.rs:153` sort.
  Confirm `Event::start_utc()` is in scope for each `Event` value edited (it is
  an inherent method on `Event`).

### 2. Regression test

- [x] 2.1 Add a unit test near the `event-timezone` tests
  (`src/domain/event.rs`) or a portal test that builds an `Event` with a 7 PM
  `America/New_York` wall-clock and asserts: at a `now` that is after `19:00Z`
  but before the derived `start_utc()` (i.e. `00:00Z` next day),
  `start_utc() > now` is TRUE (upcoming) while the raw `start_time > now` would
  be FALSE. This pins the offset bug so it cannot regress.

### 3. Verify

- [x] 3.1 `cargo test` — full suite green, including the new test and the
  existing `event-timezone` / reminder suites.
- [x] 3.2 `cargo clippy` on the touched files — no new warnings.
