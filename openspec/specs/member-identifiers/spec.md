# member-identifiers Specification

## Purpose
TBD - created by archiving change a66-case-insensitive-member-identifiers. Update Purpose after archive.
## Requirements
### Requirement: Member identifiers match without regard to case

Every lookup of a member by email address or by username SHALL match without regard to letter case. This SHALL be implemented once, in the member repository, so that login, password reset, signup's duplicate handling, public donation attachment, and bulk import all behave identically and no caller can reintroduce a byte-exact comparison.

Case-sensitive matching has no security value for these identifiers and a real cost: it rejects members who type their own address or username the way people ordinarily do. Mail providers treat addresses case-insensitively in practice, and phones and mail clients routinely change the capitalisation a person types. A password reset requested with a differently-cased address fails silently — the reset page must not reveal which addresses exist — so the member sees a confirmation and receives nothing.

Case-insensitivity SHALL be ASCII, consistent with the database's own case folding. Unicode case folding is out of scope.

#### Scenario: A password reset requested in a different case is sent

- **WHEN** a reset is requested for `mia@example.com` and the member is stored as `Mia@example.com`
- **THEN** the reset SHALL be issued to that member

#### Scenario: A donation matches its member regardless of case

- **WHEN** a public donation is made with an email that matches an existing member's except in case
- **THEN** it SHALL be treated exactly as a donation whose email matched exactly

### Requirement: Identifiers are stored as entered

Emails and usernames SHALL be stored and displayed exactly as the member or administrator entered them. Case-insensitive matching SHALL NOT be achieved by rewriting stored values, and no existing row SHALL be altered to accommodate it.

Rewriting stored identifiers would change how members' names appear, and would break a password manager entry saved with the original capitalisation — a new lockout introduced by the fix for an old one. Matching insensitively over preserved values fixes every affected member on deployment with no data migration.

#### Scenario: Display preserves case

- **WHEN** a member registered as `RelicIX` views their profile
- **THEN** their username SHALL be shown as `RelicIX`

#### Scenario: Existing rows are untouched

- **WHEN** this change is deployed
- **THEN** no member's stored email or username SHALL be modified

### Requirement: Identifiers are unique without regard to case

No two members SHALL have emails, or usernames, that differ only in letter case. Creating or changing a member's email or username to a value that matches another member's except in case SHALL be refused, through the same duplicate path that already refuses an exact match.

This is what makes case-insensitive matching safe. Without it, `relicix` could be registered beside `RelicIX`, and a login for either would be ambiguous — potentially signing a person into someone else's account. Uniqueness and matching SHALL therefore use the same notion of equality.

The constraint SHALL be enforced by the database, not only by application checks, so that concurrent requests and every write path are covered.

#### Scenario: A case-variant signup is refused

- **WHEN** a signup requests the username `relicix` while `RelicIX` exists
- **THEN** it SHALL be refused as a duplicate, exactly as `RelicIX` would be

#### Scenario: A case-variant email is refused

- **WHEN** a new member or an edit supplies an email equal to an existing member's except in case
- **THEN** it SHALL be refused as a duplicate

#### Scenario: Bulk import reports case-variant duplicates

- **WHEN** an imported row's email or username matches an existing member's except in case
- **THEN** that row SHALL be reported as already existing and SHALL NOT be created

