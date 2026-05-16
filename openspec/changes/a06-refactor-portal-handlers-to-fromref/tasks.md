## 1. Refactor `BaseContext::for_member` to take granular inputs

- [ ] 1.1 In `src/web/templates/mod.rs`, change `BaseContext::for_member(state: &AppState, current_user: &CurrentUser, session: &SessionInfo)` to `BaseContext::for_member(csrf_service: &CsrfService, current_user: &CurrentUser, session: &SessionInfo)`. The body that today reads `state.service_context.csrf_service.generate_token(...)` becomes `csrf_service.generate_token(...)`.
- [ ] 1.2 `cargo build --all-targets --features test-utils`. Every existing call site will fail to compile; that's expected. The fixes are mechanical — extract `State<Arc<CsrfService>>` at each caller and pass `&csrf_service` instead of `&state`. Walk through each call site as you reach it in later task sections; don't try to fix them all here.

## 2. Pre-auth handlers (src/web/templates/)

- [ ] 2.1 `src/web/templates/auth.rs`: migrate every handler (login, logout, login-totp, etc.) to granular extraction. Typical needs: `Arc<AuthService>`, `Arc<MemberRepository>`, `Arc<PendingLoginService>`, `LoginLimiter`, `Arc<Settings>`.
- [ ] 2.2 `src/web/templates/setup.rs`: migrate the setup-wizard handler. Needs: `Arc<AuthService>`, `Arc<MemberRepository>`, `Arc<AsyncMutex<()>>` (setup_lock), `Arc<AtomicBool>` (admin_exists_observed), `Arc<AuditService>`, plus `Arc<IntegrationManager>` and `Arc<dyn EmailSender>` if it emits a welcome email.
- [ ] 2.3 `src/web/templates/reset.rs`: migrate password-reset handlers. Needs: `Arc<MemberRepository>`, `Arc<dyn EmailSender>`, `Arc<AuthService>` (for hashing), `LoginLimiter` (shared with login per existing spec).
- [ ] 2.4 `src/web/templates/verify.rs`: migrate email-verification handler. Needs: `Arc<MemberRepository>` and the email-token service.
- [ ] 2.5 `cargo build --all-targets --features test-utils` after each file — green.

## 3. Member-facing portal handlers (src/web/portal/, excluding admin/)

- [ ] 3.1 `src/web/portal/dashboard.rs`: migrate `member_dashboard`, `upcoming_events`, `dues_warning`, `recent_payments`, etc. Typical needs: `Arc<MemberRepository>`, `Arc<MembershipTypeService>`, `Arc<dyn EventRepository>`, `Arc<dyn PaymentRepository>`, `Arc<CsrfService>` (for BaseContext on full pages, not API fragments).
- [ ] 3.2 `src/web/portal/profile.rs`: migrate `profile_page`, `update_profile`, `update_password`. Needs: `Arc<MemberRepository>`, `Arc<AuthService>`, `Arc<CsrfService>`.
- [ ] 3.3 `src/web/portal/security.rs`: migrate TOTP/recovery-code handlers. Needs: `Arc<TotpService>`, `Arc<MemberRepository>`, `Arc<CsrfService>`.
- [ ] 3.4 `src/web/portal/events.rs`: migrate `events_page`, `events_list_api`, `rsvp_event`, `cancel_rsvp_event`. Needs: `Arc<dyn EventRepository>`, `Arc<CsrfService>`.
- [ ] 3.5 `src/web/portal/announcements.rs`: migrate `announcements_page`, `announcements_list_api`. Needs: `Arc<dyn AnnouncementRepository>`, `Arc<CsrfService>`.
- [ ] 3.6 `src/web/portal/payments.rs`: migrate the many payment handlers. Needs: `Arc<dyn PaymentRepository>`, `Arc<dyn SavedCardRepository>`, `Arc<BillingService>`, `Option<Arc<StripeClient>>`, `Arc<MembershipTypeService>`, `MoneyLimiter`, `Arc<CsrfService>`. This is one of the heavier non-admin files; allow `State<AppState>` exception per the design's D3 if a handler legitimately needs 6+ extractors.
- [ ] 3.7 `src/web/portal/donations.rs`: migrate `donate_page`, `donate_api`. Needs: `Arc<dyn DonationCampaignRepository>`, `Option<Arc<StripeClient>>`, `MoneyLimiter`, `Arc<CsrfService>`.
- [ ] 3.8 `src/web/portal/restore.rs`: migrate `restore_page`. Light — needs `Arc<CsrfService>` and `Arc<MembershipTypeService>` for displayed amounts.
- [ ] 3.9 `src/web/portal/partials.rs`: if it contains handlers (rather than just helpers), migrate them. If it's all helpers called from other handlers, skip.
- [ ] 3.10 `cargo build --all-targets --features test-utils` after each file — green.

## 4. Admin portal handlers (src/web/portal/admin/)

These are the heaviest. Each handler typically needs 2–5 granular extractors. The post-`lift-member-admin-orchestration` `members.rs` is lighter than the rest because the bodies delegate to `MemberService`.

- [ ] 4.1 `src/web/portal/admin/members.rs`: migrate every `admin_*` handler. Typical needs after the prior MemberService lift: `Arc<MemberService>`, `Arc<MembershipTypeService>` (for membership-type dropdown), `Arc<dyn PaymentRepository>` (for payment listings on the detail page), `Arc<CsrfService>`. The refund handler additionally needs `Arc<dyn PaymentRepository>`, `Option<Arc<StripeClient>>`, `MoneyLimiter`.
- [ ] 4.2 `src/web/portal/admin/events.rs`: migrate every `admin_*` handler. Typical needs: `Arc<dyn EventRepository>`, `Arc<EventBasicTypeService>` (the basic-type service wrapping the event kind), `Arc<RecurringEventService>`, `Arc<AuditService>`, `Arc<IntegrationManager>`, `Arc<CsrfService>`.
- [ ] 4.3 `src/web/portal/admin/announcements.rs`: migrate every `admin_*` handler. Typical needs: `Arc<dyn AnnouncementRepository>`, `Arc<AnnouncementBasicTypeService>`, `Arc<AuditService>`, `Arc<IntegrationManager>`, `Arc<CsrfService>`.
- [ ] 4.4 `src/web/portal/admin/types.rs`: migrate every `admin_*` handler. After `consolidate-configurable-types`, the basic-type handlers are parameterized by `:kind`. Needs: both `Arc<EventBasicTypeService>` and `Arc<AnnouncementBasicTypeService>` (for the index page that lists both), plus `Arc<MembershipTypeService>` for the membership branch, `Arc<AuditService>`, `Arc<CsrfService>`.
- [ ] 4.5 `src/web/portal/admin/settings.rs`: migrate every `admin_*` handler. Needs: `Arc<SettingsService>`, `Arc<AuditService>`, `Arc<CsrfService>`.
- [ ] 4.6 `src/web/portal/admin/email.rs`: migrate every `admin_*` handler. Needs: `Arc<SettingsService>`, `Arc<dyn EmailSender>`, `Arc<AuditService>`, `Arc<CsrfService>`.
- [ ] 4.7 `src/web/portal/admin/discord.rs`: migrate every `admin_*` handler. Needs: `Arc<SettingsService>`, `Arc<IntegrationManager>` (or the specific Discord integration), `Arc<MemberRepository>` (for reconcile), `Arc<AuditService>`, `Arc<CsrfService>`.
- [ ] 4.8 `src/web/portal/admin/billing.rs`: migrate every `admin_*` handler. Needs: `Arc<BillingService>`, `Arc<dyn PaymentRepository>`, `Arc<dyn ScheduledPaymentRepository>`, `Arc<AuditService>`, `Arc<CsrfService>`.
- [ ] 4.9 `src/web/portal/admin/audit.rs`: migrate every `admin_*` handler. Needs: `Arc<AuditService>`, `Arc<CsrfService>`.
- [ ] 4.10 `src/web/portal/admin/partials.rs`: if it contains handlers, migrate them. If it's all helper functions, skip.
- [ ] 4.11 `cargo build --all-targets --features test-utils` after each file — green.

## 5. Validation

- [ ] 5.1 `cargo build --all-targets --features test-utils` — final check across the whole project, clean.
- [ ] 5.2 `cargo test --features test-utils` — full suite passes. No handler-level test should require modification; if any fails, investigate the migration before adjusting the test.
- [ ] 5.3 `grep -rn "State<AppState>" src/web/` — count the remaining occurrences. Each should fall into the D3 exception (cross-cutting handler with ≥6 components) and carry a brief comment explaining why. Bare `State<AppState>` without a comment is a defect.

## 6. Spec sync

- [ ] 6.1 Confirm the change's delta spec (`openspec/changes/a06-refactor-portal-handlers-to-fromref/specs/routing-architecture/spec.md`) matches the implemented behavior.
