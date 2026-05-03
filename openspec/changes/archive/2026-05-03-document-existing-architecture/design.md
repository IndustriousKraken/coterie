## Context

OpenSpec is freshly installed in this repo (`openspec/specs/` is empty; no prior changes archived). The codebase is mid-sized — three HTTP surfaces, ~10 service modules, ~12 repositories, three external integrations (Stripe, Discord, UniFi), background billing runner. The architectural rules that matter most live in CLAUDE.md as prose; the deeper data-model and side-effect invariants live only in the code.

This change writes ~37 specs covering current behavior. It produces no production code changes. The output is a contract future work can be checked against, plus a machine-readable home for the rules CLAUDE.md states informally today.

The current state is "code is the spec." The risk of leaving it that way is the same risk that bit us before: a contributor (human or AI) who doesn't know an unwritten rule violates it, and the violation is only caught by review luck. We have already proven that risk is real (the JSON admin surface incident, deleted 2026-04).

## Goals / Non-Goals

**Goals:**
- One `specs/<capability>/spec.md` per capability listed in `proposal.md`.
- Each requirement uses SHALL/MUST and has at least one `#### Scenario:` block (4 hashtags — exact format matters; the lint silently drops 3-hashtag scenarios).
- Spec content is observably true today: every requirement is grounded in a specific file/route/handler in the current source tree.
- The secure-by-default rules from CLAUDE.md become normative requirements with scenarios that would catch a future violation.
- The three-surface routing contract is encoded in a way that a future change touching `src/api/mod.rs` or `src/web/portal/mod.rs` can be checked against.
- Service-layer side-effect invariants (audit log, integration events) are in the spec, not just CLAUDE.md.

**Non-Goals:**
- Changing any production source. If a spec writer notices a discrepancy between CLAUDE.md and code, they update the spec to match the *code*, not the other way around — and surface the discrepancy as an open question, not a unilateral fix.
- Documenting wire formats / OpenAPI shape for `/public/*`. That already lives in `src/api/docs.rs`. Specs reference it; they don't duplicate it.
- Documenting database schema / migrations as specs. Migrations are the source of truth for schema; specs describe behavior.
- Trimming CLAUDE.md. Out of scope; happens (if at all) in a follow-up change once specs prove load-bearing.
- Adding new tests. Scenarios are testable in principle; turning them into actual `cargo test` cases is a follow-up change if we want it.

## Decisions

### Decision 1: Capability granularity — one capability per "rule someone could violate"

A capability is the smallest unit of behavior a future change might touch. We picked names like `csrf-protection`, `rate-limiting`, `auth-middleware-tiers` rather than rolling them into a single `security` capability — because each is a separate rule with its own exempt list, its own check, its own way of being violated.

Alternatives considered:
- *One mega-spec per surface (`portal`, `public`, `api`):* rejected — it would force every cross-cutting rule (CSRF, headers, rate-limiting) to be repeated three times or scattered awkwardly.
- *One spec per source file:* rejected — file boundaries don't align with behavior boundaries (e.g., `auth/session.rs`, `auth/csrf.rs`, and `api/middleware/security.rs` together implement one capability: `csrf-protection`).
- *Granularity matching the proposal's bullets:* picked — ~37 capabilities is a lot but each is small enough to write and review independently, and each maps cleanly to "if someone broke this, what would break."

### Decision 2: Specs reflect observed behavior, not aspirational behavior

Where CLAUDE.md prose and code diverge, the spec records what the code does today. Discrepancies go in `## Open Questions` here in design.md or in the relevant capability's spec.md, not silently "fixed" by writing the spec the way CLAUDE.md says it should be. A follow-up change can then propose closing the gap with full review.

This is the inverse of the usual "spec is source of truth" workflow, and it's deliberate: we are *capturing* truth from a running system, not declaring it.

### Decision 3: Scenarios written as testable WHEN/THEN, not implementation steps

Each `#### Scenario:` block is a single observable behavior at the system boundary — an HTTP request and its response, a service call and its side-effects (audit row written, integration event emitted), a repository call and its idempotency outcome. Internal call graphs do not appear in scenarios.

This keeps specs decoupled from refactors. A handler can move modules without invalidating the spec; the spec only changes when externally observable behavior changes.

### Decision 4: Sequencing — security/routing first, then per-surface

Spec authoring is sequenced so the highest-risk rules are written first. If we run out of energy, the most important specs exist:

1. **Pass 1 — load-bearing security rules**: routing-architecture, csrf-protection, auth-middleware-tiers, rate-limiting, security-headers, cors-policy, bot-challenge.
2. **Pass 2 — auth & sessions**: session-auth, password-management, email-tokens, totp-2fa, recovery-codes.
3. **Pass 3 — public surface**: public-signup, public-donate, public-content-feeds.
4. **Pass 4 — admin portal**: admin-members, admin-events, admin-announcements, admin-billing-dashboard, admin-payments, admin-settings, admin-types, admin-audit-log, admin-integrations.
5. **Pass 5 — member portal**: member-dashboard, member-profile, member-content, member-saved-cards, member-donations, dues-restoration.
6. **Pass 6 — billing & payments**: stripe-webhook, saved-card-management, recurring-billing, scheduled-payments, payment-recording.
7. **Pass 7 — integrations**: discord-integration, unifi-integration, admin-alert-email.
8. **Pass 8 — domain & service rules**: domain-types, repository-contracts, audit-logging, integration-events.

Tasks.md sequences each pass. Each pass ends with `openspec validate --strict` to catch mis-formatted scenarios early.

### Decision 5: Source-of-truth files for each spec are recorded in tasks.md, not in the spec

To keep specs portable, the path-to-source mapping ("this requirement is grounded in `src/api/middleware/security.rs:42`") lives in tasks.md as a checklist for the writer, not in the spec itself. Specs name endpoints, services, and repositories — concepts a reader can grep — not line numbers, which rot.

### Decision 6: Modified capabilities list is empty

`openspec/specs/` is empty today, so every capability is `## ADDED Requirements`. We will not invent prior specs to "modify"; the change is purely additive.

## Risks / Trade-offs

- **Spec drift from code** → Mitigation: each capability's tasks.md entry names the source files the writer must read at write time; reviewers spot-check that the spec matches the code, not CLAUDE.md.
- **Scenarios written too implementation-y** → Mitigation: scenarios at the request/response or service-call boundary, not internal calls. Decision 3 above.
- **The 4-hashtag-scenario gotcha silently dropping requirements** → Mitigation: every pass ends with `openspec validate --strict --change document-existing-architecture` before moving on.
- **Scope fatigue — author abandons mid-pass leaving partial specs** → Mitigation: passes are ordered by risk; first three passes alone cover the load-bearing security work and are independently valuable.
- **CLAUDE.md and specs disagreeing post-merge** → Mitigation: specs win going forward for the rules they cover; CLAUDE.md trimmed in a follow-up. Until then, spec text is authoritative for documented capabilities, CLAUDE.md for everything else.
- **"Documentation only" tempting writers to fix small bugs in passing** → Mitigation: this change touches *zero* files outside `openspec/`. Anything else is a separate change.

## Open Questions

- Should `domain-types` and `repository-contracts` be specs or design docs? They describe internal patterns more than user-visible behavior. **Resolved during apply**: kept as specs; scenarios fit fine at the trait/value-object boundary.
- Are there integrations/jobs not listed in proposal.md? **Resolved during apply**: walked `src/integrations/` (admin_alert_email, discord, discord_client, unifi) and `src/jobs/` (billing_runner) — all covered.
- `bin/` directory contents — do any binaries need their own capability? **Resolved during apply**: only `src/bin/seed.rs`, a dev/test data seeder; tooling rather than a runtime capability — no spec added.
- Once specs land, do we trim CLAUDE.md? Out of scope here. Recommend a follow-up change `trim-claude-md-overlap` after the first one or two changes against these specs prove they hold up.

## Drift Discoveries (from Apply phase verification)

These are observable behaviors that diverge from CLAUDE.md prose or our prior assumptions. Specs have been updated to match observed code; some rows here are also potential security gaps worth a follow-up change.

| # | Capability | Discovery | Action taken |
|---|-----------|-----------|--------------|
| 1 | `routing-architecture` | Auxiliary routes outside the "three surfaces" exist (pre-session auth pages, `/static`, `/uploads`, root/health, `/auth/*`, OpenAPI docs). | Spec broadened to acknowledge auxiliary routes with named examples. |
| 2 | `csrf-protection` | The web login form posts JSON to `/login`, which is **not** in `CSRF_EXEMPT_PATHS` — only `/auth/login` is. With no session, the CSRF middleware should 403 the request. | Flagged as potential dead-code-vs-real-bug; left for separate investigation (NOT spec drift). |
| 3 | `auth-middleware-tiers` | "Every router declares a tier" was too strong; container routers (`.merge()`-only) and deliberately-public routers exist. | Spec broadened with three categories. |
| 4 | `security-headers` | Original spec was anemic. Real implementation has full CSP with per-request nonce + strict-dynamic + `__CSP_NONCE__` body rewrite, HSTS-when-secure, four named baseline headers. | Spec rewritten thoroughly. |
| 5 | `rate-limiting` | `login_limiter` also covers `POST /forgot-password`, not just login. `/public/signup` does **not** use `money_limiter`. | Spec corrected; "credential flows" replaces "login." |
| 6 | `session-auth` | Tokens are stored as SHA-256 hashes (DB doesn't hold plaintext). Origin/Referer check on `/auth/login` is **not implemented** (CLAUDE.md says it should be). Login invalidates all pre-existing sessions for the member. API uses email only, web uses username-or-email. TOTP-enrolled members get `pending_login` cookie + redirect to `/login/totp`. | Spec rewritten; **flag**: Origin/Referer check absent — potential security gap. |
| 7 | `password-management` | Password change does **not** invalidate other sessions; password reset **does**. | Spec corrected; **flag**: changing password leaves other devices logged in — likely a real defect, worth a follow-up change. |
| 8 | `email-tokens` | Cross-purpose protection is via **separate tables** (`email_verification_tokens`, `password_reset_tokens`), not a "purpose" column. | Spec corrected. |
| 9 | `totp-2fa` | Secret is **not** persisted as "pending" — it round-trips through the page in a hidden field, only persisted on confirm. ChaCha20-Poly1305 named explicitly. | Spec corrected. |
| 10 | `recovery-codes` | 10 codes; alphabet is `ABCDEFGHJKMNPQRSTVWXYZ23456789` (look-alike free); JSON-array storage on `members.totp_recovery_codes`; constant-time iteration. | Spec corrected. |
| 11 | `public-content-feeds` | Members-only events **do** appear in `/public/events` with sanitized title/description/location/image_url — they are NOT excluded. `format=ical` query serves iCal alongside `/public/feed/calendar`. | Spec corrected (this was a significant discovery — the previous wording was actively wrong about a privacy-relevant behavior). |
| 12 | Audit + integration events architecture | CLAUDE.md says "side-effects live in services so handlers can't accidentally skip them." Reality: payments follow this rule (`PaymentService::record_manual` emits audit), but **member operations, settings, types, events, and announcements emit audit + integration events from the HANDLER**. There is no `MemberService`. | Specs for `audit-logging`, `integration-events`, `admin-members`, `admin-events`, `admin-announcements`, `admin-settings`, `admin-types`, `payment-recording` all updated. **Flag**: this is the single biggest CLAUDE.md/code divergence; worth deciding whether to (a) close the gap by introducing service-layer wrappers, or (b) update CLAUDE.md to match observed reality. |
| 13 | `member-profile` | Profile update accepts only `full_name`, and is **not audited** today. | Spec corrected; **flag**: profile changes are silent in the audit log. |
| 14 | `member-content` | RSVP / cancel-RSVP do not emit audit rows. | Spec corrected. |
| 15 | `stripe-webhook` | Table name is `processed_stripe_events` (not `processed_events`). Idempotency uses atomic `INSERT OR IGNORE`. Failed processing **releases** the claim so Stripe's retry can succeed. Test seams are named `dispatch_payment_intent_succeeded`, `dispatch_charge_refunded`, `dispatch_subscription_deleted`, `dispatch_checkout_session_completed`. | Spec corrected. |
| 16 | `domain-types` | Initial spec invented variant names for `Payer`/`PaymentKind`/`StripeRef`. Real names: `Payer::Member`/`PublicDonor`; `PaymentKind::Membership`/`Donation { campaign_id }`/`Other`; `StripeRef::PaymentIntent`/`CheckoutSession`/`Invoice`. "No Stripe" is `Option<StripeRef> = None`, NOT a `NoStripe` variant. | Spec corrected. |
| 17 | `payment-recording` | Two entry points (not one): `PaymentService::record_manual` (non-Stripe) and `WebhookDispatcher::handle_*` (Stripe). `record_manual` rejects `PaymentMethod::Stripe`. Refund is a third path emitted from the handler. Audit-action mapping centralized via `audit_action(method, kind)`. | Spec corrected. |

**Recommended follow-up changes** (not in scope here):

- `add-origin-referer-login-csrf-defense` — implement the Origin/Referer check on `/auth/login` that CLAUDE.md describes but code does not.
- `invalidate-sessions-on-password-change` — close the "other-devices-stay-logged-in" gap.
- `audit-profile-and-rsvp-changes` — decide whether RSVP / member profile updates should be audited; either add audit rows or document the "intentionally not audited" decision in the spec.
- `decide-handler-vs-service-side-effects` — pick one: either move audit + integration emission into service-layer wrappers (matching CLAUDE.md) or update CLAUDE.md to acknowledge the observed mixed model.
