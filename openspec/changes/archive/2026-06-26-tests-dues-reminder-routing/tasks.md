## 1. Test harness

- [x] 1.1 Add `tests/dues_reminder_test.rs`. Build a `Notifications` via
  `Notifications::new(...)` wired with a recording `EmailSender`
  (`Arc<Mutex<Vec<EmailMessage>>>`, returns `Ok(())`), plus the repos and
  services (`SavedCardRepository`, `MembershipTypeService`, `SettingsService`,
  member repo, `db_pool`, `base_url`) constructed exactly as in
  `tests/auto_renew_alert_test.rs`. Reuse that file's `seed_default_card`
  helper and member/membership-type seeding.
- [x] 1.2 Helper `set_dues_window(pool, member_id, days_from_now)` — set
  `members.dues_paid_until` so the member falls inside the reminder window
  (e.g. 3 days out), `status = 'Active'`, `bypass_dues = 0`,
  `dues_reminder_sent_at = NULL`; set `membership.reminder_days_before` to a
  value that includes it (e.g. 7) via direct SQL into `settings`.

## 2. Case 1 — manual billing sends a plain reminder

- [x] 2.1 `manual_member_gets_plain_reminder` — seed an Active manual-billing
  member in the window; run `send_dues_reminders()`; assert it returns `1`,
  the recorder holds one message whose subject contains `"dues are due soon"`,
  and the body does NOT contain the card-invalid callout.

## 3. Case 2 — auto-renew + valid card + monthly is skipped and stays eligible

- [x] 3.1 `autorenew_monthly_valid_card_is_skipped` — seed a CoterieManaged
  member on a monthly membership type with a default card valid past
  `dues_paid_until`; run `send_dues_reminders()`; assert it returns `0` and
  no message was recorded.
- [x] 3.2 `autorenew_monthly_skip_does_not_set_reminder_flag` — after 3.1's
  run, assert `members.dues_reminder_sent_at` is still `NULL` for that member
  (case 2 must leave them eligible for a later cycle).

## 4. Case 3 — auto-renew + valid card + yearly gets a renewal notice

- [x] 4.1 `autorenew_yearly_valid_card_gets_renewal_notice` — seed a
  CoterieManaged member on a yearly membership type with a valid default card;
  run `send_dues_reminders()`; assert it returns `1` and the recorded
  message's subject contains `"will renew"` (the renewal-notice subject), not
  `"dues are due soon"`.

## 5. Case 4 — auto-renew + invalid/missing card gets reminder with callout

- [x] 5.1 `autorenew_expired_card_gets_reminder_with_card_invalid_callout` —
  seed a CoterieManaged member whose default card expires BEFORE
  `dues_paid_until` (or seed no card); run `send_dues_reminders()`; assert it
  returns `1`, the subject contains `"dues are due soon"`, and the body
  contains the card-invalid callout (the `card_invalid = true` branch of the
  `ReminderHtml`/`ReminderText` templates).

## 6. Lifetime members are skipped

- [x] 6.1 `lifetime_member_is_skipped` — seed an Active member on a lifetime
  membership type who somehow lands in the window; run
  `send_dues_reminders()`; assert it returns `0` and no message recorded.

## 7. Idempotency within a cycle

- [x] 7.1 `reminder_is_idempotent_within_cycle` — run `send_dues_reminders()`
  for a manual member (Case 1), assert `1` sent, then run it again without
  resetting `dues_reminder_sent_at` and assert the second run returns `0` and
  records no additional message.
