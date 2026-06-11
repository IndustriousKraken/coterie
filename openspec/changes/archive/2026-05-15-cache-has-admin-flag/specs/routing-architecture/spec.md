## ADDED Requirements

### Requirement: Setup-redirect check is process-cached after first positive observation

`AppState` SHALL hold an `admin_exists_observed: Arc<AtomicBool>` flag, initialized to `false`. The `require_setup` middleware SHALL consult this flag before querying the database; once set to `true`, the middleware SHALL forward the request without querying. The middleware SHALL set the flag to `true` the first time it observes any admin row in the database.

The flag SHALL be process-local; a multi-process deployment SHALL have each process independently arm its own cache. The flag SHALL be sticky for the lifetime of the process — it SHALL NOT be cleared by any application-level operation. Operators who manually remove admin status from every member via direct SQL SHALL restart the server to re-arm the setup-redirect path.

The setup-wizard handler (POST `/setup`) SHALL set the flag to `true` immediately after successfully creating the first admin, so the very next request bypasses the redundant DB query.

#### Scenario: First request after setup observes admin and arms cache

- **WHEN** an instance has just completed first-boot setup and the very next request arrives at the middleware
- **THEN** the middleware SHALL forward the request, AND `admin_exists_observed` SHALL be `true` afterward (set proactively by the setup-wizard handler or by the middleware itself on a positive DB lookup)

#### Scenario: Subsequent requests skip the DB query

- **WHEN** `admin_exists_observed` is `true` and a non-static, non-setup-prefix request arrives
- **THEN** the middleware SHALL forward the request without running `SELECT 1 FROM members WHERE is_admin = 1 LIMIT 1`

#### Scenario: First-boot redirect still fires before any admin exists

- **WHEN** a fresh-install instance with no admin yet receives a request to a non-static, non-setup-prefix path
- **THEN** the middleware SHALL run the DB query, observe no admin, and respond with a redirect to `/setup`; `admin_exists_observed` SHALL remain `false`

#### Scenario: Concurrent first-boot requests converge

- **WHEN** two concurrent requests reach the middleware before any admin exists, while a third request is concurrently completing the setup-wizard handler
- **THEN** each first-boot request SHALL independently consult the DB, the wizard's admin creation SHALL be serialized by `setup_lock`, and after the wizard completes the next request SHALL observe `admin_exists_observed = true` (set either by the wizard's proactive store or by the middleware's own observation)

#### Scenario: Direct-SQL admin removal does not re-trigger redirect without restart

- **WHEN** an operator directly clears `is_admin` on every member row via DB tooling outside the application
- **THEN** the cached `admin_exists_observed = true` SHALL persist; subsequent requests SHALL continue to forward (not redirect to `/setup`); recovery SHALL require a server restart so the flag re-initializes to `false`
