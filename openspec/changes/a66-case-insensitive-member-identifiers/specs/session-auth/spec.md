# session-auth Specification Delta

## MODIFIED Requirements

### Requirement: Login lookup by email (api) and username-or-email (web)

`POST /auth/login` (api JSON handler in `src/api/handlers/auth.rs`) SHALL look up the member by email only. `POST /login` (web template handler in `src/web/templates/auth.rs`) SHALL look up by username first, falling back to email. Both handlers SHALL apply rate limiting before any database lookup.

Both lookups SHALL match without regard to letter case, as every lookup of a member identifier does under the `member-identifiers` capability. A member stored as `Mia@rupecreativehouse.com` SHALL be found by `mia@rupecreativehouse.com`, and `RelicIX` by `relicix`. Byte-exact matching locked two members out in September 2026 while their accounts and passwords were entirely valid: every attempt was denied as an unknown identifier, so no password — including one freshly set by a completed reset — was ever checked.

#### Scenario: API handler accepts email only

- **WHEN** a JSON request to `/auth/login` carries `email`
- **THEN** the handler SHALL look up by `email` and SHALL NOT attempt a username lookup

#### Scenario: Web handler accepts username OR email

- **WHEN** a JSON request to `/login` (the web template handler) carries a `username` field
- **THEN** the handler SHALL try `find_by_username` first and fall back to `find_by_email` if username yields nothing

#### Scenario: An email typed in a different case signs in

- **WHEN** a member whose stored email is `Mia@example.com` signs in as `mia@example.com` with the correct password
- **THEN** the login SHALL succeed

#### Scenario: A username typed in a different case signs in

- **WHEN** a member whose stored username is `RelicIX` signs in as `relicix` with the correct password
- **THEN** the login SHALL succeed
