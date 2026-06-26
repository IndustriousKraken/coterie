## Why

`Notifications::send_dues_reminders`
(`src/service/billing_service/notifications.rs:272-473`) is the daily
runner that emails members before their dues lapse. It has **zero test
coverage** — it is referenced only by `src/jobs/billing_runner.rs:78`.
`tests/auto_renew_alert_test.rs` and `tests/expiration_test.rs` cover the
*charge* and *expiry* paths, never the *reminder* path.

The function is non-trivial branching logic with four distinct outcomes
(documented in its own doc comment at lines 259-271) and a per-cycle
idempotency flag, none of which is asserted anywhere:

1. **Manual billing** → "pay your dues" reminder.
2. **Auto-renew + card valid at charge time + monthly period** → skip
   silently (the charge will just happen), and deliberately do **not**
   set `dues_reminder_sent_at` so the member stays eligible if their card
   or mode changes mid-window.
3. **Auto-renew + card valid + yearly period** → "we're about to
   auto-charge you" renewal notice.
4. **Auto-renew + card expired/missing by charge time** → the manual
   reminder **plus a card-invalid callout** so the member knows
   auto-charge won't save them.

The highest-consequence branch is case 4: a member who believes
auto-renew has them covered, whose card is actually dead, currently
depends entirely on untested routing to be warned before they silently
lapse to `Expired`.

### Contract change (why spec lane, not issue lane)

The reminder routing is **not described in any canonical spec**. The
`recurring-billing` capability specifies the billing runner, dunning,
idempotency, AdminAlert, and expiry sweep, but says nothing about which
of the four reminder outcomes each member receives. Pinning this
behavior with tests asserts a new capability invariant, so it lands in
the spec lane: this change adds a `recurring-billing` requirement
codifying the four-case routing and the case-2 "don't mark" idempotency
nuance, and the tests assert it.

## What Changes

- Add a `recurring-billing` requirement specifying dues-reminder routing
  by billing mode and card validity, plus the per-cycle idempotency flag.
- Add `#[tokio::test]` cases to a new `tests/dues_reminder_test.rs`
  exercising the four routing cases, the lifetime skip, and idempotency.

## Impact

- `tests/dues_reminder_test.rs` (new) — reminder routing tests built with
  a recording `EmailSender`, reusing the member/card/settings seeding
  patterns from `tests/auto_renew_alert_test.rs`.
- `openspec/specs/recurring-billing/spec.md` (via this change's delta) —
  one new requirement.
- No production code change.
