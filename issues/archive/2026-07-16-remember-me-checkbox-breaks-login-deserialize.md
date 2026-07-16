## Report (issues lane candidate, a010)

Origin: maintainer report.

Source: reported issue #99 — Remember me possibly not working or flaky

## Diagnosis (maintainer-approved classification)

The login template posts the "remember_me" checkbox via json-enc as the string "on", but LoginRequest.remember_me is Option<bool>, so Axum's Json extractor rejects the body and login fails whenever "Remember me" is checked — violating session-auth's "valid credentials create a session" requirement. Fixing the field's wire type restores conformance with no spec change.

## Acceptance

The code must conform to the EXISTING specification in `openspec/specs/`. This is a bug fix; it carries NO spec delta. If the fix would require a behavior change, kick it back to the changes lane.

## Tasks

- [x] 1.1 In src/web/templates/auth.rs, change LoginRequest.remember_me from Option<bool> to `#[serde(default)] remember_me: Option<String>`, matching the existing HTML-checkbox convention used in src/web/portal/admin/unifi.rs / discord.rs / stripe.rs (present="on" when checked, absent otherwise).
- [x] 1.2 Update the three read sites in login_handler (the pending_login create call and the two session/cookie duration branches) to treat the checkbox as checked via `credentials.remember_me.is_some()` instead of `.unwrap_or(false)`.
