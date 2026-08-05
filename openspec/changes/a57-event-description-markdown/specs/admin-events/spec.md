# admin-events Specification Delta

## ADDED Requirements

### Requirement: The event editor states that Markdown is supported

The event create and edit forms SHALL tell the admin that the description field
supports Markdown, naming the constructs available, in the same manner the
announcement editor already does.

An input that silently accepts a formatting language it does not advertise gives
its author no signal in either direction: the text saves, the box looks right,
and the outcome appears later on a surface the author was not looking at. That is
how a production event came to carry `**Monthly Hack the Box and Training
Night**` into a social preview card — the announcement editor had taught the
organizer that Markdown was accepted, and the event editor said nothing to the
contrary.

The wording SHALL match the announcement editor's rather than being written
afresh, so the two fields are described identically and cannot drift into
implying different capabilities.

#### Scenario: The create form advertises Markdown

- **WHEN** an admin opens the event creation form
- **THEN** the description field SHALL indicate that Markdown formatting is
  supported

#### Scenario: The edit form advertises Markdown

- **WHEN** an admin opens an existing event for editing
- **THEN** the description field SHALL carry the same indication

#### Scenario: The hint matches the announcement editor's

- **WHEN** the event and announcement editors are compared
- **THEN** both SHALL describe the supported constructs in the same terms
