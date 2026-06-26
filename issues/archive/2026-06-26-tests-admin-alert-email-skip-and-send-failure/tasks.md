# Tasks

Add a new integration test file `tests/admin_alert_email_test.rs`. Build
`AdminAlertEmailIntegration::new(settings, sender)` directly:

- `settings` = `Arc::new(SettingsService::new(pool.clone(), crypto))` with
  `crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"))` and
  `pool = common::fresh_pool().await` — the same construction used in
  `tests/auto_renew_alert_test.rs`.
- Seed `org.contact_email` (and, where needed, `org.name`) by raw
  `INSERT`/`UPDATE` into the `settings` table that `SettingsService::get_value`
  reads (mirror the direct-SQL settings seeding in
  `tests/expiration_test.rs`). For the "skip" case, leave `org.contact_email`
  empty/unset.
- Define two tiny in-test `EmailSender` impls: a `RecordingSender`
  (`Arc<Mutex<Vec<EmailMessage>>>`, returns `Ok(())`) and a `FailingSender`
  (returns `Err(AppError::External(...))`). `NoopEmailSender` in
  `tests/auto_renew_alert_test.rs` is the pattern to copy.

Invoke the handler via the `Integration` trait:
`integration.handle_event(&IntegrationEvent::AdminAlert { subject, body })`.

## 1. Empty recipient setting → no send
- [x] 1.1 `admin_alert_email_skips_when_contact_email_unset` — with
  `org.contact_email` empty/unset and a `RecordingSender`, call
  `handle_event(&AdminAlert { subject: "x".into(), body: "y".into() })`;
  assert it returns `Ok(())` AND the recorder is empty (sender never called).

## 2. Send failure is absorbed, not propagated
- [x] 2.1 `admin_alert_email_send_failure_is_swallowed` — with
  `org.contact_email = "ops@example.org"` and a `FailingSender`, call
  `handle_event(&AdminAlert { .. })`; assert it returns `Ok(())` (the
  sender's `Err` does not surface).

## 3. Configured recipient drives the To: list
- [x] 3.1 `admin_alert_email_sends_to_configured_recipient` — with
  `org.contact_email = "ops@example.org"` and a `RecordingSender`, call
  `handle_event(&AdminAlert { .. })`; assert it returns `Ok(())`, the
  recorder holds exactly one `EmailMessage`, and that message's recipient
  equals `"ops@example.org"`.

## 4. Non-AdminAlert events are a no-op
- [x] 4.1 `admin_alert_email_ignores_non_alert_events` — with a
  `RecordingSender`, call `handle_event` with any non-`AdminAlert`
  `IntegrationEvent` variant; assert it returns `Ok(())` and the recorder is
  empty.
