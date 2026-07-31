# Decompose four cross-cutting source units (architecture_advisor)

## Problem

An `architecture_advisor` pass flagged four units whose internal
organization is at fault while their observable contracts are correct.
Every one is **behavior-preserving**: the HTTP routes, the OpenAPI
document, the `coterie-provision` CLI surface, and the test assertions
are all right today and must not move.

1. **`src/api/handlers/public.rs` (1938 lines) is scoped to a URL prefix,
   not a subject.** Every sibling under `src/api/handlers/` is scoped to
   one subject area (`auth.rs`, `payments.rs`, `announcements.rs`,
   `root.rs`); `public.rs` has accumulated five unrelated areas: the
   signup funnel with Stripe Checkout creation, abandoned-checkout retry
   and the verification email (209–780); the read projections for
   events/announcements/membership types (780–1040); RSS and iCal string
   serialization with their escaping helpers (1041–1177); public
   donations (1181–1359); and guest event registration plus class
   enrollment (1363–1674). The concrete cost: the unauthenticated,
   money-moving endpoints whose fixed protection order is documented at
   1390–1407 share a file with XML/CDATA escaping, so the CAPTCHA-and-
   rate-limit ordering and its three sibling call sites are separated by
   feed generation — and the file's imports pull the Stripe client, the
   email sender, and the settings service into a compilation unit that
   also serves anonymous GETs.

2. **`admin_create_event` and `admin_update_event` carry the same
   multipart parser twice** (`src/web/portal/admin/events/single.rs`,
   437–603 and 845–957): the same ~95-line multipart drain loop, the same
   `EventType`/`EventVisibility` string match arms (573–587 / 930–944 —
   the only two such matches in the tree, since the domain enums have no
   `FromStr`), the same `%Y-%m-%dT%H:%M` start/end parsing, and the same
   `save_uploaded_file` image branch. Only the tails differ: create adds
   recurrence and series-pass fields, update adds `edit_scope` and
   `remove_image`. The copies have already drifted — on an unparseable
   start time, create returns `partials::admin_alert` (593) while update
   returns a hand-rolled `<div class="px-4 py-3 bg-red-100 …">` literal
   (948), so the same operator error renders two different ways depending
   on which button was pressed.

3. **The Stripe JSON fixture builders are copied across four test
   binaries.** `fn charge(id, amount, payment_intent)` is byte-identical
   in `tests/paid_events_test.rs:389` and `tests/series_pass_test.rs:441`,
   with a third variant as `build_charge` in
   `tests/stripe_webhook_test.rs:346`; the ~30-line `checkout.session`
   literal appears in `paid_events`, `series_pass`, `stripe_webhook`, and
   `stripe_config`, differing only in `metadata` and `amount_total`.
   `build_checkout_session(id, payment_intent, metadata)` in
   `stripe_webhook_test.rs:406` is already the generalized form the other
   three special-case. These bodies exist only to satisfy the `stripe`
   crate's `serde` requirements (note the `automatic_tax`, `custom_text`,
   `shipping_options` filler in every copy), so a crate upgrade that adds
   one required field is a four-file edit, and the copies can silently
   disagree about what a refunded charge looks like. The
   `test-infrastructure` capability already requires helpers duplicated
   ≥3 times to have exactly one implementation in `tests/common/mod.rs`,
   and every one of these files already declares `mod common;`.

4. **`deploy/coterie-provision/src/install.rs` (1466 lines) is the one
   file where every collaborator fuses.** The crate isolates each
   collaborator into its own module — `prompts.rs` for `Prompter`,
   `system.rs` for `SystemCommand`, `fs_ops.rs` for `FileSystem`,
   `output.rs`, `env_template.rs`, `caddyfile.rs` — but `install.rs`
   holds both ~525 lines of pure operator interview (`gather_inputs` plus
   `gather_stripe_inputs`/`gather_discord_inputs`/`gather_unifi_inputs`/
   `gather_caddy_inputs`, 295–821, which only resolve flag/env/prompt
   into `ResolvedInputs` and validate strings) and ~400 lines of
   privileged box mutation (`Executor`, 882–1284, which apt-installs,
   fetches `release-deploy.sh`, writes `/opt/coterie/.env`, and drives
   systemd). They share no state beyond `ResolvedInputs` and are tested
   against entirely different fakes, so a change to the Stripe key wizard
   recompiles and re-reads the systemd sequencing for no reason.

## Desired end state

Each unit is decomposed with **no observable behavior change**:

- `src/api/handlers/public.rs` becomes `src/api/handlers/public/` with
  `signup.rs`, `feeds.rs`, `donate.rs`, and `register.rs`; the read
  projections and the shared `PublicEvent` / `PublicAnnouncement` /
  `PublicMembershipType` types stay in `public/mod.rs`. Every path named
  in `src/api/mod.rs:152-185` and `src/api/docs.rs:26-35` resolves to the
  same handler and the same OpenAPI output; `initiate_checkout_on_verify`
  stays reachable as `crate::api::handlers::public::
  initiate_checkout_on_verify` for its caller in
  `src/web/templates/verify.rs:90`.
- `admin_create_event` and `admin_update_event` share one
  `EventForm::parse(&mut Multipart)` covering the superset of fields plus
  the enum and datetime conversions. Each handler keeps only its own
  business logic — create's recurrence/series-pricing decision, update's
  image-replacement and this-and-future scope — and both render the same
  `partials::admin_alert` for an invalid start time.
- The Stripe charge / checkout-session / payment-intent / subscription /
  invoice JSON builders have exactly one definition each, in
  `tests/common/mod.rs`, with callers passing their own metadata and
  amounts. No test assertion changes.
- `install.rs` becomes `install/` with `inputs.rs` (the interview) and
  `executor.rs` (the box mutation), leaving `install/mod.rs`'s `run()`
  (147–218) as the orchestrator that reads state, gathers, prints the
  summary, and executes. `coterie_provision::install::{run, detect_state,
  smoke_test, InstallArgs, PreflightState, StripeMode, ResolvedInputs,
  SMOKE_TEST_INTERVAL, SMOKE_TEST_BUDGET}` all resolve exactly as they do
  today for `main.rs`, `update.rs`, and `tests/install_flow.rs`.

Each refactor must introduce **no new** compiler errors, clippy warnings,
`cargo fmt` diffs, or test failures relative to the pre-refactor
baseline, and must change no test assertions.

## Out of scope

No HTTP route, OpenAPI path/schema, CLI flag/env surface, wire format, or
repository signature changes — those are correct today. This issue is
organization only. The one deliberate output change is unifying the
`admin_update_event` invalid-start-time alert onto `partials::admin_alert`
so both event forms render an operator error identically; nothing asserts
on the old literal. Do not bundle cleanup beyond the four units above.
