## Why

Currently, all Axum handlers receive the entire `AppState` God Object, which contains the `ServiceContext` and thereby every service and repository in the application. This breaks the principle of least privilege, as handlers have access to the entire domain rather than just the specific dependencies they require, making it harder to reason about handler dependencies.

## What Changes

- Modify `AppState` to implement Axum's `FromRef` trait for individual services, repositories, and other state components.
- Update Axum handlers across the application to extract only the specific dependencies they need via `State(service): State<Arc<dyn SpecificService>>` rather than extracting the full `AppState`.
- This is a purely structural refactoring; no external behavior or capabilities will be modified.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `src/api/state.rs` will be modified to implement `FromRef` for various inner components.
- Axum handlers in `src/api/handlers/` and `src/web/` will be updated to extract granular state.
- No database migrations, API contract changes, or external system dependencies are affected.