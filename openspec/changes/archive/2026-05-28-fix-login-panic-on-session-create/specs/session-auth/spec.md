# session-auth Specification Delta

## ADDED Requirements

### Requirement: Login surfaces a clean 500 if session creation fails

Both `POST /login` (web handler in `src/web/templates/auth.rs::login_handler`) and `POST /auth/login` (JSON handler in `src/api/handlers/auth.rs::login`) SHALL handle a `create_session` error as a `500 Internal Server Error` with the generic login-failed body. Neither handler SHALL panic on the error path: `unwrap()` / `expect()` on the `Result` returned by `AuthService::create_session` is forbidden. The error SHALL be logged at error level via `tracing` with enough context for an operator to correlate with the underlying database failure. No session cookie SHALL be set on the response.

This is a defense against panic-based DoS: the password is by definition attacker-supplied, and the underlying `sessions` INSERT can fail under DB contention or a transient SQLite `database is locked`. Returning 500 lets the caller retry; panicking drops the connection and leaves no actionable trail.

#### Scenario: Web login returns 500 on session-create failure

- **WHEN** `login_handler` (web) verifies a member's password successfully but `auth_service.create_session(...)` returns `Err`
- **THEN** the handler SHALL respond `500 Internal Server Error` with a `LoginResponse { success: false, error: Some("Login failed. Please try again."), redirect: None }` body, SHALL log the underlying error via `tracing::error!`, and SHALL NOT emit a `Set-Cookie: session=...` header

#### Scenario: JSON login returns 500 on session-create failure

- **WHEN** `login` (JSON, in `src/api/handlers/auth.rs`) verifies a member's password successfully but `auth_service.create_session(...)` returns `Err`
- **THEN** the handler SHALL surface the error via `?` (or an equivalent match) so the framework's `AppError` mapping produces a `500 Internal Server Error` response; no session cookie SHALL be set
