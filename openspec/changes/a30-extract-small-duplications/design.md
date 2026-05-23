## Context

Four small functions are duplicated across files. The architecture finding numbers:

- `capitalize_first` — duplicated 3×, identical bodies
- `generate_token` — duplicated 3× (within `src/auth/`), identical bodies
- `hash_token` — duplicated 3× (within `src/auth/`), identical bodies
- `test_result_html` — duplicated 2×, near-identical (only the wrapping div's id differs)

Plus one finding that's a documented non-extraction:

- `parse_member_status` — two implementations with different contracts (`Result<MemberStatus>` strict vs `MemberStatus` permissive). Keep separate.

## Goals / Non-Goals

**Goals:**
- One canonical implementation per extracted function.
- Call sites updated to import from the canonical location.
- No introduction of cross-module visibility issues that complicate later refactoring.

**Non-Goals:**
- Refactoring the call sites beyond the swap-to-import.
- Touching `parse_member_status` (the intentional dual implementation stays).
- Extracting things from JavaScript files in `examples/baduk-club-site/` (out of scope per user direction — three example frontends each re-implementing the Coterie API is expected).

## Decisions

### D1. Module placement

- **`capitalize_first`**: new `src/util/string.rs` module. The codebase doesn't have a `util/` directory yet; creating it gives a home for future small extractions. Path: `crate::util::string::capitalize_first`.
- **`generate_token` / `hash_token`**: both go in `src/auth/tokens.rs`. All three current locations are inside `src/auth/`, so the canonical home there is natural. Path: `crate::auth::tokens::{generate_token, hash_token}`.
- **`test_result_html`**: `src/web/portal/admin/test_result.rs`. This is admin-area-only, used by two integration settings pages (email + discord). Path: `crate::web::portal::admin::test_result::test_result_html`.

### D2. test_result_html signature

Current bodies differ only in:
```html
id="test-result"        <!-- email -->
id="discord-test-result" <!-- discord -->
```

New signature: `pub fn test_result_html(id: &str, ok: bool, detail: &str) -> Html<String>`. Callers pass the id they need.

### D3. Visibility

All extracted functions are `pub` so any module in the crate can import them. They're small utilities; gate them tighter only if a specific reason arises (none today).

### D4. parse_member_status: documented dual implementation

The strict version (`MemberRepository::parse_member_status` returning `Result<MemberStatus>`) and the permissive version (`bin/seed::parse_member_status` returning `MemberStatus` with a `Pending` fallback) serve different purposes. Adding a one-line comment to the seed copy ("permissive parsing for seed fixtures; for runtime parsing, see MemberRepository::parse_member_status") records the distinction so a future reader doesn't try to "consolidate" them and break seed behavior.

### D5. Single PR, low risk

All four extractions are independent of each other. They land together because the architecture pass found them together; splitting into four PRs would be more churn for no benefit.

## Risks / Trade-offs

- **Risk**: the autocoder consolidates two callers but misses a third. → Mitigation: tasks include a final grep-sweep for each function name to confirm only the canonical definition remains.
- **Risk**: a previously-private function becomes `pub`, exposing it more broadly than needed. → Mitigation: these are utility functions; `pub` is appropriate. The visibility "leak" is intentional.
- **Trade-off**: introduces a new top-level module (`src/util/`). Minor cost; pays off the first time another small extraction needs a home.

## Migration Plan

Single PR.

1. Create `src/util/mod.rs` + `src/util/string.rs` with `capitalize_first`. Update `src/lib.rs` (or `src/main.rs`) to add `mod util;`.
2. Create `src/auth/tokens.rs` with `generate_token` + `hash_token`. Update `src/auth/mod.rs` to add `pub mod tokens;`.
3. Create `src/web/portal/admin/test_result.rs` with `test_result_html(id, ok, detail)`. Update parent `mod.rs` to declare it.
4. Update each call site:
   - Replace local definition with `use crate::util::string::capitalize_first;` (or the corresponding path).
   - Confirm `cargo build` clean after each replacement.
5. Add the one-line comment to `bin/seed.rs::parse_member_status`.
6. Grep-sweep: confirm each function name now has exactly one definition (the canonical one).
7. `cargo test --features test-utils`, `cargo clippy --features test-utils -- --deny warnings`, `cargo fmt --check`.
