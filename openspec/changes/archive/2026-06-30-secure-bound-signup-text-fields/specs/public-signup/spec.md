# public-signup Specification

## ADDED Requirements

### Requirement: Signup bounds and validates its input fields

`POST /public/signup` SHALL validate and length-bound its free-text input fields before creating a member, so an unauthenticated caller cannot persist unbounded data. The handler SHALL reject the request with `400` (`AppError::BadRequest`) when any of the following fails (checked against the trimmed value):

- `email`: non-empty, contains `@`, and at most 254 characters.
- `full_name`: non-empty and at most 200 characters.
- `username`: non-empty and at most 100 characters.

These bounds match the existing public-donate handler so the two unauthenticated entry points are consistent. Validation SHALL run after the bot-challenge gate and before any member is persisted.

#### Scenario: Over-long field is rejected

- **WHEN** a signup request supplies an `email` longer than 254 characters, a `full_name` longer than 200 characters, or a `username` longer than 100 characters
- **THEN** the handler SHALL return `400` and SHALL NOT create a member

#### Scenario: Empty required field is rejected

- **WHEN** a signup request supplies an empty or whitespace-only `username` or `full_name`
- **THEN** the handler SHALL return `400` and SHALL NOT create a member

#### Scenario: Valid bounded signup succeeds

- **WHEN** a signup request supplies a valid `@`-bearing email within 254 characters and non-empty `username`/`full_name` within their bounds, with a verified bot-challenge token
- **THEN** a `Pending` member SHALL be created (the bounds do not reject normal-length input)
