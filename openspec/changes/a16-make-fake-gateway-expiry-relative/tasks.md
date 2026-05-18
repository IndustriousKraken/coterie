## 1. Verify no test depends on the literal 2030

- [ ] 1.1 Run `grep -rn "exp_year" /Users/rab/Dropbox/code/coterie/tests/ /Users/rab/Dropbox/code/coterie/src/`. Confirm the only meaningful match is `src/payments/fake_gateway.rs:273` (the line being changed). If any test asserts on the literal value `2030`, plan a parallel adjustment of that assertion in the same change.

## 2. Make the fix

- [ ] 2.1 In `src/payments/fake_gateway.rs`, ensure `use chrono::{Datelike, Utc};` is in scope at the top of the file (extend an existing chrono import if one exists; `Datelike` is what provides `.year()` on `DateTime<Utc>`).
- [ ] 2.2 In `retrieve_payment_method`, change `exp_year: 2030,` to `exp_year: Utc::now().year() + 5,`. Leave `exp_month: 12` unchanged.

## 3. Verify

- [ ] 3.1 `cargo build --all-targets --features test-utils` — clean.
- [ ] 3.2 `cargo test --features test-utils` — full suite passes.
- [ ] 3.3 Grep verify: `grep -n "2030" src/payments/fake_gateway.rs` returns nothing (the only `2030` in that file was the line we just changed).
