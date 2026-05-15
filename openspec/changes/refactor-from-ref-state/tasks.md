## 1. Implement FromRef for AppState

- [ ] 1.1 In `src/api/state.rs`, implement `axum::extract::FromRef<AppState>` for `Arc<dyn MemberRepository>`.
- [ ] 1.2 In `src/api/state.rs`, implement `FromRef<AppState>` for `Arc<dyn EventRepository>`.
- [ ] 1.3 In `src/api/state.rs`, implement `FromRef<AppState>` for `Arc<dyn AnnouncementRepository>`.
- [ ] 1.4 In `src/api/state.rs`, implement `FromRef<AppState>` for `Arc<dyn PaymentRepository>`.
- [ ] 1.5 In `src/api/state.rs`, implement `FromRef<AppState>` for `Arc<AuthService>`.
- [ ] 1.6 In `src/api/state.rs`, implement `FromRef<AppState>` for `Arc<SettingsService>`.
- [ ] 1.7 In `src/api/state.rs`, implement `FromRef<AppState>` for `Arc<BillingService>`.
- [ ] 1.8 In `src/api/state.rs`, implement `FromRef<AppState>` for all other commonly used services and repositories in `ServiceContext` (e.g., `EventSeriesRepository`, `DonationCampaignRepository`, `AuditService`, etc.).
- [ ] 1.9 In `src/api/state.rs`, implement `FromRef<AppState>` for rate limiters and locks (`login_limiter`, `money_limiter`, `setup_lock`).

## 2. Refactor Web/Portal Handlers

- [ ] 2.1 Refactor handlers in `src/web/templates/auth.rs` to use `State(...)` with specific dependencies (e.g., `AuthService`, `login_limiter`) instead of `AppState`.
- [ ] 2.2 Refactor handlers in `src/web/templates/setup.rs` to extract `setup_lock` and specific repositories instead of `AppState`.
- [ ] 2.3 Refactor handlers in `src/web/templates/reset.rs` and `verify.rs` to extract specific services.
- [ ] 2.4 Refactor all admin portal handlers (`src/web/portal/`) to extract their required repositories and services.

## 3. Refactor API/Public Handlers

- [ ] 3.1 Refactor handlers in `src/api/handlers/public.rs` to use `FromRef` extraction.
- [ ] 3.2 Refactor handlers in `src/api/handlers/payments.rs` to use `FromRef` extraction.
- [ ] 3.3 Refactor handlers in `src/api/handlers/auth.rs` to use `FromRef` extraction.
- [ ] 3.4 Refactor handlers in `src/api/handlers/announcements.rs` to use `FromRef` extraction.

## 4. Validation

- [ ] 4.1 Run `cargo check` to ensure all handlers correctly extract their state and compile successfully.
- [ ] 4.2 Run `cargo test` to verify that no functional regressions were introduced during the refactoring.