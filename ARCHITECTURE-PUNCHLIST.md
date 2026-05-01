# Architecture punchlist — round 2

Fresh review after the F1–F6 round (Payment domain sum types — `Payer` /
`PaymentKind` / `StripeRef` — landed). The codebase is in good shape
overall; this list is "what's next," not a first pass.

Work order (each step shrinks or simplifies the next):
**F5 → F1 → F6 → F4 → F3 → F2 → F7/F8.**

---

## F1 — `StripeRef` sum type exists but `refund_payment` still does prefix matching

**What's wrong.** `domain::StripeRef` is a clean three-variant enum and
`Payment.external_id: Option<StripeRef>`, yet `StripeClient::refund_payment`
re-enters the stringly-typed world: it takes `stored_stripe_id: &str` and
re-implements the same `starts_with("pi_")` / `"cs_"` / `"in_"` ladder that
`StripeRef::from_id` already encodes. The admin refund handler then
*unwraps the enum back to a string* (`payment.external_id.as_ref().map(|r| r.as_str())`)
just to feed it back in. Exactly the runtime check the F6-spirit retires.

**Where.**
- `src/payments/stripe_client.rs:421-455` (`refund_payment` signature and body)
- `src/web/portal/admin/members.rs:637-674` (caller unwrapping the enum)
- `src/domain/payment.rs:107-117` (the canonical parser)

**Why it matters.** A new Stripe id shape requires editing two prefix
ladders the compiler can't keep in sync. Refund is a money-mutating path
and should benefit from exhaustive `match`. Today the unknown-prefix case
returns `BadRequest` at runtime; with `match StripeRef`, the compiler
tells you about the gap at edit time.

**What to do.** Change the signature to
`refund_payment(&self, stripe_ref: &StripeRef, idempotency_key: &str)` and
`match` on the variant. The `Invoice → PaymentIntent` and
`CheckoutSession → PaymentIntent` resolutions become two arms. The admin
handler stops doing `r.as_str()` and passes the enum through.

**Effort + risk.** Small. Mechanical change, two files. Risk bounded —
gateway calls don't move, just dispatch shape.

---

## F2 — Admin members list filters/sorts in Rust over `list(1000, 0)`

**What's wrong.** `admin_members_page` calls `member_repo.list(1000, 0)`
then filters, sorts, and paginates *in memory*. Repo has no search,
status/type filter, or sort. Same pattern in `admin/announcements.rs:114`
and `admin/events.rs:123`. The `1000` is a hard cap masquerading as
pagination.

**Where.**
- `src/web/portal/admin/members.rs:74-222`
- `src/web/portal/admin/announcements.rs:114`
- `src/web/portal/admin/events.rs:123`
- `src/repository/member_repository.rs` (no search/sort/filter methods)

**Why it matters.**
1. **Correctness ceiling.** >1000 members and the admin page silently
   drops the rest. No error, no log.
2. **Sort by `format!("{:?}", m.status)`** couples sort key to the `Debug`
   derive — variant rename changes order silently.
3. **Validation in handlers.** Search predicate belongs in the repo or a
   service.

**What to do.** Add `MemberRepository::search(MemberQuery)` returning
`(Vec<Member>, total_count)`, where `MemberQuery` is a typed struct
(search string, optional `MemberStatus`, optional `MembershipType`, sort
field enum, sort order enum, limit, offset). Move the SQL into the repo;
handler shrinks to ~30 lines. Same shape for events and announcements.

**Effort + risk.** Medium. Non-trivial repo method + new struct, but
handler shrinks dramatically. Three admin pages, one at a time.

---

## F3 — Payment-record / payment-validation logic duplicated across four handlers

**What's wrong.** "Build a `Payment` from a request, validate
amount/cap/member/campaign, write it, conditionally extend dues +
reschedule" is implemented four times with subtly different shapes:

| Handler                           | File                                | Notes                                    |
| --------------------------------- | ----------------------------------- | ---------------------------------------- |
| `admin_record_payment_submit`     | `web/portal/admin/members.rs:861-1003` | Form-based, has `parse_dollars_to_cents` |
| `donate_api` (member)             | `web/portal/donations.rs:132-275`   | JSON, Pending-first if saved card        |
| `donate` (public)                 | `api/handlers/public.rs:442-547`    | JSON, Stripe Checkout only               |
| `create_manual` / `waive` (API)   | `api/handlers/payments.rs:240-416`  | JSON admin                               |

All four repeat: amount > 0 check, `MAX_PAYMENT_CENTS` check,
member-exists check (sometimes), campaign-exists check (sometimes),
`PaymentKind` construction, the same 12-field `Payment` struct, then
post-work branching on `PaymentKind::Membership`.

**Why it matters.** The four sites already disagree:
- `admin_record_payment_submit` validates campaign exists; `create_manual`
  validates campaign exists *only when one is supplied*; `donate_api`
  doesn't reject deleted campaigns mid-form (only inactive).
- Amount-positive check is `<= 0` in three sites and `< 0` in
  `parse_dollars_to_cents`.
- Audit-log action strings (`"manual_payment"`, `"manual_donation"`,
  `"manual_other"`) live in two places.

Adding the next payment modality means editing four files.

**What to do.** Create a `PaymentService::record_manual` (or split into
`record_membership` / `record_donation` / `record_other`) that takes a
typed input, does all validation, persists, runs dues/reschedule, emits
the audit log. Handlers become wire-format conversion → service call →
response shape.

**Effort + risk.** Medium. New service module + four handler rewrites.
Risk low: new service is unit-testable before handler swap.

---

## F4 — `WebhookDispatcher` writes raw SQL against `payments` and `processed_stripe_events`

**What's wrong.** Dispatcher takes a `db_pool: SqlitePool` purely to run
two raw queries: the `processed_stripe_events` idempotency claim/rollback
and the `charge.refunded` UPDATE/SELECT chain. The struct doc already
flags this as a TODO.

**Where.** `src/payments/webhook_dispatcher.rs:47, 101-113, 199-211, 618-709`.

**Why it matters.**
1. **Layering inconsistency.** Every other repo concern is behind a
   trait. The dispatcher is the one place reaching past the seam, making
   it the one piece of post-payment logic you can't test without a real
   SQLite pool.
2. **Domain rule trapped in dispatcher.** The `charge.refunded` query
   ("find local Payment whose `stripe_payment_id` matches charge's PI or
   invoice") is a domain rule living in webhook code. If
   `stripe_payment_id` storage shape changes, this is a separate edit.

**What to do.** Add three methods to `PaymentRepository`:
`find_by_pi_or_invoice(pi_id, invoice_id)`,
`mark_refunded_if_completed(payment_id)`, and a
`ProcessedEventsRepository` (or fold its two methods into a small new
trait). Drop `db_pool` from `WebhookDispatcher`. `handle_webhook`
signature unchanged.

**Effort + risk.** Small-to-medium. Three new repo methods; integration
tests under `tests/stripe_webhook_test.rs` cover the path.

---

## F5 — `MembershipType` and `MemberStatus` round-trip through `format!("{:?}", …)` and string match

**What's wrong.** Three patterns, all coupled to `Debug`:

1. **Sort/filter by string-debug.** `admin/members.rs:120, 125, 143-144,
   180-181, 449-450` use `format!("{:?}", m.status)` as both sort key and
   template DTO value.
2. **Form parsing string-match.** `admin/members.rs:499-505, 1354-1360,
   1372-1378` do
   `match form.membership_type.as_str() { "Regular" => MembershipType::Regular, … _ => MembershipType::Regular }`.
   Fall-through silently maps unknown values to `Regular`.
3. **JSON DTO** in `api/handlers/members.rs:106-107` deserializes via
   `serde_json::from_str(&format!("\"{}\"", dto.membership_type))`,
   leaning on the derived encoding.

**Where.** `src/web/portal/admin/members.rs` (multiple),
`src/api/handlers/members.rs:100-123`, plus matching strings in
`templates/admin/members.html`.

**Why it matters.** Rename in enum changes wire format silently. New
variant doesn't trigger compile error at parse sites — falls through to
default. Same runtime-check-vs-exhaustive-match line F6 was on the right
side of for `StripeRef`; these enums never got the same treatment.

**What to do.** Two things:
- Give `MembershipType` and `MemberStatus` `as_str()` / `from_str()`
  methods. The repo already has `parse_member_status` +
  `member_status_to_str` privately at `member_repository.rs:82-100` —
  promote those onto the enums. Have callers use those, not
  `format!("{:?}", …)` and not match-with-default.
- Replace `_ => MembershipType::Regular` with `_ => return BadRequest` in
  admin form handlers. Bad form values fail loudly, not silently
  downgrade.

**Effort + risk.** Small. Mostly mechanical.

---

## F6 — Three duplicate "fix dues_paid_until" handlers embed SQL

**What's wrong.** `admin_extend_dues`, `admin_set_dues`, and
`admin_expire_now` (`web/portal/admin/members.rs:1093-1278`) each issue
a raw `sqlx::query("UPDATE members SET dues_paid_until = ? …")` directly
from the handler against `service_context.db_pool`, bypassing
`MemberRepository::set_dues_paid_until_with_revival` which already
exists.

**Where.** `src/web/portal/admin/members.rs:1127-1131, 1184-1188, 1228-1238`.

**Why it matters.** Audit/event invariants get applied unevenly.
`admin_expire_now` already manually re-fetches member after raw SQL to
dispatch `MemberUpdated`; `admin_extend_dues` doesn't fire any
integration event even though dues going expired→active should trigger
Discord role re-sync. The repo method
`set_dues_paid_until_with_revival` (line 58 of `repository/mod.rs`) is
the right primitive — just isn't wired here.

**What to do.** Replace three raw queries with the repo call. Move
"extend by N months" arithmetic into a small helper or onto the service.

**Effort + risk.** Small. Repo method exists; you're deleting code.

---

## F7 — `BillingService` constructed per-request from a factory

**What's wrong.** Every billing-using handler calls
`state.service_context.billing_service(stripe_client.clone(), state.settings.server.base_url.clone())`
and gets a fresh `BillingService` per request (~10 callsites). Currently
fine because the struct is three Arc-of-trait fields and a String. But:
the moment a field with its own lifecycle (rate limiter, idempotency
cache, periodic in-memory backoff state) is added, per-request
construction silently loses that state.

**Where.** `src/service/mod.rs:114-132` (factory) + ~10 callsites
including `web/portal/payments.rs:449, 580`, `web/portal/donations.rs`,
`api/handlers/payments.rs:188, 318, 389`.

**Why it matters.** Today: zero impact, just allocation churn. Future:
subtle bug class. Worth fixing while the rule is still "billing has no
per-instance state."

**What to do.** Build one `Arc<BillingService>` at startup in `main.rs`
(already done at line 349 for the runner) and stick it on
`ServiceContext` or `AppState`. Drop the factory.

**Effort + risk.** Small. Mechanical replace at every callsite. 3/10
payoff today; cheap insurance.

---

## F8-extended — every admin-write JSON endpoint is a half-strength duplicate, not just activate/expire

Same pattern as F8 below, broader scope. The entire `/api/v1/...`
admin surface (members CRUD, events CRUD, announcements CRUD,
payments admin manual/waive, plus the whole `/admin/*` mount —
audit-log, expired-check, settings/*, types/*) was a half-strength
duplicate of the portal admin actions: skipped audit logs, skipped
integration events, skipped welcome emails, skipped session
invalidation. The portal admin pages (`/portal/admin/*`) own the full
side-effect chain. Outcome: delete everything admin-write that the
portal doesn't call. Keep:

- `POST /api/payments/webhook/stripe` — Stripe webhook.
- `/api/payments/cards/*` — saved-card management; the portal
  frontend `fetch()`-es these directly because Stripe.js needs JSON.
- `/public/*` — the documented public surface (signup, donate,
  read-only events / announcements, RSS, iCal).
- `/auth/login`, `/auth/logout` — login form posts.
- The portal's own `/portal/api/*` HTMX routes.

Removed handler files entirely: `admin.rs`, `members.rs`, `events.rs`,
`settings.rs`, `types.rs`. Trimmed `announcements.rs` to just
`private_count`. Trimmed `payments.rs` to just the webhook + saved-card
endpoints. Removed `require_admin` middleware (only the deleted JSON
admin routes used it; portal uses `require_admin_redirect` separately).

---

## F8 — JSON API `activate` / `expire` is a half-strength duplicate of the portal admin path

**What's wrong.** `api/handlers/members.rs::activate` (line 150) and
`expire` (line 173) flip status and dispatch one integration event each.
The portal counterparts (`web/portal/admin/members.rs::admin_activate_member`,
line 224) additionally: invalidate sessions, send the welcome email, log
to audit, and dispatch the *correct* `MemberActivated` event.

**Where.** `src/api/handlers/members.rs:150-195` vs
`src/web/portal/admin/members.rs:224-347`.

**Why it matters.** Two parallel ways to do the same admin action with
different side-effect coverage. Activation via JSON API gets no welcome
email, no force-logout of pending sessions, silent audit log. Discovered
after a support ticket.

**What to do.** Either (a) extract a
`MemberAdminService::activate(member_id) -> Result<Member>` that owns
the full side-effect chain, both handlers call it; or (b) remove the
JSON activate/expire endpoints if they aren't used. Start with (b) —
if no client uses them, deletion is the right answer.

**Effort + risk.** Small either way. Both handlers admin-gated.

---

---

# Round 3 (post-CSRF-lift architecture review)

Reviewer ran a fresh pass on the whole codebase after the round-2 work
(top-level CSRF, JSON admin deletion, secure-by-default). One critical
finding that **invalidates the headline guarantee of round 2**, plus a
handful of follow-ups.

## App summary the reviewer produced (sanity-check on intent)

Coterie is a single-tenant member-management app for clubs / non-profits
(NeonTemple is the first deployment). Three HTTP surfaces:

- **`/portal/*`** — Askama+HTMX server-rendered admin & member portal.
  This is the primary admin surface.
- **`/public/*`** — read-only feeds + the two public POSTs (signup,
  donate) for the marketing site.
- **`/api/*`** — narrowly scoped: Stripe webhook + portal-frontend
  saved-card endpoints (Stripe.js needs a JSON surface).

Layering: handler → service → repository → SQLite. Sum types in
`domain::payment` make Stripe references unambiguous (Payer /
PaymentKind / StripeRef). Background tokio tasks for billing runner,
audit pruning, Discord reconcile. Integration event bus decouples
auth/payment side effects (email, audit, Discord).

The reviewer's read of the architecture matches the design intent.

## F9 — CRITICAL: top-level CSRF doesn't cover the portal

**Where.** `src/api/mod.rs:89-92` layers `csrf_protect_unless_exempt`
on `api_app` inside `api::create_app`. Then `src/main.rs:410` does
`api_app.merge(web_app)` to add the portal/login/signup routes.

**The bug.** In axum 0.7, `Router::layer` applies to routes registered
on that router *at the time the layer is applied*. Routes added later
via `.merge()` do **not** inherit the layer. So every state-changing
route in `/portal/*` (admin CRUD, refunds, settings, audit export,
profile edits, security/TOTP, login, logout) currently has **no**
top-level CSRF protection.

**Evidence the author already noticed.** Both `web /logout` and
`api /auth/logout` carry inline manual `validate_token` calls — the
hand-patched fix for the two endpoints whose breakage was most visible.
Everything else relies on the top-level layer that doesn't reach it.

**Why this is bad.** This is exactly the threat round 2 was supposed to
close. Round-2 deleted the JSON admin surface so admin actions can only
happen through `/portal/admin/*`, then added a top-level CSRF layer so
nothing could be accidentally added without protection. The layer
doesn't cover the routes it was meant to protect.

**What to do.**
1. Move the CSRF layer to apply to the merged app. Either:
   - (a) Remove the `.layer(... csrf_protect_unless_exempt)` call from
     `api::create_app` and add it once in `main.rs` after
     `api_app.merge(web_app)`, alongside `require_setup`.
   - (b) Restructure so `create_app` returns the inner router and the
     CSRF layer is applied where the merge happens.
   Option (a) is the smallest change.
2. Fix the multipart hole. `csrf_protect_unless_exempt` rejects any
   non-`application/x-www-form-urlencoded` body without an
   `X-CSRF-Token` header (`security.rs:159-162`). The admin
   announcements/events forms post `enctype="multipart/form-data"` with
   `csrf_token` as a form field — they'd be rejected. Cleanest fix:
   change those forms to send the token via `X-CSRF-Token` header
   (Alpine/HTMX can stamp it from the meta tag) and keep the middleware
   simple.
3. Add an integration test: `POST /portal/admin/members/:id/update`
   without a CSRF token returns 403. Without this, F9 can regress
   silently again.

**Effort + risk.** ~1 hour for the layer move + multipart fix; the
test guards against silent regression. Order this before everything
else in round 3.

## F13 — Inline CSRF checks in logout handlers (tied to F9)

**Where.** `web /logout` and `api /auth/logout` call
`validate_token` inline, even though `/auth/logout` is also in
`CSRF_EXEMPT_PATHS`.

**Why it matters.** Once F9 is fixed and the top-level layer covers
everything, the inline checks become genuinely redundant *and*
contradict the exempt list (logout would be checked twice on `/api`
and inconsistently on `/web`). Pick one.

**What to do.** Remove the inline `validate_token` calls; remove
`POST /auth/logout` from `CSRF_EXEMPT_PATHS`. Logout becomes an
ordinary CSRF-protected action like everything else.

**Effort + risk.** Tiny. Do alongside F9 since they share context.

## F10 — `AppState` built twice; rate limiters not shared

**Where.** `api::create_app` calls `AppState::new(...)` to build state
for the API router; `main.rs` then builds *another* `AppState` to pass
to `create_web_routes` and to the setup-check middleware. The two
states have separate `login_limiter`, `money_limiter`, `setup_lock`.

**Why it matters.** The login rate limiter on the `/auth/login` route
(api side) is a different in-memory map than the limiter on the
`/login` route (web side). An attacker hitting both surfaces gets 2x
the budget. Same for the money limiter on portal payment endpoints
vs API payment endpoints.

**What to do.** Build one `Arc<AppState>` in `main.rs`, pass it into
both `create_app` and `create_web_routes`. The API router's
`with_state` already takes the state; just hand it the shared one.

**Effort + risk.** Small. Touch points: `api::create_app` signature,
`main.rs`, `web::create_web_routes`.

## F11 — `MembershipType` enum / `membership_type_id` parallel paths

**Where.** Domain has both `MembershipType` (enum, used by signup) and
`membership_type_id` (FK to a `membership_types` table, used by
billing/dues calculation). Migration to the FK was started, never
finished.

**Why it matters.** Signup writes the enum; billing reads the FK. New
members get `membership_type_id = NULL` and the billing runner has to
infer or default. The drift will eventually cause "their dues never
got extended" tickets.

**What to do.** Either finish the migration (signup writes both, then
read-side consumers switch to FK, then drop the enum) or punt with a
tactical fix that populates `membership_type_id` at signup based on
the enum value. The former is the right answer; the latter is an OK
holdover if there's no time.

**Effort + risk.** Medium. Touches signup, billing, member repo, a
migration.

## F12 — Sweep dead code from F8-extended

**Where.** After the JSON admin deletion, several modules and methods
are now unused:

- `MemberService` — entirely dead (callers were the deleted JSON
  handlers).
- `BillingService` — several methods unused.
- Repos — a few unused methods (`list_all_event_types` etc.).
- `stripe_client.rs` — unused imports flagged in F4.
- `IntegrationEvent::MemberCreated` / `MemberDeleted` — emitted only
  by deleted handlers.
- `AppError::Payment` variant — no longer constructed.
- Stale `Redirect` / `Sha256` imports.

**What to do.** Run `cargo clippy --all-targets --all-features` and
`cargo +nightly udeps` (or equivalent), delete what's truly dead, keep
anything the portal might still reach. Smallest-change-first.

**Effort + risk.** Small. Mechanical. Low risk if test suite stays
green.

## F14 — Inline-HTML construction in handlers

**Where.** Several handlers (admin members, payments, donations) build
HTML fragments via `format!("<div>...</div>")` for HTMX swaps instead
of using Askama partials.

**Why it matters.** Templates carry escaping; `format!` doesn't. One
of the inline strings already has a TODO that says exactly this.

**What to do.** Opportunistic refactor — extract partials when you're
already touching a handler. Don't do a sweep; let it happen organically.

**Effort + risk.** Background. Not blocking.

## F15 — `setup_lock` duplicated (sub-finding of F10)

Same root cause as F10. Two `AppState`s means two `setup_lock`s. The
practical effect is small (lock is only held during the very first
boot's setup wizard) but the asymmetry is a smell. Fix falls out of
F10.

## Suggested order

F9 → F13 (tied) → F10 (subsumes F15) → F12 → F11 → F14.

F9 is the only one that's genuinely urgent. The rest are quality
improvements.

---

## What's working well — leave alone

Things the punchlist deliberately doesn't touch, so you don't
scope-creep:

- **`domain::payment` module.** `Payer` / `PaymentKind` / `StripeRef`
  are doing what sum types are supposed to do. The `as_str` /
  `member_id` / `campaign_id` accessors are right-sized convenience.
- **`PaymentRepository::extend_dues_for_payment_atomic`.** Highest-stakes
  concurrency in the codebase; the per-payment `dues_extended_at` claim
  + transactional read-then-write is exactly right.
- **`WebhookDispatcher::handle_payment_intent_succeeded`** cross-checks
  PI metadata against the local row before mutating (member_id and
  amount). That guard is load-bearing security.
- **`StripeGateway` trait + `FakeStripeGateway`.** Test seam is clean.
  Keeping `Webhook::construct_event` *out* of the trait is well-reasoned
  and documented.
- **`BillingService` facade.** 179 lines, no logic, just delegates.
  Don't second-guess.
- **The Pending-first / `complete_pending_payment` race protocol.**
  "Whoever flips owns the post-work" is consistent across Checkout,
  saved-card, and donation flows.
