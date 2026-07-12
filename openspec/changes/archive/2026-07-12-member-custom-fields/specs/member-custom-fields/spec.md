# member-custom-fields Specification

## ADDED Requirements

### Requirement: Admins define org-specific member fields

Admins SHALL manage member field definitions at `/portal/admin/settings/member-fields`: create, rename, reorder, activate/deactivate, toggle member-editability, and delete. A definition SHALL carry a display name, a stable unique `field_key`, a type (`text` or `url`), a `member_editable` flag, a sort order, and an active flag. Definition mutations SHALL be validated (non-empty bounded name; unique key) and audited. Deleting a definition SHALL remove its stored values (cascade) after an explicit confirmation in the UI; deactivating SHALL hide the field everywhere while preserving values.

#### Scenario: Admin creates a field

- **WHEN** an admin creates a definition named "HackTheBox ID" of type `text`
- **THEN** the field SHALL appear on the admin member page and (being member-editable by default) on the member profile page, and an audit entry SHALL be written

#### Scenario: Deactivation hides but preserves

- **WHEN** an admin deactivates a definition that has stored values
- **THEN** the field SHALL no longer render on member or admin pages, and reactivating it SHALL surface the previously stored values unchanged

### Requirement: Custom field values are validated and bounded

Setting a value SHALL require an existing, active definition. Values SHALL be length-bounded (500 characters) and trimmed; a blank value SHALL clear the stored row rather than storing an empty string. A `url`-typed field SHALL reject a non-empty value that does not start with `http://` or `https://`.

#### Scenario: URL field rejects a non-URL

- **WHEN** a value of `not a link` is submitted for a `url`-typed field
- **THEN** the save SHALL be rejected with a clear message and no value SHALL be stored

#### Scenario: Blank clears the value

- **WHEN** a member or admin submits an empty value for a field that has a stored value
- **THEN** the stored row SHALL be removed

### Requirement: Admins edit any member's fields from the member page

The admin member page SHALL render a Custom Fields card listing every active definition with the member's current values, saved in one form, when at least one active definition exists. Saves SHALL be audited against the member entity.

#### Scenario: Admin sets a member's field

- **WHEN** an admin saves a value for a member's field
- **THEN** the value SHALL persist, render on subsequent views of the member page, and an audit entry SHALL be written

### Requirement: Members maintain their own member-editable fields

The member profile page SHALL render the Custom Fields card restricted to active, `member_editable` definitions, saving to the member's own values only. Fields with `member_editable` off SHALL NOT be rendered to or writable by the member, even by crafted request.

#### Scenario: Member fills in their own ID

- **WHEN** a member saves a value for a member-editable field on their profile
- **THEN** the value SHALL persist and be visible to them and to admins

#### Scenario: Non-editable field rejects member writes

- **WHEN** a member submits a value for a field whose `member_editable` flag is off
- **THEN** the write SHALL be rejected and nothing SHALL be stored
