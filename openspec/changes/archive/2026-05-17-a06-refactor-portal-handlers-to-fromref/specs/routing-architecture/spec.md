## ADDED Requirements

### Requirement: Portal handlers extract granular state, not AppState

Every handler in `src/web/templates/`, `src/web/portal/`, and `src/web/portal/admin/` SHALL extract its dependencies via `State<Arc<dyn TargetService>>` (or analogous granular wrappers) rather than `State<AppState>`. The handler signature SHALL list exactly the services, repositories, and infrastructure components the body actually uses.

Exceptions are allowed for handlers that genuinely use a broad cross-section (≥6 components, or otherwise unreasonable to enumerate). Such exceptions SHALL carry a brief comment explaining the choice. The default position is granular extraction.

Middleware (functions wired via `from_fn_with_state`) SHALL continue to take `State<AppState>` — `FromRef` is for handler extraction, not for middleware.

#### Scenario: New portal handler uses granular extraction

- **WHEN** a contributor adds a new portal handler that uses a few specific services
- **THEN** the handler SHALL extract those services individually via `State<Arc<…>>`; it SHALL NOT default to `State<AppState>` for convenience

#### Scenario: Reader can see a handler's dependencies from its signature

- **WHEN** a reader inspects a portal handler's signature
- **THEN** the granular extractors SHALL enumerate exactly the dependencies the body uses; no domain navigation is needed to discover the actual surface

#### Scenario: Exception is documented at the site

- **WHEN** a contributor retains `State<AppState>` for a handler with broad cross-cutting needs
- **THEN** a brief inline comment SHALL explain why (e.g., "Builds three integration events and reaches five services; granular extraction would yield 7 extractors")

### Requirement: BaseContext takes granular inputs, not AppState

The `BaseContext::for_member` helper SHALL take granular inputs (`csrf_service: &CsrfService`, `current_user: &CurrentUser`, `session: &SessionInfo`) rather than `&AppState`. This is so handlers that build a `BaseContext` for an Askama template can themselves use granular extraction without retaining `State<AppState>` solely to feed the helper.

#### Scenario: Handler building BaseContext uses granular extraction

- **WHEN** a portal handler renders an Askama page using `BaseContext::for_member(...)`
- **THEN** the handler SHALL extract `State<Arc<CsrfService>>` granularly and pass `&csrf_service` to the helper; the handler SHALL NOT retain `State<AppState>` solely for the helper's sake
