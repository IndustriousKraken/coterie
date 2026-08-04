# event-timezone Specification Delta

## ADDED Requirements

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
