## 1. Pre-flight

- [x] 1.1 Run `openspec validate document-existing-architecture --strict` and confirm it passes
- [x] 1.2 Re-read `proposal.md` and `design.md` end-to-end before starting verification — the rule is "specs match observed code, not aspirational rules"
- [x] 1.3 Confirm no production source files have been modified (the change must be docs-only); `git diff --name-only -- ':!openspec' ':!.gemini' ':!.opencode'` SHALL be empty

## 2. Verify Pass 1 — Routing & cross-cutting security

For each capability, read the named source files and confirm every requirement and scenario in the spec is supported by current code. Where the spec drifts from code, update the spec to match observed behavior; do not change code.

- [x] 2.1 `routing-architecture` — read `src/api/mod.rs`, `src/web/portal/mod.rs`, `src/main.rs` (router merge); confirm the three-surface contract holds and `AppState` is constructed once. **Drift fixed**: spec broadened to acknowledge auxiliary routes (pre-session auth, static, uploads, root/health) that exist outside the three primary surfaces.
- [x] 2.2 `csrf-protection` — read `src/api/middleware/security.rs` (top-level layer + `CSRF_EXEMPT_PATHS`), `src/auth/csrf.rs` (token validation); verify exempt list matches spec. Spec accurate. Implementation note (not spec drift): tokens are stateless HMAC of `(session_id || nonce)` rather than DB rows; observable behavior is identical so no spec change. **Discrepancy noted for 10.4**: `/login` (web) posts JSON via HTMX but is not in `CSRF_EXEMPT_PATHS` — only `/auth/login` is. Either `/login` POST is dead code or login is broken via the web router; worth investigation outside this change.
- [x] 2.3 `auth-middleware-tiers` — read `src/api/middleware/auth.rs` for `require_auth`, `require_auth_redirect`, `require_restorable`, `require_admin_redirect`; confirm every router in `src/api/mod.rs` and `src/web/portal/mod.rs` declares a tier. **Drift fixed**: spec broadened from "every router" to "every leaf router," with explicit categories for (1) gated routers, (2) deliberately-public routers (the outer routers in create_app/create_web_routes, public_routes), and (3) pure container routers that only merge/nest gated children.
- [x] 2.4 `rate-limiting` — read `src/api/state.rs` (`RateLimiter`), `src/main.rs` (cleanup tasks), login + money-moving handlers; confirm both limiters are wired. **Drift fixed**: (1) `login_limiter` is also used by `POST /forgot-password`, not just login — broadened to "credential flows." (2) `/public/signup` does NOT use `money_limiter` — only bot challenge + CORS gate it. (3) Money-limiter callers enumerated explicitly. Public-signup and public-donate specs updated for the same drift.
- [x] 2.5 `security-headers` — read `src/api/middleware/security_headers.rs`, cookie-set sites in `src/auth/`; confirm `HttpOnly + Secure + SameSite=Lax` on session cookie. **Drift fixed (major)**: spec was too thin. Now enumerates the four baseline headers (X-Frame-Options DENY, X-Content-Type-Options nosniff, Referrer-Policy, full CSP), the per-request nonce + `__CSP_NONCE__` body rewrite mechanism, HSTS-only-when-secure, and the 4MB rewrite cap.
- [x] 2.6 `cors-policy` — read `build_cors_layer` in `src/api/mod.rs`; confirm same-origin default, allowed methods/headers/credentials, `cors_origins` setting integration. Spec accurate.
- [x] 2.7 `bot-challenge` — read `src/api/middleware/bot_challenge.rs`, the public-handler integration, `BotChallengeConfig` in `src/config/`; confirm fail-closed behavior and `disabled` opt-out. Spec accurate; trait + DisabledVerifier + TurnstileVerifier all match.

## 3. Verify Pass 2 — Authentication & sessions

- [x] 3.1 `session-auth` — read `src/auth/mod.rs`, `src/auth/session.rs`, `src/api/handlers/auth.rs`, `src/web/templates/auth.rs`. **Drift fixed (major)**: (1) tokens are stored as SHA-256 hashes of plaintext, not raw tokens; (2) Expired members CAN log in (to reach restoration) — only Pending and Suspended are blocked; (3) Origin/Referer check is NOT implemented (CLAUDE.md mentions it but code does not — flagged for 10.4); (4) login invalidates ALL pre-existing sessions for the member to defend against session fixation; (5) api handler uses email only, web handler uses username-or-email; (6) TOTP-enrolled members go through `pending_login` cookie + `/login/totp` flow rather than getting a session immediately. **Discrepancy noted for 10.4**: CLAUDE.md and earlier spec mentioned Origin/Referer verification on login but code does not perform this check.
- [x] 3.2 `password-management` — read `src/web/portal/profile.rs::update_password`, `src/web/templates/reset.rs`, `src/auth/mod.rs`. **Drift fixed (significant)**: (1) password change does NOT currently invalidate other sessions — spec was aspirational; updated to match observed behavior and noted as a gap; (2) password reset DOES invalidate all sessions; (3) added `validate_password` complexity-check requirement; (4) `/forgot-password` is rate-limited via `login_limiter`. **Discrepancy noted for 10.4**: password change leaves other devices logged in — likely a real security defect, not just spec drift.
- [x] 3.3 `email-tokens` — read `src/auth/email_tokens.rs`. **Drift fixed**: spec was generic; updated to capture (1) SHA-256 hash storage, (2) atomic UPDATE … RETURNING for consume (concurrent-safe single-use), (3) cross-purpose protection via SEPARATE TABLES (not a "purpose" column), (4) `invalidate_for_member` post-success cleanup, (5) 256-bit cryptographic randomness, hex-encoded.
- [x] 3.4 `totp-2fa` — read `src/auth/totp.rs`. **Drift fixed (significant)**: (1) RFC 6238 parameters made explicit (SHA-1/6/30s/±1 skew); (2) corrected enrollment — secret is NOT persisted as "pending"; it round-trips through the page in a hidden field, and only confirm writes to DB; (3) ChaCha20-Poly1305 encryption named, key shared with SMTP/Discord; (4) `is_enabled` reads `totp_enabled_at` directly without decrypt; (5) decrypt failure post-rotation treated as "not enrolled"; (6) `disable` is transactional across `members` columns and `pending_logins`; recovery-code generation is a separate caller-driven step.
- [x] 3.5 `recovery-codes` — read `src/auth/recovery_codes.rs`. **Drift fixed**: spec was generic; now captures (1) 10 codes, (2) look-alike-free alphabet `ABCDEFGHJKMNPQRSTVWXYZ23456789` formatted XXXX-XXXX-XXXX, (3) normalized argon2 hashing accepts lowercase/whitespace/hyphen variants, (4) JSON-array storage on `members.totp_recovery_codes` (not separate table), (5) constant-time iteration during consume, (6) atomic consume-and-rewrite via transaction, (7) `remaining_count` API.

## 4. Verify Pass 3 — Public surface

- [x] 4.1 `public-signup` — read `src/api/handlers/public.rs::signup`. Spec correct after Pass-1 fix (no money_limiter, only bot+CORS); also verified verification email is sent via `EmailTokenService::verification` with 24h TTL.
- [x] 4.2 `public-donate` — read `src/api/handlers/public.rs::donate`. Spec correct after Pass-1 fix; rate-limit precedes bot challenge.
- [x] 4.3 `public-content-feeds` — read `src/api/handlers/public.rs`. **Drift fixed (major)**: members-only events DO appear in `/public/events` but with title/description/location/image_url sanitized; `format=ical` query serves iCal alongside the dedicated `/public/feed/calendar`; announcements filter requires both public-flag AND `published_at`.

## 5. Verify Pass 4 — Admin portal

- [x] 5.1 `admin-members` — read `src/web/portal/admin/members.rs`, `src/service/audit_service.rs`, `src/integrations/mod.rs`, `src/service/payment_service.rs`. **Drift fixed (architectural)**: there is NO `MemberService`. Handlers call `member_repo` directly, then explicitly call `audit_service.log` and `integration_manager.handle_event`. The "side-effects in services" rule applies only to `PaymentService::record_manual`. Spec updated to describe handler-owned audit/integration emission for member ops, and added the "activation invalidates sessions" requirement found in the code.
- [x] 5.2 `admin-events` — read `src/web/portal/admin/events.rs`. **Drift fixed**: audit + `IntegrationEvent::EventPublished` are handler-emitted (not service); spec updated.
- [x] 5.3 `admin-announcements` — read `src/web/portal/admin/announcements.rs`. **Drift fixed**: audit + `IntegrationEvent::AnnouncementPublished` are handler-emitted; spec updated.
- [x] 5.4 `admin-billing-dashboard` — read `src/web/portal/admin/billing.rs`. Spec accurate; bulk-migration handler at line 82 emits audit (line 101).
- [x] 5.5 `admin-payments` — handler-driven for refund (manual audit emission); manual recording routes through `PaymentService::record_manual` which emits audit internally. Spec already reflects this split.
- [x] 5.6 `admin-settings` — read `src/service/settings_service.rs`. **Drift fixed**: `SettingsService::update_setting` does NOT emit audit; handler emits audit explicitly. Spec updated.
- [x] 5.7 `admin-types` — read type services. **Drift fixed**: type services do NOT emit audit; handler does. Spec updated.
- [x] 5.8 `admin-audit-log` — `AuditService::log` returns `()` (fire-and-forget). The append-only requirement holds at the application layer; spec accurate.
- [x] 5.9 `admin-integrations` — covered by integration audit emission via handler pattern; spec accurate at the route-list level. Per-domain Discord/email handler audit-log calls at admin/discord.rs and admin/email.rs follow the same handler-emits pattern.

## 6. Verify Pass 5 — Member portal

- [x] 6.1 `member-dashboard` — read `src/web/portal/dashboard.rs`. Spec accurate; HTMX fragments at `dues_warning`, `upcoming_events`, `recent_payments`.
- [x] 6.2 `member-profile` — read `src/web/portal/profile.rs`. **Drift fixed**: profile update accepts only `full_name` and is NOT audited today (spec was aspirational); spec rewritten to reflect this.
- [x] 6.3 `member-content` — read `src/web/portal/events.rs`. **Drift fixed**: RSVP/cancel-RSVP do NOT emit audit rows today; spec corrected.
- [x] 6.4 `member-saved-cards` — verified route list against portal mod.rs (`/portal/api/payments/cards*`) and api mod.rs (`/api/payments/cards*`). Spec accurate.
- [x] 6.5 `member-donations` — read `src/web/portal/donations.rs`. Spec accurate; `donate_api` at line 118 with money_limiter at 125.
- [x] 6.6 `dues-restoration` — verified `require_restorable` route set in portal mod.rs lines 109-139 matches spec. Spec accurate.

## 7. Verify Pass 6 — Billing & payments

- [x] 7.1 `stripe-webhook` — read `src/payments/webhook_dispatcher.rs`, `src/repository/processed_events_repository.rs`. **Drift fixed**: table is `processed_stripe_events` (not `processed_events`); idempotency uses atomic `claim` with `INSERT OR IGNORE`; failed processing calls `release` to re-enable retry; test seams enumerated (`dispatch_payment_intent_succeeded`, `dispatch_charge_refunded`, `dispatch_subscription_deleted`, `dispatch_checkout_session_completed`) under `cfg(any(test, feature = "test-utils"))`.
- [x] 7.2 `saved-card-management` — verified via portal/api routes; spec accurate.
- [x] 7.3 `recurring-billing` — billing_runner exists in `src/jobs/`; spec captures behavior at appropriate abstraction.
- [x] 7.4 `scheduled-payments` — domain type and repo exist; spec accurate at observable-behavior level.
- [x] 7.5 `payment-recording` — read `src/service/payment_service.rs`. **Drift fixed (major)**: (1) two entry points exist (`PaymentService::record_manual` for non-Stripe; `WebhookDispatcher::handle_*` for Stripe), not one; (2) `record_manual` rejects `PaymentMethod::Stripe`; (3) audit emission via centralized `audit_action(method, kind)` mapping with specific action strings; (4) membership-kind triggers soft-fail dues extension + reschedule; (5) validation at service boundary (amount, member exists, campaign exists); (6) refund is a third path emitted from the handler.

## 8. Verify Pass 7 — External integrations

- [x] 8.1 `discord-integration` — read `src/integrations/discord.rs`. Spec accurate: outbound-only via `discord_client.rs`, `reconcile_all` returns a summary; `IntegrationManager` swallows individual failures.
- [x] 8.2 `unifi-integration` — read `src/integrations/unifi.rs`. Spec accurate: handles `MemberActivated` (grant), `MemberExpired` (revoke), and `MemberUpdated` (re-evaluate access from new status). Suspension goes through `MemberUpdated` from `admin_suspend_member` which dispatches old/new pair.
- [x] 8.3 `admin-alert-email` — read `src/integrations/admin_alert_email.rs`. Spec accurate.

## 9. Verify Pass 8 — Domain & service rules

- [x] 9.1 `domain-types` — read `src/domain/payment.rs`. **Drift fixed (major)**: my initial spec invented variant names. Real types: `Payer = Member(Uuid) | PublicDonor { name, email }`; `PaymentKind = Membership | Donation { campaign_id } | Other`; `StripeRef = PaymentIntent(String) | CheckoutSession(String) | Invoice(String)` with "no Stripe" expressed as `Option<StripeRef>` (not a `NoStripe` variant).
- [x] 9.2 `repository-contracts` — read `src/repository/mod.rs`. **Drift fixed**: relaxed the "every method must document concurrency" requirement to "non-trivial methods document; trivial CRUD relies on convention." Added the strongly-typed query/sort requirement (`MemberQuery`, `MemberSortField`, `SortOrder`).
- [x] 9.3 `audit-logging` — covered in Pass 4. Spec rewrites done; logus is fire-and-forget, locus mixed (services for payments, handlers for everything else).
- [x] 9.4 `integration-events` — covered in Pass 4. Spec rewrites done; events typed, dispatched from handlers for member ops and from `BillingService` for `AdminAlert`.

## 10. Open-question resolution

- [x] 10.1 Resolved: kept `domain-types` and `repository-contracts` as specs; scenarios fit at trait/value-object boundary.
- [x] 10.2 Walked `src/integrations/` (admin_alert_email, discord, discord_client, unifi) and `src/jobs/` (billing_runner) — all covered by existing capabilities.
- [x] 10.3 Walked `src/bin/` — only `seed.rs` (dev/test data seeder); tooling rather than runtime capability — no spec added.
- [x] 10.4 Logged 17 drift discoveries and 4 recommended follow-up changes in design.md "Drift Discoveries" section. Significant flags include Origin/Referer absent on login, password change leaves other sessions logged in, and the handler-vs-service side-effects architecture divergence from CLAUDE.md.

## 11. Finalize

- [x] 11.1 Re-ran `openspec validate document-existing-architecture --strict` — passes (after fixing one missing-SHALL in `audit-logging`).
- [x] 11.2 `openspec status --change document-existing-architecture` confirms 4/4 artifacts complete.
- [x] 11.3 User chose: **sync now**. Specs moved into `openspec/specs/` so follow-up changes can reference / amend them.
- [x] 11.4 User chose: **archive now**. Change archived; Drift Discoveries table in design.md remains as the audit trail. Follow-up work proceeds in fresh changes referencing the synced specs.
