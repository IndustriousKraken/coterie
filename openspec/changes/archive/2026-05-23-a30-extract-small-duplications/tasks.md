## 1. Extract capitalize_first

- [x] 1.1 Create `src/util/mod.rs` containing `pub mod string;`.
- [x] 1.2 Create `src/util/string.rs` containing the `pub fn capitalize_first(s: &str) -> String` body (copy verbatim from any of the three current copies — they're byte-for-byte identical).
- [x] 1.3 Add `mod util;` (or `pub mod util;` if needed by tests) to `src/lib.rs` (or `src/main.rs`, wherever module declarations live in this codebase).
- [x] 1.4 Delete the local `fn capitalize_first` from `src/repository/basic_type_repository.rs:295`. Add `use crate::util::string::capitalize_first;` to its imports.
- [x] 1.5 Same for `src/web/portal/admin/types.rs:561`.
- [x] 1.6 Same for `src/service/basic_type_service.rs:89`.
- [x] 1.7 `cargo build` clean.
- [x] 1.8 `grep -rn "fn capitalize_first" src/` returns exactly one match (at `src/util/string.rs`).

## 2. Extract generate_token + hash_token

- [x] 2.1 Create `src/auth/tokens.rs` containing both:
  ```rust
  pub fn generate_token() -> String { /* copy verbatim from any current copy */ }
  pub fn hash_token(token: &str) -> String { /* copy verbatim from any current copy */ }
  ```
- [x] 2.2 Add `pub mod tokens;` to `src/auth/mod.rs`.
- [x] 2.3 Delete the local `fn generate_token` from `src/auth/pending_login.rs:186` and `src/auth/email_tokens.rs:180`. Replace with `use super::tokens::generate_token;` (or `crate::auth::tokens::generate_token`).
- [x] 2.4 Delete the local `fn generate_token` from `src/auth/mod.rs:142` — but be careful, this one is in the same module as the new `tokens` submodule. After moving, any caller inside `src/auth/mod.rs` references `tokens::generate_token`.
- [x] 2.5 Delete the local `fn hash_token` from `src/auth/pending_login.rs:193`, `src/auth/session.rs:151`, `src/auth/email_tokens.rs:187`. Replace with the same import pattern.
- [x] 2.6 LEAVE `src/auth/csrf.rs:50`'s `pub async fn generate_token(&self, session_id: &str)` alone — it's a method on a struct, not a duplicate.
- [x] 2.7 `cargo build` clean.
- [x] 2.8 Grep verification:
  - `grep -rn "^fn generate_token" src/` returns nothing (no more top-level definitions outside tokens.rs — wait, the new one IS top-level, so this should return exactly 1 at tokens.rs).
  - Actually the right grep: `grep -rn "fn generate_token" src/ | grep -v csrf.rs | grep -v test` should return exactly the definition in `tokens.rs` plus its usages.

## 3. Extract test_result_html

- [x] 3.1 Create `src/web/portal/admin/test_result.rs`:
  ```rust
  use axum::response::Html;

  pub fn test_result_html(id: &str, ok: bool, detail: &str) -> Html<String> {
      // body adapted from existing copies, with the div id parameterized
  }
  ```
  Copy the body from one of the existing copies; replace the hardcoded `id="test-result"` (or `"discord-test-result"`) with `id="{id}"` in the format string and pass the parameter.
- [x] 3.2 Add `mod test_result;` to `src/web/portal/admin/mod.rs` (or wherever sibling modules are declared).
- [x] 3.3 Delete the local `fn test_result_html` from `src/web/portal/admin/email.rs:328`. Update its callers to pass `"test-result"` as the first arg.
- [x] 3.4 Delete the local `fn test_result_html` from `src/web/portal/admin/discord.rs:276`. Update its callers to pass `"discord-test-result"` as the first arg.
- [x] 3.5 `cargo build` clean.
- [x] 3.6 If there are integration tests that scrape the rendered HTML, run them and confirm the output is unchanged (same id, same class list, same SVG markup).

## 4. Document the intentional non-extraction of parse_member_status

- [x] 4.1 Add a comment above `bin/seed.rs:163`'s `fn parse_member_status`:
  ```rust
  // Permissive parsing for seed fixtures — falls back to Pending on
  // unknown input. For runtime parsing that errors on invalid input,
  // see MemberRepository::parse_member_status.
  fn parse_member_status(s: &str) -> MemberStatus { ... }
  ```
- [x] 4.2 No code changes to either implementation.

## 5. Validation

- [x] 5.1 `cargo build --features test-utils` — clean.
- [x] 5.2 `cargo test --features test-utils` — all tests pass. (Six pre-existing snapshot test failures in `tests/member_template_snapshots.rs` — golden HTML references CDN URLs while the templates now self-host htmx/alpine. Reproduce on master HEAD, so unrelated to this change.)
- [x] 5.3 `cargo clippy --features test-utils -- --deny warnings` — clean. (66 pre-existing errors on master HEAD; this branch has the same 66, so this change introduces no new clippy errors.)
- [x] 5.4 `cargo fmt --check` — clean. (Codebase has 2027 pre-existing fmt diffs on master HEAD; this branch has 2025, so this change does not add fmt drift.)
- [x] 5.5 Final grep sweep:
  - `grep -rn "fn capitalize_first" src/` — exactly 1 match.
  - `grep -rn "fn hash_token" src/` — exactly 1 match (at tokens.rs).
  - For `generate_token`: 2 matches total (the new free function in tokens.rs + the CSRF method in csrf.rs, which is intentional).
  - `grep -rn "fn test_result_html" src/` — exactly 1 match (at test_result.rs).
