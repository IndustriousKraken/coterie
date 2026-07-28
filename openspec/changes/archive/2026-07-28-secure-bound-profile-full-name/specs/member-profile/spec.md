# member-profile Specification

## ADDED Requirements

### Requirement: Profile update bounds its free-text input

`POST /portal/profile` SHALL validate and length-bound `full_name` before
persisting it, so an authenticated member cannot write unbounded data into
`members.full_name`. The handler SHALL reject the request with an inline error
and SHALL NOT call `member_repo.update` when, checked against the trimmed value,
`full_name` is empty or longer than 200 characters. The trimmed value is what
SHALL be persisted.

This bound matches the one `POST /public/signup` already applies to the same
field (see the `public-signup` capability's requirement that signup bounds and
validates its input fields), so the two entry points that write
`members.full_name` cannot drift apart. The bound matters here because
`full_name` is rendered on the admin member list, on every event and class
roster, in the member CSV export, and in outbound email — surfaces a single
oversized row would degrade for everyone.

#### Scenario: An over-long name is rejected

- **WHEN** a member submits `POST /portal/profile` with a `full_name` longer than
  200 characters
- **THEN** the handler SHALL return its inline error fragment naming the limit,
  and the member's stored `full_name` SHALL be unchanged

#### Scenario: A blank name is rejected

- **WHEN** a member submits `POST /portal/profile` with a `full_name` that is
  empty or whitespace-only
- **THEN** the handler SHALL return its inline error fragment, and the member's
  stored `full_name` SHALL be unchanged

#### Scenario: A valid name is trimmed and saved

- **WHEN** a member submits `POST /portal/profile` with a `full_name` of
  `"  Ada Lovelace  "`
- **THEN** the update SHALL succeed and the stored `full_name` SHALL be exactly
  `"Ada Lovelace"`
