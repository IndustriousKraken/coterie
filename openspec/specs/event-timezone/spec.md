# event-timezone Specification

## Purpose
TBD - created by archiving change event-timezone-correctness. Update Purpose after archive.
## Requirements
### Requirement: Organization timezone is a configurable setting

The system SHALL provide an organization timezone setting keyed
`org.timezone` in the `organization` settings category, holding an IANA
zone name (for example `America/New_York`) and defaulting to `UTC`. The
setting SHALL be editable from the portal settings page, and a write SHALL
be rejected when the value is not a recognized IANA zone name.

#### Scenario: Unknown zone name is rejected

- **WHEN** an admin saves an `org.timezone` value that is not a valid IANA
  zone
- **THEN** the update SHALL be rejected with a clear error and the previous
  value SHALL be retained

#### Scenario: Default is UTC

- **WHEN** no `org.timezone` has been set
- **THEN** the system SHALL treat the organization timezone as `UTC`, which
  reproduces today's behavior

### Requirement: Event times are stored as a local wall-clock plus an IANA zone

An event's authoritative time SHALL be stored as a naive local wall-clock
together with the IANA zone name it was scheduled in. The system SHALL NOT
store a frozen UTC instant as the source of truth for an event, because for
a future event a frozen instant is lost information: a later change to the
zone's rules would silently move the intended wall-clock. The event's zone
SHALL default from `org.timezone` at creation and SHALL be frozen on the row
thereafter, so a later change to the organization setting does not
reinterpret existing events.

#### Scenario: The wall-clock is preserved, the instant is derived

- **WHEN** an event is scheduled for `2026-07-23 19:00` in
  `America/New_York`
- **THEN** the row SHALL store the local time `19:00` and the zone
  `America/New_York`, NOT a converted UTC value as the authoritative field

#### Scenario: A later rule change does not move the wall-clock

- **WHEN** the government changes the zone's offset or DST rules after an
  event is stored but before it occurs, and the tz database is updated
- **THEN** the event's stored local wall-clock SHALL be unchanged and its
  derived UTC instant SHALL reflect the new rules — the organizer's intended
  local time is preserved, not frozen to a stale instant

### Requirement: UTC is derived at read time for public output

The `/public/events` JSON response and the iCal feed SHALL emit each event's
UTC instant derived at serialization time from the stored (local wall-clock,
zone) pair using the current tz database, formatted as RFC 3339 with a `Z`
for JSON and the `Z`-suffixed basic format for iCal. The derivation SHALL
resolve DST gap/overlap cases by a defined rule rather than panicking.

#### Scenario: A remote viewer sees their own local time

- **WHEN** a member in US Pacific time loads the marketing site for the
  7 PM Eastern event
- **THEN** the served instant SHALL be `2026-07-23 23:00:00Z` (derived) and
  the browser SHALL render 4 PM

### Requirement: The admin surface uses the stored wall-clock directly

The admin event create/edit form SHALL store the naive input as-is with the
event zone and SHALL pre-fill from the stored wall-clock without conversion;
admin lists and detail views SHALL render the stored wall-clock. An admin
sees and types the same wall-clock in both directions with no offset math.

#### Scenario: Round-trip preserves the admin's wall-clock

- **WHEN** an admin saves an event at 7 PM and later reopens its edit form
- **THEN** the form SHALL show `19:00` and the admin list SHALL display 7 PM,
  independent of any consumer's timezone

### Requirement: Recurrence is computed on the wall-clock

Recurring-event occurrences SHALL be advanced on the wall-clock in the
event's zone and persisted as local wall-clocks, so a series defined at a
fixed local time keeps that local time across daylight-saving transitions
and later rule changes. Occurrences SHALL NOT be materialized as frozen UTC
instants.

#### Scenario: A weekly evening series survives a DST change

- **WHEN** a "weekly at 7 PM" series spans a daylight-saving transition
- **THEN** every occurrence SHALL remain 7 PM in the event's zone, and the
  derived UTC instants SHALL differ by an hour across the transition

### Requirement: Existing rows are annotated, not shifted

The migration SHALL annotate existing event rows, which already hold naive
local wall-clocks, with the current organization zone, and SHALL NOT shift
any stored time value. Because no instant is frozen and no value is moved,
the correction cannot double-apply and needs no run-once guard.

#### Scenario: Annotation changes no rendered time

- **WHEN** the zone-annotation migration runs against an event stored as
  `2026-07-23 19:00:00`
- **THEN** the stored `19:00` SHALL be unchanged, the row SHALL gain the
  organization zone, and the admin-rendered local time SHALL be identical
  before and after

### Requirement: An event is upcoming until it ends

The domain SHALL expose a single predicate deciding whether an event is still
upcoming at a given instant, and every surface that lists or counts upcoming
events SHALL use it rather than re-deriving the comparison. Three call sites
write the comparison out by hand today, which is how a rule of this kind drifts
between the portal, the dashboard, and the public feed.

An event SHALL be upcoming while its derived end instant is after `now`. Deriving
that instant SHALL use the same wall-clock-plus-zone resolution `start_utc` uses,
so an event does not fall out of the listing early or late by the organization's
UTC offset.

The predicate SHALL take already-derived UTC instants as its inputs rather than
deriving them from an `Event`'s stored fields itself. `Event::start_time` and
`Event::end_time` do not always hold a wall-clock: the public feed's
`derive_utc_instants` overwrites both with the derived instant in place before
filtering, so a predicate that re-derived from those fields would apply the zone
offset a second time and answer by hours on exactly the surface where an event
in progress is most visible. A convenience method on `Event` MAY wrap the
predicate for callers holding an undebased event, but the surface that has
already derived SHALL pass its instants directly.

Where an event records no end time, the event SHALL be upcoming until a defined
grace period after its derived start instant. A missing `end_time` means the end
is unknown, not that the event ends the moment it starts, and treating the absent
value as a zero-length event would drop the listing at exactly the wrong moment.

The grace period SHALL be a single named constant of two hours. It SHALL NOT be
written as a literal at any call site, and SHALL NOT be a configurable setting:
the remedy for an event whose real duration differs is to record its end time,
which is the field that already answers that question.

An event in progress SHALL sort by its start time like any other, which places it
at the head of an ascending list rather than in a separate section.

#### Scenario: An event in progress is still upcoming

- **WHEN** an event runs 19:00–21:00 in the organization's zone and `now` is 19:30
  in that zone
- **THEN** the predicate SHALL report the event as upcoming

#### Scenario: An ended event is no longer upcoming

- **WHEN** the same event is tested at 21:01 in the organization's zone
- **THEN** the predicate SHALL report the event as not upcoming

#### Scenario: An event with no end time uses the grace period

- **WHEN** an event starts at 19:00 with no recorded end time
- **THEN** the predicate SHALL report it upcoming at 20:59 and not upcoming at
  21:01, two hours being the grace period

#### Scenario: The boundary is evaluated on the derived instant, not the wall-clock

- **WHEN** an event's end wall-clock is 21:00 in a zone offset from UTC
- **THEN** the predicate SHALL compare the zone-resolved instant against `now`, so
  the event neither drops early nor lingers by the size of the offset

#### Scenario: An already-derived event is not converted twice

- **WHEN** the predicate decides an event on a surface that has already replaced
  the stored wall-clock with the derived instant
- **THEN** the answer SHALL equal the answer for the same event on a surface that
  has not, and SHALL NOT differ by the zone's UTC offset

