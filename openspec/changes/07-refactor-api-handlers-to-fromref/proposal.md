## Why

After `add-fromref-impls-on-appstate` lands, `AppState` exposes `FromRef<AppState>` impls for every constituent service, repository, and piece of infrastructure. This change migrates the API-side handlers (`src/api/handlers/*.rs`) from `State<AppState>` extraction to granular `State<Arc<dyn TargetService>>` extraction.

The API surface is narrower than the portal surface — about 4 files containing ~10–15 handler signatures (Stripe webhook, saved-card endpoints, public signup/donate/feeds, login/logout, root/health). After the prior `narrow-saved-card-json-surface` change, three saved-card endpoints are deleted; the remaining surface is the truly load-bearing JSON path.

This is the third of three changes split out from the original `refactor-from-ref-state` work. It's the smallest of the three because the API surface is intentionally narrow per CLAUDE.md, and it's independent of `refactor-portal-handlers-to-fromref` — both depend on `add-fromref-impls-on-appstate` but neither depends on the other.

## What Changes

- **For every handler in `src/api/handlers/`**: replace `State<AppState>` with one or more granular `State<Arc<…>>` extractors based on what the handler body actually uses.
  - `src/api/handlers/public.rs` — `signup`, `donate`, `list_events`, `list_announcements`, `rss_feed`, `calendar_feed`, `private_event_count`. Typical needs: `Arc<MemberRepository>`, `Arc<MembershipTypeService>`, `Arc<dyn EventRepository>`, `Arc<dyn AnnouncementRepository>`, `Option<Arc<StripeClient>>`, `Arc<dyn BotChallengeVerifier>`, `Arc<dyn EmailSender>` (signup confirmation), `Arc<Settings>` (org-name lookup).
  - `src/api/handlers/payments.rs` — `stripe_webhook`, `create_setup_intent`, `save_card`. Typical needs: `Option<Arc<WebhookDispatcher>>`, `Option<Arc<StripeClient>>`, `Arc<BillingService>`, `Arc<dyn PaymentRepository>`, `Arc<dyn SavedCardRepository>`, `Arc<MemberRepository>`, `Arc<AuditService>`.
  - `src/api/handlers/auth.rs` — `login`, `logout`. Needs: `Arc<AuthService>`, `Arc<TotpService>`, `Arc<PendingLoginService>`, `Arc<MemberRepository>`, `LoginLimiter`, `Arc<AuditService>`.
  - `src/api/handlers/announcements.rs` — `private_count`. Needs: `Arc<dyn AnnouncementRepository>`.
  - `src/api/handlers/root.rs` — `root`, `health_check`, `api_info`. These typically need nothing from state and may not even take `State<…>` today; check each.
- **No behavioral changes.** Same URLs, same status codes, same response shapes, same audit emission, same webhook routing.
- **No routing changes.** `src/api/mod.rs` continues to register routes against `AppState`.

## Capabilities

### New Capabilities

(None — this is an internal refactor matching the pattern established in `refactor-portal-handlers-to-fromref`.)

### Modified Capabilities
- `routing-architecture`: extends the granular-extraction requirement from `refactor-portal-handlers-to-fromref` to cover API handlers under `src/api/handlers/`. Same documented-exception clause for handlers with genuine cross-cutting needs.

## Impact

- **Code**: ~5 files in `src/api/handlers/`, ~10–15 handler signatures rewritten. Bodies change only where they previously read `state.<path>`.
- **Wire shape**: zero change.
- **Tests**: existing tests assert HTTP responses; they continue to pass. The integration tests added by `narrow-saved-card-json-surface` (`tests/saved_card_routes_test.rs`) and any other handler-level tests are the regression net.
- **Risk**: low. The API surface is small and well-tested. Mechanical.
- **Dependency**: requires `add-fromref-impls-on-appstate` to have landed. Independent of `refactor-portal-handlers-to-fromref` — can run in parallel or in either order.
- **Sequencing in the autocoder queue**: ordered `07-` (after `06-` alphabetically) but no logical dependency on `06`. If `06` times out or splits further, `07` is unaffected.
