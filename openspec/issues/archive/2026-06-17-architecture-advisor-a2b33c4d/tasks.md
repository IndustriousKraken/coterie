# Tasks

All five refactors are behavior-preserving. Treat any change to an HTTP
route, the `coterie-provision` CLI surface, a repository trait signature, the
seed binary's output, or a test assertion as a regression — keep them
identical.

## 1. Split `src/web/portal/admin/events.rs` into CRUD + occurrence submodules

- [x] 1.1 Convert `src/web/portal/admin/events.rs` into a module directory
  `src/web/portal/admin/events/` (delete the single file once moved). Keep
  the module path `crate::web::portal::admin::events` intact so the router in
  `src/web/portal/mod.rs` (lines ~98–130) needs **no edits**.
- [x] 1.2 Create `events/single.rs` holding the single-event CRUD handlers and
  their helpers: `admin_events_page`, `admin_event_detail_page`,
  `admin_new_event_page`, `admin_create_event`, `build_recurrence` (private),
  `parse_until` (private), `admin_update_event`, `admin_delete_event`, plus the
  templates/forms used only by these (e.g. `AdminEventsTemplate`,
  `AdminEventsTableTemplate`, `DeleteEventForm`).
- [x] 1.3 Create `events/occurrences.rs` holding the per-occurrence
  (recurring-series exception) handlers and their types:
  `admin_occurrence_override_form`, `admin_cancel_event_occurrence`,
  `admin_override_event_occurrence`, `admin_restore_event_occurrence`,
  `admin_event_series_detail_page`, plus `EventOccurrenceRowTemplate`,
  `OccurrenceRowInfo`, `EventOccurrenceOverrideFormTemplate`,
  `OverrideFormEvent` (the block at current lines 962–1372).
- [x] 1.4 In `events/mod.rs` declare `mod single;` and `mod occurrences;`,
  re-export the handlers the router names (`pub use single::*;`
  `pub use occurrences::*;`, or selective re-exports), and house any type used
  by **both** submodules (e.g. `TypeOption`) so neither submodule depends on
  the other. Keep each handler's visibility the narrowest that compiles
  (`pub(super)`/`pub`).
- [x] 1.5 Reconcile `use` imports per submodule (copy the original file's
  `use` block into each, then prune to what each actually references).
- [x] 1.6 Confirm the handlers still route through `EventAdminService` for
  mutations (no handler regresses to calling `event_repo.{create,update,
  delete}`, `audit_service.log`, or `integration_manager.handle_event`
  directly) — this is required by the `admin-events` / `event-admin-service`
  capabilities and must survive the move unchanged.

## 2. Break `gather_inputs` into per-integration helpers (`install.rs`)

- [x] 2.1 In `deploy/coterie-provision/src/install.rs`, extract the Stripe
  block (current lines ~357–526, including the `enable_stripe` gate, the
  test/live mode branch, and the live-credential pre-load) into a private
  helper, e.g. `gather_stripe_inputs(args, prompts, no_prompt) -> Result<…>`
  returning the Stripe tuple/struct that `gather_inputs` assembles today.
- [x] 2.2 Extract the Discord block (~530–578) into `gather_discord_inputs`.
- [x] 2.3 Extract the UniFi block (~582–620) into `gather_unifi_inputs`.
- [x] 2.4 Extract the Caddy block (~625–702) into `gather_caddy_inputs`.
- [x] 2.5 Leave `gather_inputs` as the orchestrator: it still collects the
  core inputs (org name, domains, contact/admin email, admin
  username/full-name/password, session secret) and now calls the four
  helpers, assembling the same `ResolvedInputs`.
- [x] 2.6 Preserve exactly, per the `provisioning-wizard` capability: every
  prompt's flag → `COTERIE_PROVISION_*` env → interactive-prompt resolution
  order (via the existing `resolve`/`resolve_secret`/`resolve_bool` helpers),
  every prefix/`validate_prefix` check, the live-mode "accept pk_test_ or
  pk_live_" leniency, the test-mode strict-prefix checks, the
  `preload_live_creds` inference, and `--no-prompt` behavior. No prompt text,
  default, or validation may change.

## 3. Extract `seed.rs` `main`'s phases into per-entity seed functions

- [x] 3.1 In `src/bin/seed.rs`, extract each phase of `main` (current
  ~322–905) into its own `async fn`, threading the `pool`/config/RNG and
  returning the counts `main` prints:
  - `seed_configurable_types(...)` — the "Creating configurable types" phase
    (~411–428).
  - `seed_members(...)` — the "Creating members" phase including seeded test
    users and randomly generated members (~472–661); return the member
    collection `main` later references.
  - `seed_events(...)` — the "Creating events" phase (~666–738); return
    `event_count`.
  - `seed_announcements(...)` — the "Creating announcements" phase
    (~743–795); return `announcement_count`.
  - `seed_payments(...)` — the "Creating payment records" phase (~800–869);
    return `payment_count`.
- [x] 3.2 Keep `main` as the orchestrator: arg parsing, config load, pool
  setup, migrations, the data-clear step, the ordered calls to the phase
  functions, and the final summary `println!` block (~874–end).
- [x] 3.3 Preserve every `println!` (text, order, and emitted counts) and the
  RNG seeding so a seeded database and the printed summary are identical to
  before. Reuse the existing module-level helpers (`make_payment`,
  `make_event`, `make_username`, `generate_member_config`, etc.) — do not
  duplicate them.

## 4. Factor the shared filter/sort builder out of `search` and `export_rows`

- [x] 4.1 In `src/repository/member_repository.rs`, add a private helper (in
  the same file — `repository-contracts` keeps the trait, impl, and aux types
  per-file) that builds the WHERE fragment and the ordered list of bound
  values from a `&MemberQuery`, and a second that builds the ORDER-BY
  fragment. Both `search` (~658) and `export_rows` (~750) call them.
- [x] 4.2 Parameterize the column qualification: `search` uses unqualified
  columns (`full_name`, `email`, `username`, `status`, `membership_type_id`);
  `export_rows` uses `m.`-qualified columns and joins `membership_types mt`.
  The helper must emit the correct form for each caller (e.g. take a
  prefix/qualified flag), not collapse them to one.
- [x] 4.3 Preserve the per-caller ORDER-BY difference for
  `MemberSortField::MembershipType`: `search` orders by `membership_type_id`,
  `export_rows` orders by `LOWER(mt.name)`. Preserve the
  `DuesPaidUntil` → "NULL sorts last" behavior (`dues_paid_until IS NULL,
  dues_paid_until <dir>`, prefixed for export). Keep sort fields/directions
  mapped to constant strings (no user input on the sort path) so the
  injection-safety guaranteed by `repository-contracts` is retained.
- [x] 4.4 Preserve the exact bind order: WHERE binds first — search pattern
  (bound three times) → status → membership-type — then `search` appends
  `LIMIT ?`/`OFFSET ?` while `export_rows` appends neither. Both the row query
  and `search`'s `COUNT(*)` query bind the same WHERE params.
- [x] 4.5 Leave the `MemberRepository::search` and
  `MemberRepository::export_rows` trait signatures and the returned shapes
  (`(Vec<Member>, i64)` and `Vec<MemberExportRow>`) unchanged.

## 5. Move `event_admin_service`'s inline test module to a sibling file

- [x] 5.1 Move the entire `#[cfg(test)] mod tests { … }` block (current lines
  651–2057) out of `src/service/event_admin_service.rs` into a new sibling
  file `src/service/event_admin_service_tests.rs`, and in
  `event_admin_service.rs` replace the inline module with
  `#[cfg(test)]` `#[path = "event_admin_service_tests.rs"] mod tests;`.
- [x] 5.2 Do **not** convert `event_admin_service.rs` into a module directory
  — the `event-admin-service` capability pins the service at
  `src/service/event_admin_service.rs`. The `#[path = …]` sibling keeps that
  path and keeps `use super::*;` resolving from the moved tests.
- [x] 5.3 Move the test bodies verbatim; do not rewrite assertions. The
  runtime-relative-anchor rule for materializer tests (per the `admin-events`
  / `event-admin-service` capabilities) still applies in the sibling file —
  keep the existing `Utc::now()`-relative anchors; do not reintroduce
  hardcoded `with_ymd_and_hms(...)` calendar inputs.

## 6. Validation (run after each refactor)

- [x] 6.1 `cargo build --features test-utils` — introduces no compiler errors
  beyond the pre-refactor baseline.
- [x] 6.2 `cargo test --features test-utils` — every test that passed before
  still passes; none are added, removed, or have changed assertions
  (especially the webhook, member-repository, event-admin-service, and
  provisioning golden-snapshot tests).
- [x] 6.3 `cargo clippy --features test-utils` and `cargo fmt --check` —
  introduce no new warnings or formatting diffs relative to the baseline.
- [x] 6.4 Spot-check that the split actually relocated code (each named unit
  is materially smaller / the duplication is gone) rather than leaving a
  re-export shim in place.
