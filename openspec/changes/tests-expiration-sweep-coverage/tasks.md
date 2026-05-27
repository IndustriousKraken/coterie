## 1. Add coverage for `Expiration::check_expired_members`

- [ ] 1.1 `expires_active_member_past_grace_period` — seed an Active
  member with `dues_paid_until = now - 10 days`, set the grace-period
  setting to `3`. Run `check_expired_members`. Assert returned count
  is `1`. Assert the member's status row reads `Expired`. Assert the
  `RecordingIntegration` received exactly one
  `IntegrationEvent::MemberExpired` for that member.
- [ ] 1.2 `does_not_expire_member_within_grace_period` — seed an
  Active member with `dues_paid_until = now - 1 day`, grace = `3`.
  Run the sweep. Assert returned count is `0` and the member's status
  remains `Active`.
- [ ] 1.3 `does_not_expire_bypass_dues_member` — seed an Active
  member with `bypass_dues = 1` and `dues_paid_until = now - 999
  days`. Run the sweep. Assert returned count is `0` and status
  remains `Active` (the `AND bypass_dues = 0` clause holds).
- [ ] 1.4 `expiration_invalidates_live_sessions` — seed an Active
  member past grace AND an active `sessions` row for that member.
  Run the sweep. Assert the `sessions` row is gone (the
  member-id IN (...) DELETE fires).
- [ ] 1.5 `expiration_uses_default_grace_when_setting_unset` —
  seed an Active member with `dues_paid_until = now - 5 days`. Do
  NOT set `membership.grace_period_days` (so the setting is unset).
  Run the sweep. Assert returned count is `1` (default = 3 days
  applied, and 5 > 3).
