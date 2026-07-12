# admin-member-quick-actions

## Why

The member page's Quick Actions card renders "Send Password Reset" and
"Delete Member" buttons targeting `POST /portal/admin/members/:id/reset-password`
and `DELETE /portal/admin/members/:id` — neither route exists, no
handler exists, and `MemberRepository` has no delete method. Both
buttons 404 into a red error (observed in production). Same class of
defect as the ADMIN-notes fiction: scaffolded UI with no backend.

Both actions are real needs: admins send resets on behalf of members
who ask for help, and delete cleans up test/spam/typo signups (which
pay-at-signup makes more common — abandoned checkouts leave Pending
members behind).

## What Changes

- **Send password reset**: `POST /portal/admin/members/:id/reset-password`
  issues the SAME one-hour token + email the self-service
  forgot-password flow sends (shared `create_password_reset_token` +
  `email::templates::{ResetHtml,ResetText}`), audited as
  `send_password_reset`. Routed through `MemberService` per the
  member-admin-service contract.
- **Delete member (guarded)**: `DELETE /portal/admin/members/:id` hard
  deletes a member ONLY when they have no payment rows — payments are a
  ledger; members with financial history get a clear rejection pointing
  at suspend/expire instead. Additional guards: an admin cannot delete
  themselves, and cannot delete the last administrator. Dependent rows
  (profile, sessions, saved cards, scheduled payments, tokens, event
  attendance) go via existing `ON DELETE CASCADE`; a residual FK
  violation (member authored events/announcements or audit entries)
  maps to a friendly rejection rather than a 500. Deletion is audited
  as `delete_member` with the member's identifying info in the entry.
  On success the page redirects to the members list.
- Quick Action buttons get an inline result target so errors render as
  the standard fragment instead of a raw red toast.

## Impact

- Spec: `admin-members` — 2 ADDED requirements.
- Code: route registrations, `admin/members/quick_actions.rs` handlers,
  `member_service/admin_actions.rs` (service methods + guards + audit),
  `MemberRepository::delete` + count helper, member_detail template.
- Tests: service-level (reset audited; delete happy path, self/
  last-admin/has-payments guards); regenerated member-detail goldens.
