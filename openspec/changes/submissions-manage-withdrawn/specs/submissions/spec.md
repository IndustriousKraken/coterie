# submissions Specification

## MODIFIED Requirements

### Requirement: A member can access only their own submissions

A member SHALL be able to read, edit, withdraw, delete, and re-open ONLY submissions whose submitter is that same authenticated member. Editing SHALL be permitted only while the submission is `submitted`. The owner MAY DELETE a submission that is in a terminal `withdrawn` or `declined` state — removing it from their list and deleting its attachment, if any; deletion of a submission in any other state (`submitted`, `under_review`, `accepted`, `scheduled`) SHALL be refused. The owner MAY RE-OPEN a `withdrawn` submission back to `submitted` for revision and resubmission; re-open SHALL be allowed only from `withdrawn` (a `declined` submission is not resurrected). A request by any member to read or mutate a submission they do not own SHALL be denied without revealing the resource, and MUST NOT rely on the id being unguessable. Admins are exempt from the ownership restriction for review purposes.

#### Scenario: Cross-member read is denied

- **WHEN** member A requests submission `X` owned by member B (by its id)
- **THEN** the response SHALL be denied (404 or 403) and SHALL NOT disclose `X`'s
  contents

#### Scenario: Owner reads and withdraws their own submission

- **WHEN** member B requests or withdraws submission `X` that B submitted
- **THEN** the operation SHALL succeed, and after withdrawal `X` SHALL be in a
  terminal `withdrawn` state

#### Scenario: Editing after a decision is refused

- **WHEN** a member edits a submission that is no longer `submitted`
- **THEN** the edit SHALL be refused

#### Scenario: Owner deletes a withdrawn or declined submission

- **WHEN** member B deletes their own submission that is `withdrawn` or `declined`
- **THEN** the submission SHALL be removed from their list and its attachment (if
  any) SHALL be deleted

#### Scenario: Deleting a non-terminal submission is refused

- **WHEN** member B attempts to delete their own submission that is `submitted`,
  `under_review`, `accepted`, or `scheduled`
- **THEN** the deletion SHALL be refused and the submission SHALL be unchanged

#### Scenario: Owner re-opens a withdrawn submission

- **WHEN** member B re-opens their own `withdrawn` submission
- **THEN** the submission SHALL return to `submitted` and become editable again;
  re-opening a submission that is not `withdrawn` SHALL be refused

#### Scenario: A non-owner cannot delete or re-open a submission

- **WHEN** member A attempts to delete or re-open submission `X` owned by member B
- **THEN** the request SHALL be denied without disclosing `X`

### Requirement: Publication and scheduling are reviewer-gated

A member's requested visibility SHALL be treated as a request only; NO
submission content SHALL reach the public/marketing surface until an admin
accepts it. An admin SHALL move a submission through
`submitted → under_review → accepted | declined`. Aside from the submitter
withdrawing their own submission, or re-opening their own `withdrawn` submission
back to `submitted` (both per the owner-access requirement), no non-admin SHALL
change a submission's status. On acceptance with a schedule, the service SHALL create a
standard `Event` through the existing event path, whose visibility mirrors the
accepted visibility; declined and withdrawn submissions SHALL never be published.
Status transitions by an admin SHALL be recorded in the audit log.

#### Scenario: A member cannot self-publish

- **WHEN** a member sets requested visibility `public` and saves
- **THEN** the submission SHALL NOT appear on any public/marketing surface while
  its status is `submitted` or `under_review`

#### Scenario: Acceptance with a schedule creates an event

- **WHEN** an admin accepts a submission and supplies a schedule
- **THEN** a standard `Event` SHALL be created via the existing event path, with
  visibility matching the accepted submission, and the decision SHALL be audited

#### Scenario: A non-admin cannot change status

- **WHEN** a member attempts to set a submission's status to `accepted`
- **THEN** the attempt SHALL be denied and the status SHALL be unchanged
