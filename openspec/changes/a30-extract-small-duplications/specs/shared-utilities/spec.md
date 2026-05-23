## ADDED Requirements

### Requirement: capitalize_first is defined once

`fn capitalize_first(s: &str) -> String` SHALL exist exactly once in the codebase, at `src/util/string.rs`, with visibility `pub`. The duplicate copies at `src/repository/basic_type_repository.rs`, `src/web/portal/admin/types.rs`, and `src/service/basic_type_service.rs` SHALL be removed; each call site SHALL import via `use crate::util::string::capitalize_first;`.

#### Scenario: grep finds exactly one definition

- **WHEN** `grep -rn "fn capitalize_first" src/` is run after this change
- **THEN** exactly one match SHALL appear, at `src/util/string.rs`

### Requirement: generate_token and hash_token are defined once each

The 32-byte-hex random token generator `fn generate_token() -> String` SHALL be defined exactly once at `src/auth/tokens.rs` with visibility `pub`. The SHA256-hex hasher `fn hash_token(token: &str) -> String` SHALL also be defined exactly once in the same module.

The duplicate copies in `src/auth/pending_login.rs`, `src/auth/mod.rs` (for `generate_token`), `src/auth/email_tokens.rs`, and `src/auth/session.rs` (for `hash_token`) SHALL be removed. Each call site SHALL import via `use crate::auth::tokens::{generate_token, hash_token};` (or whichever subset it needs).

The unrelated `pub async fn generate_token(&self, session_id: &str)` on the CSRF type at `src/auth/csrf.rs` is NOT a duplicate (different signature on a different type) and SHALL be left in place unchanged.

#### Scenario: grep finds exactly one free-function definition of each

- **WHEN** `grep -rn "^fn generate_token" src/ "    fn generate_token" src/` is run (catching both top-level and method definitions excluding the CSRF method via its `pub async` keyword)
- **THEN** exactly one free-function definition SHALL exist at `src/auth/tokens.rs`
- **WHEN** the same check is done for `fn hash_token`
- **THEN** exactly one definition SHALL exist at `src/auth/tokens.rs`

### Requirement: test_result_html is defined once with an id parameter

`fn test_result_html(id: &str, ok: bool, detail: &str) -> axum::response::Html<String>` SHALL be defined exactly once at `src/web/portal/admin/test_result.rs` with visibility `pub`. The duplicate copies at `src/web/portal/admin/email.rs` and `src/web/portal/admin/discord.rs` SHALL be removed. Call sites SHALL be updated to pass the id they need (`"test-result"` for email, `"discord-test-result"` for discord).

#### Scenario: Both admin pages render the same HTML shape as before

- **WHEN** the email-test action and the discord-test action are exercised after this change
- **THEN** each SHALL produce HTML byte-for-byte identical to the pre-change output (same div ids, same class lists, same SVG markup)

### Requirement: parse_member_status remains intentionally duplicated

The two `parse_member_status` implementations SHALL be left as separate functions:

- `MemberRepository::parse_member_status` returns `Result<MemberStatus>` and errors on invalid input (runtime use).
- `bin/seed::parse_member_status` returns `MemberStatus` and defaults to `Pending` on invalid input (permissive use for seed fixtures).

A one-line comment SHALL be added to the seed copy noting that the permissive behavior is intentional and pointing at the strict version for runtime parsing.

#### Scenario: Seed binary's parse_member_status keeps its fallback

- **WHEN** the seed binary encounters an unknown status string after this change
- **THEN** it SHALL still produce `MemberStatus::Pending` (NOT error), and a comment in the source SHALL document this behavior
