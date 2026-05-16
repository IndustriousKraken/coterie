## 1. Migrate root and announcements handlers

- [ ] 1.1 `src/api/handlers/root.rs`: inspect `root`, `health_check`, `api_info`. If they extract `State<AppState>`, replace with the smallest sufficient extractor (or remove the extractor entirely if none of `state.*` is used).
- [ ] 1.2 `src/api/handlers/announcements.rs::private_count`: migrate to `State<Arc<dyn AnnouncementRepository>>`. Body rewrites `state.service_context.announcement_repo.count_private_published()` to `announcement_repo.count_private_published()`.
- [ ] 1.3 `cargo build --all-targets --features test-utils` — green.

## 2. Migrate auth handlers

- [ ] 2.1 `src/api/handlers/auth.rs::login`: migrate to granular extraction. Needs: `Arc<AuthService>`, `Arc<TotpService>`, `Arc<PendingLoginService>`, `Arc<MemberRepository>`, `LoginLimiter` (the newtype), `Arc<AuditService>`. If the body also reads `state.settings` for session config, add `Arc<Settings>`.
- [ ] 2.2 `src/api/handlers/auth.rs::logout`: migrate. Needs: `Arc<AuthService>`, `Arc<AuditService>`.
- [ ] 2.3 `cargo build --all-targets --features test-utils` — green.

## 3. Migrate public handlers

- [ ] 3.1 `src/api/handlers/public.rs::signup`: migrate. Needs: `Arc<MemberRepository>`, `Arc<MembershipTypeService>`, `Arc<dyn BotChallengeVerifier>`, `Arc<dyn EmailSender>`, `Arc<Settings>` (for org-name).
- [ ] 3.2 `src/api/handlers/public.rs::donate`: migrate. Needs: `Arc<dyn DonationCampaignRepository>`, `Option<Arc<StripeClient>>`, `Arc<dyn BotChallengeVerifier>`, `MoneyLimiter` (per the rate-limiting spec, public donate is gated by the money limiter), `Arc<dyn PaymentRepository>` (to seed the pending payment row).
- [ ] 3.3 `src/api/handlers/public.rs::list_events`: migrate. Needs: `Arc<dyn EventRepository>`.
- [ ] 3.4 `src/api/handlers/public.rs::list_announcements`: migrate. Needs: `Arc<dyn AnnouncementRepository>`.
- [ ] 3.5 `src/api/handlers/public.rs::private_event_count`: migrate. Needs: `Arc<dyn EventRepository>`.
- [ ] 3.6 `src/api/handlers/public.rs::rss_feed`: migrate. Needs: `Arc<dyn AnnouncementRepository>`, possibly `Arc<Settings>` for the feed metadata (org name, base URL).
- [ ] 3.7 `src/api/handlers/public.rs::calendar_feed`: migrate. Needs: `Arc<dyn EventRepository>`, possibly `Arc<Settings>`.
- [ ] 3.8 `cargo build --all-targets --features test-utils` — green.

## 4. Migrate payment handlers

- [ ] 4.1 `src/api/handlers/payments.rs::create_setup_intent`: migrate. Needs: `Option<Arc<StripeClient>>`, `Arc<MemberRepository>` (to load the member for Stripe customer-id resolution).
- [ ] 4.2 `src/api/handlers/payments.rs::save_card`: migrate. Needs: `Option<Arc<StripeClient>>`, `Arc<dyn SavedCardRepository>`, `Arc<MemberRepository>`, `Arc<BillingService>` (the post-save migration step).
- [ ] 4.3 `src/api/handlers/payments.rs::stripe_webhook`: migrate. Needs: `Option<Arc<WebhookDispatcher>>`, `Arc<BillingService>`, `Arc<Settings>` (for webhook secret), possibly more depending on what `webhook_dispatcher.handle_webhook(...)` requires. If the signature would have ≥6 extractors, retain `State<AppState>` with an inline comment per the design's exception clause.
- [ ] 4.4 `cargo build --all-targets --features test-utils` — green.

## 5. Validation

- [ ] 5.1 `cargo build --all-targets --features test-utils` — final check across the whole project, clean.
- [ ] 5.2 `cargo test --features test-utils` — full suite passes. Stripe webhook tests, saved-card routes test, signup tests etc. continue to pass.
- [ ] 5.3 `grep -rn "State<AppState>" src/api/handlers/` — count remaining occurrences. Each must carry a comment explaining the exception (most likely just `stripe_webhook` if at all). Bare uncommented `State<AppState>` is a defect.

## 6. Spec sync

- [ ] 6.1 Confirm the change's delta spec (`openspec/changes/a07-refactor-api-handlers-to-fromref/specs/routing-architecture/spec.md`) matches the implemented behavior.
