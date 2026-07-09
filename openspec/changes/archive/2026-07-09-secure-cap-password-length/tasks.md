# Tasks

## 1. Add a maximum-length bound to the password validator

- [x] 1.1 In `src/auth/mod.rs::validate_password`, add a check that
  rejects passwords longer than 128 characters, returning
  `Err("Password must be at most 128 characters")`. Place it alongside
  the existing minimum-length check; leave the minimum and the
  upper/lower/digit rules unchanged.

## 2. Tests

- [x] 2.1 Add a unit test in `src/auth/mod.rs` asserting
  `validate_password` returns `Err` for a password of length 129 (e.g. a
  129-char string that otherwise satisfies the complexity rules) and
  returns `Ok` for a valid 128-char password.
- [x] 2.2 Add a unit test asserting a multi-kilobyte password (e.g.
  10_000 chars) is rejected by `validate_password`, documenting the
  Argon2 DoS guard.
