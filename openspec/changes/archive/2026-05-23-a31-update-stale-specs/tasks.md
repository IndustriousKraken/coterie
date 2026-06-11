## 1. integration-events: rewrite the member-events requirement

- [x] 1.1 At archive time, OpenSpec applies the RENAMED + MODIFIED operations in `openspec/changes/a31-update-stale-specs/specs/integration-events/spec.md` to `openspec/specs/integration-events/spec.md`. Verify the result reads correctly post-archive — no orphan title, no contradiction between body and surrounding text.

## 2. audit-logging: update the locus-by-domain inventory

- [x] 2.1 OpenSpec applies the MODIFIED operation in `specs/audit-logging/spec.md` to the existing capability spec. The "Member operations" bullet flips from handler-emitted to `MemberService`-emitted; the "types" bullet adds the parenthetical note about a32 fixing the missing audit calls.

## 3. payment-recording: enumerate three entry points

- [x] 3.1 RENAMED + MODIFIED operations rewrite the entry-point requirement.
- [x] 3.2 Scenarios are added/updated to cover the new third entry point (`BillingService::process_scheduled_payment`).

## 4. Validation

- [x] 4.1 `openspec validate a31-update-stale-specs` — confirms structural well-formedness.
- [x] 4.2 Confirm no source code changes appear in the PR diff. This is spec-only.
- [x] 4.3 Confirm no test changes appear. Verified via `git diff --stat` after applying the change — should show only `.md` files modified.
- [x] 4.4 `cargo test --features test-utils` — the test suite is unchanged and SHALL still pass without modification (since this is spec-only). (Cargo not available in this sandbox; verified instead that zero `.rs` files are touched, so the suite is logically unaffected.)

## 5. Documentation cross-references

- [x] 5.1 If `CLAUDE.md` references "audit/events live in services for payments only" or similar, update the language to reflect that member operations also follow this rule. Grep `CLAUDE.md` for relevant phrasing; update if found. (No `CLAUDE.md` exists in the repository; vacuously satisfied.)
