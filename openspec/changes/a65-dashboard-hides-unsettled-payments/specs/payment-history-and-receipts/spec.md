# payment-history-and-receipts Specification Delta

## MODIFIED Requirements

### Requirement: The member payment history lists only settled payments

The member-facing payment history SHALL list only settled payments — `Completed`
and `Refunded` — and SHALL omit `Pending` and `Failed` from the member's view.
This applies to **every member-facing surface that lists payments**, including
`GET /portal/payments` and its HTMX list fragment and the dashboard's recent-payments
fragment, and keeps abandoned-checkout and transient-failure rows out of the
member's history.
Admin-facing payment views SHALL be unaffected: an admin viewing a member's
payments SHALL still see every status, including `Pending` and `Failed`.
Per-payment receipts and the annual dues statement are unchanged (both already
reflect settled payments).

The rule is a property of the member's view, not of a route. Naming one page left
the dashboard fragment outside it, and an implementation that filtered exactly
where the requirement pointed produced a portal whose two payment lists disagreed:
an abandoned checkout appeared on the dashboard and vanished on the page it linked
to. A member reading their own history is checking whether they were charged, and
two answers to that question is worse than either.

Where a surface limits how many payments it shows, the filter SHALL be applied
**before** the limit. Truncating first and filtering after would show a member
with several unsettled rows an empty list while settled payments sat just outside
the window — a different defect wearing this one's fix.

The decision SHALL have one implementation shared by every surface. The defect
this requirement corrects is a surface that did not use the existing shared
predicate, so a second copy of that predicate SHALL NOT be introduced.

#### Scenario: An abandoned checkout is hidden from the member

- **WHEN** a member cancels a Stripe checkout, leaving a `Pending`-then-`Failed`
  payment row
- **THEN** that row SHALL NOT appear in the member's Payments history

#### Scenario: An abandoned checkout is hidden on the dashboard too

- **WHEN** that same member views the dashboard's recent-payments fragment
- **THEN** the row SHALL NOT appear there either, and the two surfaces SHALL agree

#### Scenario: A limited list filters before it truncates

- **WHEN** a member's most recent payments are unsettled and older settled
  payments exist, on a surface that shows a fixed number
- **THEN** the settled payments SHALL be shown, rather than an empty list

#### Scenario: Completed and refunded payments are shown

- **WHEN** a member has `Completed` and `Refunded` payments
- **THEN** both SHALL appear in the member's Payments history

#### Scenario: Admins still see all statuses

- **WHEN** an admin views a member's payments
- **THEN** `Pending` and `Failed` payments SHALL still be visible in the admin view
