# Tasks

## 1. Matching

- [x] 1.1 `src/repository/member_repository.rs`: make `find_by_email` and `find_by_username` compare case-insensitively (`COLLATE NOCASE` on the comparison, or `lower()` on both sides — choose one and use it for both). Every caller inherits it; do not add case handling at call sites.
- [x] 1.2 (Verified: `EXPLAIN QUERY PLAN` shows `SEARCH … USING INDEX idx_members_email_nocase` / `idx_members_username_nocase`.) Confirm the queries can use an index after the change — a `lower()` comparison needs the expression index from 2.1, a `COLLATE NOCASE` comparison needs a NOCASE index. Member count is small today, but a login path should not quietly become a table scan.
- [x] 1.3 Audit every other query that compares `email` or `username` (admin search, uniqueness checks, anything in `bulk_import.rs`) and bring it onto the same comparison, so there is one notion of equality.

## 2. Uniqueness

- [x] 2.1 Migration: add unique indexes on the case-folded email and username, using the same folding as 1.1. The existing byte-exact `UNIQUE` constraints may stay; the new indexes are strictly stronger.
- [x] 2.2 The migration must fail loudly rather than proceed if existing data violates the new constraint. Production had no case-only collisions on 2026-09-30, but another instance might; a migration that silently dropped or altered rows would be far worse than one that stops.
- [x] 2.3 Confirm signup's existing `is_unique_violation` handling and bulk import's duplicate reporting both catch the new index's violations and report them as duplicates, not as internal errors.

## 3. Preservation

- [x] 3.1 Do not lowercase on write anywhere. Do not rewrite existing rows.

## 4. Tests

- [x] 4.1 Web login succeeds for a mixed-case stored email typed in lowercase, and for a username typed in a different case. API login likewise for email.
- [x] 4.2 A password reset requested in a different case is issued to the member.
- [x] 4.3 A signup with a username or email differing from an existing one only by case is refused as a duplicate.
- [x] 4.4 Bulk import reports a case-variant row as already existing.
- [x] 4.5 A public donation with a case-variant email attaches to the member. Covered at the repository level: the donate handler's attachment is a direct `find_by_email` call, asserted case-insensitive in `repository_lookups_ignore_case`; exercising the full handler would need a Stripe fake for no additional coverage of this change.
- [x] 4.6 Stored and displayed identifiers keep their original case after login and after an attempted duplicate.
- [x] 4.7 The migration refuses to run against a fixture containing two emails differing only by case.
- [x] 4.8 A wrong password for a case-variant identifier is still denied as `bad_password` — case-insensitive matching must not loosen the password check.
