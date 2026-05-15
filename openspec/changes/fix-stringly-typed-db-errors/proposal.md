## Why

Currently, `AppError::Database` eagerly converts `sqlx::Error` to a string using a `From` trait implementation. This "stringly-typed" approach prevents services from programmatically reacting to specific database errors, such as unique constraint violations (e.g., duplicate email during registration). This change will wrap the `sqlx::Error` directly, enabling richer error handling without parsing raw SQL error strings.

## What Changes

- Modify `AppError::Database` in `src/error.rs` to wrap `sqlx::Error` directly rather than converting it to a string.
- Update instances in the codebase where `AppError::Database` is instantiated manually with a string to use a separate error variant (e.g., `AppError::Internal` or a new data-specific variant) if it's not a direct `sqlx::Error`.
- Ensure that the HTTP response translation in `IntoResponse` still logs the underlying error detail appropriately but does not expose raw SQL errors to the client.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `src/error.rs` will be modified.
- Services and repositories that explicitly map strings into `AppError::Database` will need to be updated.
- Future changes to the codebase will be able to utilize programmatic error handling of DB errors by matching on `sqlx::Error` variants.