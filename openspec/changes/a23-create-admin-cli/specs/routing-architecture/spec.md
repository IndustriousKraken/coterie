## ADDED Requirements

### Requirement: GET /setup redirects when an admin already exists

The `/setup` GET handler SHALL check whether an admin already exists (via `check_admin_exists` or the `admin_exists_observed` `AppState` flag) and, if true, redirect to `/login` instead of rendering the setup form.

This complements the existing POST /setup behavior (which already refuses post-bootstrap inside the `setup_lock` guard). After this change, /setup is fully a dead-end once bootstrap is complete: GET redirects away, POST refuses. The security exemption that lets /setup operate without a session is bounded strictly to the bootstrap window.

#### Scenario: GET /setup after admin exists redirects to /login

- **WHEN** a request hits `GET /setup` on an instance where at least one admin already exists
- **THEN** the response SHALL be a 303 (or 302) redirect to `/login`; the setup form HTML SHALL NOT be rendered

#### Scenario: GET /setup before admin exists renders the form

- **WHEN** a request hits `GET /setup` on a fresh instance with no admin
- **THEN** the response SHALL render the setup form HTML (preserving current first-boot behavior)

#### Scenario: Reaches admin_exists_observed cache when populated

- **WHEN** the GET /setup handler runs and `admin_exists_observed` is already `true` (set either by an earlier middleware check or by the wizard's create_admin call)
- **THEN** the handler SHALL consult that cached value rather than re-querying the database, matching the optimization the `cache-has-admin-flag` change introduced for the middleware
