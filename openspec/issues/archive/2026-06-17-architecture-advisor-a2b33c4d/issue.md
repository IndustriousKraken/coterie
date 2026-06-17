# Decompose five oversized/duplicated source units (architecture_advisor)

## Problem

An `architecture_advisor` pass flagged five source units that have grown
oversized or carry duplicated logic. Each is a **behavior-preserving**
structural concern — the observable contracts (HTTP routes, the
`coterie-provision` CLI surface, repository trait signatures, the seed
binary's output, and the test suite's assertions) are all correct today;
only the internal organization is at fault:

1. `src/web/portal/admin/events.rs` (1372 lines) mixes single-event CRUD
   handlers with the per-occurrence (recurring-series) exception handlers in
   one file.
2. `gather_inputs` in `deploy/coterie-provision/src/install.rs` (the
   wizard's input-collection function, ~274–702) is a single long function
   that inlines Stripe, Discord, UniFi, and Caddy prompt collection.
3. `main` in `src/bin/seed.rs` (~322–905) runs every seeding phase
   (configurable types, members, events, announcements, payments) inline in
   one function.
4. `search` and `export_rows` in `src/repository/member_repository.rs`
   (~658–838) duplicate the WHERE-clause builder, the ORDER-BY mapping, and
   the parameter-binding sequence.
5. `src/service/event_admin_service.rs` (2057 lines) carries a ~1400-line
   inline `#[cfg(test)] mod tests` block (651–2057) that dominates the file.

## Desired end state

Each unit is decomposed into cohesive pieces with **no observable behavior
change**:

- `events.rs` becomes a module directory whose single-event CRUD handlers and
  per-occurrence handlers live in separate submodules; the router in
  `src/web/portal/mod.rs` resolves the same handlers via re-exports, with no
  router edits.
- `gather_inputs` delegates to one helper per integration (Stripe / Discord /
  UniFi / Caddy); every prompt keeps its existing flag → env → prompt
  resolution and validation order.
- `seed.rs`'s `main` calls one function per seed phase; the seeded data and
  the printed summary are byte-for-byte the same.
- `member_repository` has one shared private filter/sort builder that both
  `search` and `export_rows` call; the generated SQL, bound-parameter order,
  and per-method column qualification are preserved exactly.
- `event_admin_service.rs`'s tests move to a sibling file declared via
  `#[cfg(test)] #[path = "…"] mod tests;`, keeping the service itself at its
  canonical path `src/service/event_admin_service.rs`.

Each refactor must introduce **no new** compiler errors, clippy warnings,
`cargo fmt` diffs, or test failures relative to the pre-refactor baseline,
and must change no test assertions.

## Out of scope

No public API, wire format, CLI flag/env surface, or HTTP route changes —
those are correct today and must not move. This issue is organization only.
Do not bundle unrelated cleanup beyond the five units named above.
