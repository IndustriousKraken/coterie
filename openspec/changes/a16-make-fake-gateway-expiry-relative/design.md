## Context

`FakeStripeGateway::retrieve_payment_method` returns a default `PaymentMethodDetails` when no test has queued a specific response. That default represents "a valid Visa card ending in 4242, expiring December 2030." It's the fixture every test implicitly relies on if they don't override it.

The hardcoded year 2030 was reasonable at the time of writing (~4–5 years of headroom). But it's a future-fragility trap: the same shape as the test-anchor drift `a14` and `a15` fixed. Replacing with a runtime-relative computation eliminates the trap permanently for one line of code.

## Goals / Non-Goals

**Goals:**
- The fake's default "valid card" response stays valid no matter when the test runs.
- Pattern is documented in the saved-card-management spec so future fixtures avoid the same trap.

**Non-Goals:**
- Refactoring how the fake gateway stores or computes other fields.
- Adding configuration knobs for "how many years out" the default expiry is.
- Changing the fake's behavior in any way visible to tests other than this one field.
- Touching the real `RealStripeGateway` — it doesn't have hardcoded expiry; it forwards what Stripe returns.

## Decisions

### D1. `chrono::Utc::now().year() + 5`

Five years matches the original 2030 (which was ~5 years out when written). Plenty of headroom; well beyond any reasonable test-suite runtime variance.

Alternatives considered:
- `+10`: more headroom but also further outside the "realistic card expiry" range. Real cards usually expire 3–5 years out. Stay close to realism.
- `+1`: too close to "now" — tests that simulate "card expires next year" semantics could conflict.
- A `const` like `DEFAULT_VALID_YEARS_OUT: i32 = 5`: nice-to-read but adds module-level noise for a value used in exactly one place. Inline is fine.

### D2. `exp_month: 12` stays

December as the expiry month means the card is valid through Dec 31 of (year + 5). Maximum month-grain headroom. No reason to change.

### D3. Import `chrono::Datelike`

`Utc::now()` returns `DateTime<Utc>`, which has a `.year()` method available only when the `chrono::Datelike` trait is in scope. Easy to forget; the task list calls it out.

### D4. Spec delta lands on `saved-card-management`

The principle ("test fixtures representing valid saved cards SHALL use runtime-relative expiry dates") fits the saved-card-management capability. Parallel to the `a14`/`a15` rule that lives under `admin-events`.

### D5. No test changes expected

No existing test asserts specifically on `exp_year == 2030`. (Quick verification step in tasks.) If any test does, the assertion needs to be made relative too — but this is unlikely; tests of card-validity logic use deliberate hardcoded boundary inputs (per the earlier sweep's bucket B), not the fake gateway's default response.

## Risks / Trade-offs

- **Risk**: a test exists somewhere that asserts on the exact year value. → **Mitigation**: grep for `exp_year` and `2030` in tests/ before changing. If matches exist, plan an assertion shift in lock-step.
- **Trade-off**: a tiny bit of computational overhead per call (`Utc::now()` + arithmetic). Imperceptible; only runs in test builds.

## Migration Plan

Single PR. One line.

1. `grep -rn "exp_year" tests/ src/` to confirm no test asserts on the literal `2030`.
2. Edit `src/payments/fake_gateway.rs::retrieve_payment_method` per the proposal.
3. Add the missing import if `Datelike` isn't already in scope.
4. `cargo build --all-targets --features test-utils`.
5. `cargo test --features test-utils`.
