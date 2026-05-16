## 1. Repository FromRef impls

- [ ] 1.1 In `src/api/state.rs`, add `impl axum::extract::FromRef<AppState> for Arc<dyn MemberRepository>` that returns `state.service_context.member_repo.clone()`.
- [ ] 1.2 Same for `Arc<dyn EventRepository>` → `state.service_context.event_repo`.
- [ ] 1.3 Same for `Arc<dyn EventSeriesRepository>` → `state.service_context.event_series_repo`.
- [ ] 1.4 Same for `Arc<dyn AnnouncementRepository>` → `state.service_context.announcement_repo`.
- [ ] 1.5 Same for `Arc<dyn PaymentRepository>` → `state.service_context.payment_repo`.
- [ ] 1.6 Same for `Arc<dyn SavedCardRepository>` → `state.service_context.saved_card_repo`.
- [ ] 1.7 Same for `Arc<dyn ScheduledPaymentRepository>` → `state.service_context.scheduled_payment_repo`.
- [ ] 1.8 Same for `Arc<dyn DonationCampaignRepository>` → `state.service_context.donation_campaign_repo`.
- [ ] 1.9 Same for `Arc<dyn BasicTypeRepository>` → `state.service_context.basic_type_repo` (added by the earlier `consolidate-configurable-types` change).
- [ ] 1.10 Same for `Arc<dyn MembershipTypeRepository>` → `state.service_context.membership_type_repo` (or wherever it lives on `ServiceContext`).
- [ ] 1.11 Same for `Arc<dyn ProcessedEventsRepository>` → its field on `ServiceContext`.

## 2. Service FromRef impls

- [ ] 2.1 In `src/api/state.rs`, add `impl FromRef<AppState>` for `Arc<AuthService>` → `state.service_context.auth_service.clone()`.
- [ ] 2.2 Same for `Arc<CsrfService>` → `state.service_context.csrf_service`.
- [ ] 2.3 Same for `Arc<TotpService>` → `state.service_context.totp_service`.
- [ ] 2.4 Same for `Arc<PendingLoginService>` → `state.service_context.pending_login_service`.
- [ ] 2.5 Same for `Arc<SettingsService>` → `state.service_context.settings_service`.
- [ ] 2.6 Same for `Arc<AuditService>` → `state.service_context.audit_service`.
- [ ] 2.7 Same for `Arc<PaymentService>` → `state.service_context.payment_service`.
- [ ] 2.8 Same for `Arc<RecurringEventService>` → `state.service_context.recurring_event_service`.
- [ ] 2.9 Same for `Arc<MembershipTypeService>` → `state.service_context.membership_type_service`.
- [ ] 2.10 Same for the two `Arc<BasicTypeService>` instances (event-kind and announcement-kind). These share a type; the field names on `ServiceContext` are `event_type_service` and `announcement_type_service`. Since both are `Arc<BasicTypeService>`, a bare `FromRef<AppState> for Arc<BasicTypeService>` is ambiguous — wrap each in a newtype (`EventBasicTypeService(pub Arc<BasicTypeService>)`, `AnnouncementBasicTypeService(pub Arc<BasicTypeService>)`) and provide `FromRef` for each newtype.
- [ ] 2.11 Same for `Arc<MemberService>` → `state.service_context.member_service` (added by the earlier `lift-member-admin-orchestration` change).
- [ ] 2.12 Same for `Arc<dyn EmailSender>` → `state.service_context.email_sender`.
- [ ] 2.13 Same for `Arc<IntegrationManager>` → `state.service_context.integration_manager`.

## 3. Infrastructure FromRef impls

- [ ] 3.1 Add `impl FromRef<AppState> for Arc<BillingService>` → `state.billing_service.clone()`.
- [ ] 3.2 Add `impl FromRef<AppState> for Option<Arc<StripeClient>>` → `state.stripe_client.clone()`. The `Option` is preserved; handlers that need a configured Stripe still match on it.
- [ ] 3.3 Add `impl FromRef<AppState> for Option<Arc<WebhookDispatcher>>` → `state.webhook_dispatcher.clone()`.
- [ ] 3.4 Add `impl FromRef<AppState> for Arc<dyn BotChallengeVerifier>` → `state.bot_challenge_verifier.clone()`.
- [ ] 3.5 Add `impl FromRef<AppState> for Arc<Settings>` → `state.settings.clone()`.
- [ ] 3.6 Add `impl FromRef<AppState> for SqlitePool` → `state.service_context.db_pool.clone()`.

## 4. Rate limiter and lock FromRef impls (with newtype disambiguation)

- [ ] 4.1 Declare `pub struct LoginLimiter(pub RateLimiter);` and `pub struct MoneyLimiter(pub RateLimiter);` in `src/api/state.rs`. Both wrap `RateLimiter` directly (no `Arc` — `RateLimiter` is already `Clone` and cheap to clone via its inner `Arc<Mutex<...>>`).
- [ ] 4.2 Add `impl FromRef<AppState> for LoginLimiter` returning `LoginLimiter(state.login_limiter.clone())`.
- [ ] 4.3 Add `impl FromRef<AppState> for MoneyLimiter` returning `MoneyLimiter(state.money_limiter.clone())`.
- [ ] 4.4 Add `impl FromRef<AppState> for Arc<AsyncMutex<()>>` → `state.setup_lock.clone()`. Note: if multiple `Arc<AsyncMutex<()>>` ever appear on `AppState`, this needs its own newtype too. Today there's only `setup_lock`, so a bare impl is fine.
- [ ] 4.5 Add `impl FromRef<AppState> for Arc<AtomicBool>` → `state.admin_exists_observed.clone()`. Same note about uniqueness; today there's only one `Arc<AtomicBool>`.

## 5. Verify

- [ ] 5.1 `cargo build --all-targets --features test-utils` — clean. No new warnings expected.
- [ ] 5.2 `cargo test --features test-utils` — full suite passes. No handler changes in this change, so behavior is byte-identical to pre-change.
- [ ] 5.3 Eyeball: `grep -c "^impl axum::extract::FromRef<AppState>" src/api/state.rs` should yield ~30 lines (each FromRef impl starts on its own line). If the count is meaningfully lower than expected, an impl was missed.

## 6. Spec sync

- [ ] 6.1 Confirm the change's delta spec (`openspec/changes/a05-add-fromref-impls-on-appstate/specs/routing-architecture/spec.md`) matches the implemented behavior.
