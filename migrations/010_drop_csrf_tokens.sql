-- CSRF tokens are now stateless (HMAC-signed) rather than DB-stored.
-- See src/auth/csrf.rs. The old table had a per-session PRIMARY KEY
-- that caused tokens to clobber each other across browser tabs — the
-- new design has no such problem.

DROP TABLE IF EXISTS csrf_tokens;
