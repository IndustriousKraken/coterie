## Why

Today every Axum handler in Coterie extracts the full `AppState` God Object via `State(state): State<AppState>`. The state contains the `ServiceContext` and thereby every service and repository in the application. Each handler has access to the entire domain, which breaks the principle of least privilege, complicates testing, and obscures the specific dependencies any given route actually needs.

The fix is `axum::extract::FromRef`. Once `AppState` exposes `FromRef<AppState>` impls for each of its constituent services and repositories, handlers can write `State(svc): State<Arc<dyn TargetService>>` to extract exactly what they need.

This change is the FIRST of three split out from the original `refactor-from-ref-state` work. It is purely additive: it adds the impls without touching any handler. Handlers continue to use `State<AppState>` and compile unchanged. The follow-up changes (`refactor-portal-handlers-to-fromref` and `refactor-api-handlers-to-fromref`) migrate handlers to the new extraction shape.

Splitting the original change into three keeps each change's scope bounded enough that an autocoder run completes within a single ~30-minute budget. This first change is intentionally the smallest of the three.

## What Changes

- **Add `FromRef<AppState>` impls** in `src/api/state.rs` for every component a handler might reasonably want to extract:
  - **Repositories**: `Arc<dyn MemberRepository>`, `Arc<dyn EventRepository>`, `Arc<dyn AnnouncementRepository>`, `Arc<dyn PaymentRepository>`, `Arc<dyn EventSeriesRepository>`, `Arc<dyn SavedCardRepository>`, `Arc<dyn ScheduledPaymentRepository>`, `Arc<dyn DonationCampaignRepository>`, `Arc<dyn EventTypeRepository>`, `Arc<dyn AnnouncementTypeRepository>`, `Arc<dyn MembershipTypeRepository>`, `Arc<dyn ProcessedEventsRepository>`.
  - **Services**: `Arc<AuthService>`, `Arc<CsrfService>`, `Arc<TotpService>`, `Arc<PendingLoginService>`, `Arc<SettingsService>`, `Arc<AuditService>`, `Arc<PaymentService>`, `Arc<RecurringEventService>`, `Arc<MembershipTypeService>`, plus the now-collapsed `Arc<BasicTypeService>` instances (event and announcement kinds), and `Arc<MemberService>` (added by the earlier `lift-member-admin-orchestration` change).
  - **Infrastructure**: `Arc<BillingService>`, `Option<Arc<StripeClient>>`, `Option<Arc<WebhookDispatcher>>`, `Arc<dyn EmailSender>`, `Arc<IntegrationManager>`, `Arc<dyn BotChallengeVerifier>`, `Arc<Settings>`.
  - **Locks / limiters**: `RateLimiter` (for `login_limiter` and `money_limiter` — each gets its own newtype if both need to be extractable; otherwise a single impl is ambiguous), `Arc<AsyncMutex<()>>` for `setup_lock`, `Arc<AtomicBool>` for `admin_exists_observed`, and `SqlitePool`.
- **No handler changes**. Every existing handler keeps its `State<AppState>` extractor. After this change, both extraction patterns are valid — handlers can extract `AppState` (existing) or specific dependencies (new).
- **No spec change** beyond documenting that `FromRef<AppState>` is supported.

## Capabilities

### New Capabilities

(None — this is internal plumbing inside an existing capability.)

### Modified Capabilities
- `routing-architecture`: adds an internal-structure requirement that `AppState` exposes `FromRef<AppState>` impls for each of its constituent services, repositories, and infrastructure components, enabling granular state extraction in handlers. Externally-visible routing behavior is unchanged.

## Impact

- **Code**: `src/api/state.rs` grows by ~40–60 impl blocks (one per FromRef target). Each is 3–5 lines: `impl axum::extract::FromRef<AppState> for Arc<X> { fn from_ref(state: &AppState) -> Self { state.<field>.clone() } }`.
- **Wire shape**: zero change. No handler signature changes, no routing changes.
- **Compilation**: clean. Unused `FromRef` impls are trait impls, not items — the `dead_code` lint does not fire on them. Handlers that continue to use `State<AppState>` compile exactly as today.
- **Tests**: no test changes required. `cargo check` and `cargo test` are the validation gates.
- **Risk**: very low. The change adds code without removing or modifying any existing path.
- **Sequencing**: `refactor-portal-handlers-to-fromref` and `refactor-api-handlers-to-fromref` depend on this change. Both can run in either order after this lands; neither depends on the other.
