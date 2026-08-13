# Tasks

This change reconciles canon with code that already ships. There is no behavior
to build; the work is verifying that the claim canon is about to make is true,
and leaving a guard so the two cannot drift apart again.

## 1. Verify the code already satisfies the amended requirement

- [x] 1.1 Confirm `PublicEvent` in `src/api/handlers/public/mod.rs` carries
  `description_html` alongside `description`, and that no other field has
  appeared on the projection that the enumerated list does not name. The list is
  closed, so an unlisted field is a defect in either the code or the list — check
  which, do not assume.
- [x] 1.2 Confirm `description_html` is populated from the **projected**
  description, after members-only sanitization, per the rule in
  `announcement-markdown`. Do not re-implement or re-specify that rule; confirm
  the existing implementation and its test still hold.
- [x] 1.3 Confirm the generated OpenAPI schema reflects the field —
  `PublicEvent` derives `ToSchema` and is registered in `ApiDoc`
  (`src/api/docs.rs`), so this should already be true. If it is not, that is a
  real gap and is in scope.

## 2. Guard against recurrence

- [x] 2.1 Add or extend a test asserting the serialized `/public/events` entry's
  key set equals exactly the enumerated field list. A closed list in canon with
  nothing enforcing it is how the two came apart: the field was added to the
  struct and the list was never updated, and nothing failed.
- [x] 2.2 Write it so it fails on a field added to the struct **and** on a field
  removed from it. A one-directional assertion catches only half the drift.
- [x] 2.3 Point the assertion's failure message at this requirement by name, so
  whoever trips it knows the list is canon and not an arbitrary fixture.

## 3. No behavior change

- [x] 3.1 Change no serialization, no renderer, no template, and no existing
  test's expectations. If any existing test needs editing to make this change
  pass, stop — that means the code does not actually match what canon is being
  amended to say, and the amendment is wrong.
