## ADDED Requirements

### Requirement: Dues reminders route by billing mode and card validity

`Notifications::send_dues_reminders` SHALL select reminder recipients as
Active, non-`bypass_dues` members whose `dues_paid_until` falls within the
configured reminder window (`membership.reminder_days_before`, clamped to
1..=90, default 7) and whose `dues_reminder_sent_at` is unset. For each
such member the runner SHALL choose exactly one of four outcomes based on
billing mode, the default saved card's validity **at the charge date**
(`dues_paid_until`), and the membership type's billing period:

1. Manual billing → a "dues due soon" reminder with no card-invalid
   callout.
2. Auto-renew (CoterieManaged or StripeSubscription) with a card valid at
   the charge date and a monthly period → no email, and `dues_reminder_sent_at`
   SHALL NOT be set (so the member remains eligible if their card or mode
   changes mid-window).
3. Auto-renew with a card valid at the charge date and a yearly period →
   a renewal notice announcing the upcoming auto-charge.
4. Auto-renew with no card or a card that is not valid at the charge date →
   the "dues due soon" reminder plus a card-invalid callout.

Lifetime-period members SHALL be skipped (no email). Sending a reminder
(cases 1, 3, 4) SHALL mark `dues_reminder_sent_at` so the same member is
not reminded twice in one cycle; case 2 SHALL NOT mark it. The method
SHALL return the count of emails actually sent.

#### Scenario: Manual member receives a plain reminder

- **WHEN** an Active manual-billing member's `dues_paid_until` is within the reminder window
- **THEN** `send_dues_reminders` SHALL send one "dues due soon" reminder with no card-invalid callout and SHALL count it as sent

#### Scenario: Auto-renew monthly member with a valid card is skipped and stays eligible

- **WHEN** an auto-renew member on a monthly membership type has a default card valid at `dues_paid_until` and is within the window
- **THEN** `send_dues_reminders` SHALL send no email AND SHALL leave `dues_reminder_sent_at` unset for that member

#### Scenario: Auto-renew yearly member with a valid card gets a renewal notice

- **WHEN** an auto-renew member on a yearly membership type has a default card valid at `dues_paid_until` and is within the window
- **THEN** `send_dues_reminders` SHALL send a renewal-notice email (distinct from the "dues due soon" reminder) and SHALL count it as sent

#### Scenario: Auto-renew member with an invalid or missing card gets a reminder with a card-invalid callout

- **WHEN** an auto-renew member is within the window and has no default card, or a default card not valid at `dues_paid_until`
- **THEN** `send_dues_reminders` SHALL send the "dues due soon" reminder including the card-invalid callout and SHALL count it as sent

#### Scenario: Lifetime member in the window is skipped

- **WHEN** a member on a lifetime-period membership type lands in the reminder window
- **THEN** `send_dues_reminders` SHALL send no email for that member

#### Scenario: Reminders are idempotent within a cycle

- **WHEN** `send_dues_reminders` runs twice in succession without the reminder flag being reset, for a member who was reminded on the first run
- **THEN** the second run SHALL send no further email to that member (the `dues_reminder_sent_at IS NULL` filter excludes them)
