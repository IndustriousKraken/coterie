# Tasks

## 1. The dashboard fragment

- [ ] 1.1 `src/web/portal/dashboard.rs::recent_payments` — filter through the
  existing `crate::web::portal::partials::is_member_visible`. Do not write a
  second predicate; the defect is a surface that did not use the shared one.
- [ ] 1.2 Filter **before** `truncate(5)`. The current order truncates first, so
  filtering after it would give a member with five abandoned checkouts an empty
  dashboard while settled payments sit just outside the window — the same class
  of bug with a new shape.
- [ ] 1.3 Replace the comment that says "regardless of status", which currently
  documents the defect as if it were the intent.

## 2. Prevent the next surface from missing it

- [ ] 2.1 Look for any other member-facing surface that lists payments and does
  not route through the predicate. Two were expected and one was missed; the
  question is whether there is a third.
- [ ] 2.2 Prefer making the filtered read the easy path — a repository or helper
  call that returns member-visible payments — over each call site remembering to
  filter. A rule enforced by every caller is a rule that will be missed again.
- [ ] 2.3 Leave admin surfaces alone. They must keep showing every status; that
  is how an abandoned checkout gets diagnosed.

## 3. Tests

- [ ] 3.1 A member with a `Failed` payment sees it on neither the dashboard
  fragment nor the payments list.
- [ ] 3.2 The two surfaces agree: for a fixture mixing `Completed`, `Refunded`,
  `Pending`, and `Failed`, the set the dashboard shows is a prefix of what the
  payments list shows.
- [ ] 3.3 Ordering guard for 1.2: a member whose five most recent payments are all
  unsettled, with settled payments older than those, still sees the settled ones
  on the dashboard. Written so it fails if filtering moves after truncation.
- [ ] 3.4 An admin viewing the same member still sees every status.
- [ ] 3.5 Receipts and the dues statement are unchanged.

## 4. Close the loop

- [ ] 4.1 The originating report is `IndustriousKraken/coterie#120`, reopened by a
  comment noting the dashboard still shows what the Payments page hides. Confirm
  the reported reproduction — cancel a Stripe checkout, then compare dashboard and
  Payments — no longer differs between the two surfaces.
