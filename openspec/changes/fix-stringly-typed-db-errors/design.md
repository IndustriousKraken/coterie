## Context

The `AppError::Database` variant in `src/error.rs` currently holds a `String`. This string is constructed eagerly via the `From<sqlx::Error>` implementation. While this is sufficient for returning generic 500 errors to the client and logging the raw string, it makes it impossible for the `Service` layer to match on specific database errors (like `sqlx::Error::Database` where the code indicates a unique constraint violation) to return a 400 Bad Request or a domain-specific error. 

## Goals / Non-Goals

**Goals:**
- Wrap `sqlx::Error` directly in `AppError::Database`.
- Allow upstream callers to inspect the original `sqlx::Error`.
- Maintain the current behavior of returning a generic 500 error to the client to avoid leaking database internals.
- Maintain error logging visibility.

**Non-Goals:**
- Refactoring the entire error handling architecture.
- Adding specific domain error variants for every possible database error right now.

## Decisions

1. **Change Variant Signature:**
   Change `AppError::Database(String)` to `AppError::Database(#[from] sqlx::Error)` or simply `AppError::Database(sqlx::Error)`.
   Given we use `thiserror`, we will change the variant to:
   ```rust
   #[error("Database error: {0}")]
   Database(#[from] sqlx::Error),
   ```
   We can then remove the manual `From<sqlx::Error> for AppError` implementation since `thiserror` will generate it.

2. **Handle manual constructions:**
   There are places in the codebase that manually construct `AppError::Database(e.to_string())` for non-sqlx errors (e.g., UUID parsing errors). These will be updated to use `AppError::Internal(e.to_string())` or another appropriate variant, as they are not true `sqlx::Error`s.

3. **IntoResponse modification:**
   The `IntoResponse` implementation for `AppError::Database` currently logs the `msg`. It will be updated to log `err.to_string()` instead.

## Risks / Trade-offs

- **Compilation Errors:** This change will break compilation anywhere `AppError::Database` is manually constructed with a String. 
  - **Mitigation:** The compiler will easily catch all these instances, and they can be mechanically updated to use `AppError::Internal` or by propagating the correct `sqlx::Error`.