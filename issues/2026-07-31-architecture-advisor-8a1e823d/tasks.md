# Tasks

All four refactors are behavior-preserving. Treat any change to an HTTP
route, an OpenAPI path or schema, the `coterie-provision` CLI surface, or
a test assertion as a regression — keep them identical. The single
intended output change is task 2.6.

Line numbers reference the pre-refactor tree; re-locate by symbol name if
they have shifted.

## 1. Split `src/api/handlers/public.rs` into a `public/` module

- [ ] 1.1 Convert `src/api/handlers/public.rs` into a module directory
  `src/api/handlers/public/` (delete the single file once moved). Keep the
  module path `crate::api::handlers::public` intact so `src/api/mod.rs`
  (`public_routes`, lines 152–185) and `src/api/docs.rs` (26–35, plus the
  `components(schemas(...))` list at 43–51) need **no** path edits beyond
  what re-exports already cover.
- [ ] 1.2 Create `public/signup.rs` holding the signup funnel (current
  209–779): `signup`, `signup_checkout_urls`, `create_signup_checkout`,
  `initiate_checkout_on_verify`, `retry_pending_checkout`,
  `send_verification_email`, `org_name`, and the `SignupRequest` /
  `SignupResponse` DTOs.
- [ ] 1.3 Create `public/feeds.rs` holding the feed serialization (current
  1041–1180): `escape_cdata`, `generate_rss_feed`, `escape_ical_text`,
  `generate_ical_feed`, and the `rss_feed` / `calendar_feed` handlers
  (969–1020).
- [ ] 1.4 Create `public/donate.rs` holding the donation path (current
  1181–1362): `PublicDonateRequest`, `PublicDonateResponse`, and `donate`.
- [ ] 1.5 Create `public/register.rs` holding guest registration and class
  enrollment (current 1363–1674): `PublicEventRegisterRequest`,
  `PublicEventRegisterResponse`, `register_for_event`, `enroll_in_class`,
  and their private helpers `is_form_encoded` and `parse_body`. Move the
  fixed protection-order doc comment (1390–1407) with
  `register_for_event`; both handlers must keep their current
  rate-limit → bot-challenge → registerability → seat-and-charge ordering
  verbatim.
- [ ] 1.6 Leave in `public/mod.rs`: the read projections `list_events`,
  `list_announcements`, `list_membership_types`, `private_event_count`
  (780–857, 909–1039), their query/DTO types `PublicEventsQuery`,
  `PublicEvent`, `PublicAnnouncement`, `PublicMembershipType`,
  `PrivateEventCount`, the `MAX_RANGE_SPAN_DAYS` const and `parse_range`
  (168–207), and `derive_utc_instants` (1106–1122) — the last is called by
  both `list_events` (815) and `calendar_feed` (1008), so keep it in
  `mod.rs` and have `feeds.rs` reach it via `use super::derive_utc_instants;`.
- [ ] 1.7 In `public/mod.rs` declare the four submodules and re-export
  every item `src/api/mod.rs` and `src/api/docs.rs` name, so
  `handlers::public::signup`, `::donate`, `::rss_feed`, `::calendar_feed`,
  `::register_for_event`, `::enroll_in_class` and the DTO schemas all still
  resolve. Keep `initiate_checkout_on_verify` `pub(crate)` and re-exported,
  so its caller at `src/web/templates/verify.rs:90` —
  `crate::api::handlers::public::initiate_checkout_on_verify(...)` —
  compiles unchanged.
- [ ] 1.8 Keep the `#[cfg(test)] mod announcement_markdown_tests` block
  (1675–1938) in `public/mod.rs`; it exercises `list_announcements` (in
  `mod.rs`) and `generate_rss_feed` (now in `feeds.rs`), so adjust only its
  imports (`use super::feeds::generate_rss_feed;` alongside the existing
  `use super::*;`). Do not rewrite any assertion.
- [ ] 1.9 Reconcile `use` imports per submodule: copy the original file's
  `use` block into each new file, then prune to what each actually
  references. `signup.rs` and `donate.rs` should be the only files pulling
  the Stripe client / email sender / settings service; `feeds.rs` should
  pull neither.
- [ ] 1.10 Confirm `cargo test --features test-utils` still produces the
  same OpenAPI document — the `#[utoipa::path]` attributes move with their
  handlers, so the paths, tags, and schema names must be byte-identical.

## 2. Extract one shared multipart event-form parser

- [ ] 2.1 In `src/web/portal/admin/events/single.rs`, add a private
  `struct EventForm` holding the **superset** of both handlers' parsed
  fields: `title`, `description`, `event_type` (`EventType`), `visibility`
  (`EventVisibility`), `start_time`/`end_time` (already converted),
  `location`, `max_attendees`, `rsvp_required`, `member_price_str`,
  `guest_price_str`, `guest_registration_enabled`, `image_url`,
  plus create's `repeat_kind` / `repeat_interval` / `repeat_weekdays` /
  `repeat_day` / `repeat_weekday` / `repeat_ordinal` / `repeat_until_str` /
  `series_member_price_str` / `series_guest_price_str` / `series_capacity` /
  `series_guest_registration_enabled`, plus update's `edit_scope` and
  `remove_image`.
- [ ] 2.2 Add `async fn parse(multipart: &mut Multipart) -> Result<EventForm,
  Response>` containing the single multipart drain loop (the union of the
  match arms at 472–572 and 865–927), the `EventType`/`EventVisibility`
  match arms, the `%Y-%m-%dT%H:%M` start/end parsing, and the `image`
  branch that calls `save_uploaded_file`. Return `Err(Response)` for the
  cases the handlers return early on today (image-upload failure, invalid
  start time), so each handler is `let form = match EventForm::parse(&mut
  multipart).await { Ok(f) => f, Err(r) => return r };`.
- [ ] 2.3 Preserve each field's exact defaults and parse semantics:
  `repeat_kind` defaults to `"none"`, `repeat_interval` to `1`,
  `repeat_weekday` to `"mon"`, `repeat_ordinal` to `1`, `edit_scope` to
  `"this"`; unchecked checkboxes are absent fields and mean `false`; prices
  stay **raw strings** parsed after the loop so a blank field and a typed
  `"0"` both land on `0`; the unknown-field arm still drains
  `field.bytes()`; the `csrf_token` arm still drains without storing.
- [ ] 2.4 Preserve the enum fallbacks exactly: an unrecognized
  `event_type` → `EventType::Meeting`, an unrecognized `visibility` →
  `EventVisibility::MembersOnly`.
- [ ] 2.5 Preserve the datetime conversion exactly:
  `NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M")` then
  `DateTime::from_naive_utc_and_offset(dt, Utc)` — the `event-timezone`
  capability stores a naive local wall-clock in a UTC container, so do NOT
  "fix" this into a real timezone conversion. `end_time` stays
  `Option`, empty-string → `None`, unparseable → `None` (never an error).
- [ ] 2.6 Unify the invalid-start-time response: `admin_update_event`
  (current 948) returns a hand-rolled `<div class="px-4 py-3 bg-red-100
  text-red-800 rounded-md text-sm">Invalid start time</div>` literal;
  `EventForm::parse` returns `partials::admin_alert("error", "Invalid start
  time", false).into_response()` for both callers, matching
  `admin_create_event` (593). Nothing asserts on the old literal.
- [ ] 2.7 Rewrite `admin_create_event` (437–603) and `admin_update_event`
  (845–957) to read from `EventForm`, each keeping only its own downstream
  logic — create's `build_recurrence` / series-pass pricing decision,
  update's `new_image_url` > `remove_image` > keep-existing resolution, the
  old-image disk deletion, and the `edit_scope` this-vs-this-and-future
  branch. Each handler ignores the `EventForm` fields its form never sends;
  parsing a field a handler does not read must not change what it does.
- [ ] 2.8 Net result is a deletion: the duplicated ~95-line drain loop, the
  duplicate enum match arms, and the duplicate datetime parsing exist once.
  Do not add a `FromStr` impl on the domain enums — the match arms move
  into the parser, they do not become a public conversion.

## 3. Move the Stripe JSON fixture builders into `tests/common/mod.rs`

- [ ] 3.1 Add the generalized builders to `tests/common/mod.rs` as `pub`
  functions, taking the fields the callers vary and keeping the `stripe`
  crate's required-field filler (`automatic_tax`, `custom_text`,
  `shipping_options`, and the rest) in exactly one place:
  - `build_checkout_session(id, payment_intent, metadata)` — port the
    already-generalized form from `tests/stripe_webhook_test.rs:406`, and
    add the `amount_total` parameter the `paid_events` / `series_pass`
    copies vary.
  - `build_charge(id, amount, payment_intent)` — the copies at
    `tests/paid_events_test.rs:389` and `tests/series_pass_test.rs:441` are
    byte-identical; reconcile them with the `build_charge` variant at
    `tests/stripe_webhook_test.rs:346`, and if they differ behaviorally,
    keep both as separately-named functions per the `test-infrastructure`
    capability's "behaviorally-different copies are preserved" scenario —
    do NOT silently pick one.
  - The `payment_intent`, `subscription`, and `invoice` builders wherever
    they are duplicated across these same four binaries.
- [ ] 3.2 Delete the local copies from `tests/paid_events_test.rs`,
  `tests/series_pass_test.rs`, `tests/stripe_webhook_test.rs`, and
  `tests/stripe_config_test.rs`, replacing each call site with the shared
  builder and passing that file's own `metadata` / `amount_total` as
  arguments. All four already declare `mod common;` — no new module wiring.
- [ ] 3.3 Keep the thin per-file wrappers that only bind this-test-specific
  metadata (e.g. `event_fee_session`, `series_pass_session`,
  `expired_session`) in their own files, re-implemented as one-line calls
  into the shared builder. Do not move test-specific fixtures into
  `common`.
- [ ] 3.4 Verify with `grep -rn "fn charge\|fn build_charge\|\"object\": \"checkout.session\"" tests/`
  that each builder body has exactly one definition and it lives in
  `tests/common/mod.rs`.
- [ ] 3.5 Change no assertion in any of the four binaries; every test that
  passed before still passes.

## 4. Split `install.rs` into `install/inputs.rs` + `install/executor.rs`

- [ ] 4.1 Convert `deploy/coterie-provision/src/install.rs` into
  `deploy/coterie-provision/src/install/` with `mod.rs`, `inputs.rs`, and
  `executor.rs` (delete the single file once moved). Keep the module path
  `coterie_provision::install` so `src/lib.rs:6` (`pub mod install;`) and
  every `install::…` reference in `src/main.rs:4,299`, `src/update.rs:36,320`,
  and `tests/install_flow.rs` compile with **no** edits.
- [ ] 4.2 Move to `install/inputs.rs` the operator interview (current
  242–826): `ResolvedInputs`, `StripeInputs`, `DiscordInputs`,
  `UnifiInputs`, `normalize_marketing_domain`, `gather_inputs`,
  `gather_stripe_inputs`, `gather_discord_inputs`, `gather_unifi_inputs`,
  `gather_caddy_inputs`, and `generate_session_secret`. This module depends
  on `Prompter` only — it must not import `SystemCommand` or `FileSystem`.
- [ ] 4.3 Move to `install/executor.rs` the box mutation (current
  882–1313): the `Executor` struct and all its methods (`announce`, `run`,
  `apt_update`, `apt_install`, `fetch_release_deploy`, `run_release_deploy`,
  `assert_binaries_present`, `render_and_write_env`,
  `write_live_overlay_if_needed`, `bootstrap_admin`, `chown_data_dir`,
  `write_caddyfile`, `enable_and_start_service`, `smoke_test`), the free
  `pub fn smoke_test` (1284), and `overwrite_with_zeros` (1306). This
  module depends on `SystemCommand` / `FileSystem` / `Output` and takes
  `&ResolvedInputs` from `inputs.rs` — it must not import `Prompter`.
- [ ] 4.4 Leave in `install/mod.rs`: `StripeMode` and its `as_str` /
  `FromStr` impls, `InstallArgs`, `PreflightState`, `detect_state`,
  `is_root` / the `geteuid` extern, the `SMOKE_TEST_INTERVAL` /
  `SMOKE_TEST_BUDGET` constants (still `pub(crate)` for `update.rs:36`),
  `print_summary`, `print_exit_summary`, and `run()` (147–218) as the
  orchestrator: read state → gather → print summary → execute.
- [ ] 4.5 Re-export from `install/mod.rs` everything the crate's public
  surface names today, so these all still resolve unchanged:
  `install::run`, `install::detect_state`, `install::smoke_test`,
  `install::InstallArgs`, `install::PreflightState`, `install::StripeMode`,
  `install::ResolvedInputs`, `install::SMOKE_TEST_INTERVAL`,
  `install::SMOKE_TEST_BUDGET`. Give each moved item the narrowest
  visibility that compiles (`pub(super)` / `pub(crate)` / `pub`).
- [ ] 4.6 Split the inline `#[cfg(test)] mod tests` (1315–end) to follow
  its subject: `normalize_marketing_domain_trims_and_strips_www` and
  `stripe_bad_prefix_rejected` / `missing_required_input_fails_under_no_prompt`
  go to `inputs.rs`; the `detect_state_*` and `dry_run_install_*` tests stay
  in `mod.rs` with `make_args`. Move test bodies verbatim; do not rewrite
  assertions.
- [ ] 4.7 Preserve exactly, per the `provisioning-wizard` capability: every
  prompt's flag → `COTERIE_PROVISION_*` env → interactive-prompt resolution
  order, every prefix/`validate_prefix` check, the live-mode "accept
  pk_test_ or pk_live_" leniency, the test-mode strict-prefix checks, the
  `preload_live_creds` inference, `--no-prompt` behavior, and the
  `Executor`'s apt → fetch → env-write → bootstrap → chown → caddy →
  systemd → smoke-test ordering. No prompt text, default, validation, or
  command sequence may change.
- [ ] 4.8 Keep `deploy/coterie-provision/tests/install_flow.rs` and its
  `fixtures/` golden snapshots passing untouched — a diff there means the
  move changed behavior.

## 5. Validation (run after each refactor)

- [ ] 5.1 `cargo build --features test-utils` — no compiler errors beyond
  the pre-refactor baseline.
- [ ] 5.2 `cargo test --features test-utils` — every test that passed
  before still passes; none are added, removed, or have changed assertions.
- [ ] 5.3 `cargo test --manifest-path deploy/coterie-provision/Cargo.toml`
  — the `install_flow`, `caddyfile`, `switch_to_live`, and `update_flow`
  suites all still pass, golden snapshots included.
- [ ] 5.4 `cargo clippy --features test-utils` and `cargo fmt --check` — no
  new warnings or formatting diffs relative to the baseline.
- [ ] 5.5 Spot-check that each split actually relocated code (each named
  unit is materially smaller, each duplicate has exactly one definition)
  rather than leaving a re-export shim over an unchanged file.
