# event-timezone Specification (delta)

## ADDED Requirements

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
