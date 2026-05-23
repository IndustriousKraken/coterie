## Why

The architecture pass surfaced several small functions duplicated across files with identical (or near-identical) bodies. After filtering noise (trait impls, distinct types sharing method names, etc.), four real extractions remain:

- `capitalize_first(s: &str) -> String` — three byte-for-byte identical copies across `basic_type_repository.rs`, `portal/admin/types.rs`, `service/basic_type_service.rs`.
- `generate_token() -> String` — three byte-for-byte identical copies across `auth/pending_login.rs`, `auth/mod.rs`, `auth/email_tokens.rs`. All produce 32 random bytes hex-encoded. (The fourth match — `csrf.rs`'s `pub async fn generate_token(&self, session_id: &str)` — is a different signature on a different type and NOT a duplicate.)
- `hash_token(token: &str) -> String` — three byte-for-byte identical copies across `auth/pending_login.rs`, `auth/session.rs`, `auth/email_tokens.rs`. All produce SHA256 hex digest of the input.
- `test_result_html(ok: bool, detail: &str) -> Html<String>` — two near-identical copies across `portal/admin/email.rs` and `portal/admin/discord.rs`. Only difference: the wrapping `<div>`'s `id` attribute (`"test-result"` vs `"discord-test-result"`). Extracted version takes the id as a parameter.

One finding is explicitly NOT being extracted:

- `parse_member_status(s: &str)` exists in two places — `repository/member_repository.rs:216` returns `Result<MemberStatus>` (errors on invalid input, runtime use), and `bin/seed.rs:163` returns `MemberStatus` (defaults to `Pending` on invalid input, permissive for seed data). Different contracts; intentional. The seed binary's permissive parsing is the right thing for seed fixtures.

## What Changes

- **New `src/util/` module** (currently doesn't exist):
  - `src/util/mod.rs` — module declarations.
  - `src/util/string.rs` — `pub fn capitalize_first(s: &str) -> String`.
- **New `src/auth/tokens.rs`** (or extend an existing auth module):
  - `pub fn generate_token() -> String`.
  - `pub fn hash_token(token: &str) -> String`.
- **New `src/web/portal/admin/test_result.rs`** (or place inside an existing portal/admin shared module):
  - `pub fn test_result_html(id: &str, ok: bool, detail: &str) -> Html<String>` — takes the wrapping div id as a parameter.
- **Update call sites** to import the extracted functions; remove the duplicate definitions.
- **Document the intentional non-extraction**: leave `parse_member_status` as two separate functions, but add a one-line comment in `bin/seed.rs`'s copy explaining that it's deliberately permissive vs `MemberRepository::parse_member_status`.

## Capabilities

### New Capabilities
- `shared-utilities`: small reusable helpers (`capitalize_first`, `generate_token`, `hash_token`, `test_result_html`) live in shared modules rather than being duplicated per call site.

### Modified Capabilities
None.

## Impact

- **Code**: net negative line count (deleting 8 duplicate function bodies, adding 4 canonical versions).
- **Wire shape**: zero runtime change.
- **Tests**: existing tests pass unchanged.
- **Risk**: low. Each extraction is isolated and the bodies are byte-for-byte identical (or near-identical for `test_result_html`).
- **Dependency**: none. Independent of all other queued changes.
