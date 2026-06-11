## Context

The `admin-types` capability spec was written assuming the handlers do audit emission. They don't. This change makes the code match the spec — pure addition, no architectural decisions to make.

The handlers in `src/web/portal/admin/types.rs`:
- `admin_create_basic_type` (line 253) — handles both event and announcement basic types via the `BasicTypeKind` enum
- `admin_update_basic_type` (line 283) — same
- `admin_delete_basic_type` (line 319) — same
- `admin_create_membership_type` (line 424) — membership types only
- `admin_update_membership_type` (line 453) — same
- `admin_delete_membership_type` (line 489) — same

None currently take `AuditService` as a State extractor; the change adds that.

## Goals / Non-Goals

**Goals:**
- Every successful type mutation writes exactly one audit row.
- Action strings are searchable and consistent (`{create,update,delete}_{event,announcement,membership}_type`).
- For updates and deletes, `old_value` captures the pre-change name so post-deletion forensics work.
- Audit insert failure does NOT roll back the mutation (per `audit-logging` spec's fire-and-forget contract).

**Non-Goals:**
- Changing the service layer to emit audit. The `admin-types` spec says handlers do it — that stays the design.
- Changing the action-string convention or audit row shape. Use whatever pattern other admin handlers already use.
- Adding audit for read-only handlers (page renders, list fetches). Only mutations.

## Decisions

### D1. Action strings

Per-kind action strings rather than a single `mutate_type` with kind in `old_value`:

| Kind | Create | Update | Delete |
|------|--------|--------|--------|
| Event type | `create_event_type` | `update_event_type` | `delete_event_type` |
| Announcement type | `create_announcement_type` | `update_announcement_type` | `delete_announcement_type` |
| Membership type | `create_membership_type` | `update_membership_type` | `delete_membership_type` |

Reason: audit log queries like "show me all membership-type changes" become straightforward `WHERE action LIKE '%membership_type'`. With a generic action, the kind would be buried in old_value/new_value as freeform text.

### D2. entity_type strings

`event_type`, `announcement_type`, `membership_type`. These match the action-string suffix; same kind ↔ same entity_type.

### D3. old_value / new_value

- **Create**: `old_value=None`, `new_value=Some(&name)` where name is the just-created type's display name.
- **Update**: `old_value=Some(&old_name)`, `new_value=Some(&new_name)`. The handler reads the existing type's name before calling the service so the old value is available even if the service's update returns only the new value.
- **Delete**: `old_value=Some(&name)`, `new_value=None`. The handler reads the type's name before calling delete so we capture what was removed.

If a future contributor wants richer audit (e.g., serializing the full before/after state as JSON), the row shape supports it — `old_value` and `new_value` are opaque strings. Don't pre-optimize.

### D4. IP address

Per the `audit-logging` spec's row shape, `ip_address` is optional. If the handler already extracts `SessionInfo` (which contains the client IP), pass it through. If not, pass `None` — operator can decide later whether to wire IPs through.

For the basic-type handlers, the current signatures don't extract `SessionInfo`. The change adds it for IP capture. For membership-type handlers, same.

### D5. The currently-unused `_current_user` becomes used

Each handler has `Extension(_current_user): Extension<CurrentUser>` (underscore = unused) just to enforce the admin-redirect middleware. After this change, the handlers actually use it: `actor_id = current_user.member.id`. Drop the underscore.

### D6. Helper for the basic-type kind mapping

The basic-type handlers handle two kinds (Event, Announcement) through one function. Both audit-action and entity-type strings depend on the kind. Centralize:

```rust
fn audit_strings_for_kind(kind: BasicTypeKind, op: &'static str) -> (String, &'static str) {
    let entity_type = match kind {
        BasicTypeKind::Event => "event_type",
        BasicTypeKind::Announcement => "announcement_type",
    };
    let action = format!("{op}_{entity_type}");
    (action, entity_type)
}
```

Called as `audit_strings_for_kind(kind, "create")`, etc. Lives in the same file or a small helper module.

### D7. Fire-and-forget per the audit-logging spec

Per `audit-logging` spec: `audit_service.log` returns `()` and swallows DB errors via tracing. Handlers call `audit_service.log(...).await` and don't propagate. Existing pattern across the codebase.

## Risks / Trade-offs

- **Risk**: a misbehaving audit_service insert silently drops rows. → Mitigation: per spec, that's documented and acceptable.
- **Risk**: handler signatures grow (one more State extractor, one more Extension). → Acceptable; signatures are already long.
- **Risk**: integration tests that asserted "no audit row exists for type mutations" (none today, just hypothetical) would break. → No such tests exist; verified via grep.
- **Trade-off**: 6 handlers × ~5 lines of audit-related code = ~30 new lines. Worth it for forensic capability.

## Migration Plan

Single PR.

1. Add the `audit_strings_for_kind` helper (or skip and inline if it doesn't read better).
2. For each of the 6 handlers:
   - Add `State(audit_service): State<Arc<AuditService>>` to the signature.
   - Add `State(session_info): State<SessionInfo>` or `Extension<SessionInfo>` (whichever pattern matches the codebase) for IP capture.
   - Drop the underscore on `_current_user`.
   - For update/delete, fetch the pre-mutation type and bind its name to a local before calling the service.
   - After the service's `Ok(...)` arm, call `audit_service.log(...)` with the right action, entity_type, entity_id, old_value, new_value, ip_address.
3. Add integration tests in `tests/admin_types_audit_test.rs` (or extend an existing test file) covering one create, one update, one delete per kind.
4. `cargo test`, `cargo clippy`, `cargo fmt` all clean.
