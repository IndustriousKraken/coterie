# admin-members Specification

## ADDED Requirements

### Requirement: Admin can send a member a password reset

`POST /portal/admin/members/:id/reset-password` SHALL issue the member the same single-use, time-limited reset token and email that the self-service forgot-password flow issues, routed through `MemberService` and audited as `send_password_reset`. The action SHALL NOT reveal or change the member's password and SHALL NOT alter their status or sessions.

#### Scenario: Reset email goes out from the member page

- **WHEN** an admin triggers Send Password Reset on a member
- **THEN** a password-reset token SHALL be created for that member, the reset email SHALL be sent to their address, and a `send_password_reset` audit entry SHALL be written

### Requirement: Admin can delete a member without financial history

`DELETE /portal/admin/members/:id` SHALL hard-delete the member and their dependent records (profile, sessions, saved cards, scheduled payments, tokens, attendance) ONLY when the member has no payment rows. The deletion SHALL be rejected with guidance (suspend or expire instead) when payment rows exist — payment history is a ledger and never silently disappears. An admin SHALL NOT be able to delete their own account, and the last remaining administrator SHALL NOT be deletable. Deletion SHALL be audited as `delete_member` carrying the member's identifying info, and a successful deletion SHALL navigate the admin back to the members list.

#### Scenario: Test signup is deleted cleanly

- **WHEN** an admin deletes a Pending member who has no payment rows
- **THEN** the member row and their cascade-linked records SHALL be removed, a `delete_member` audit entry SHALL be written, and the admin SHALL land on the members list

#### Scenario: Member with payments is rejected with guidance

- **WHEN** an admin attempts to delete a member who has any payment row
- **THEN** the deletion SHALL be rejected with a message directing them to suspend or expire, and nothing SHALL be removed

#### Scenario: Self-deletion and last-admin deletion are rejected

- **WHEN** an admin attempts to delete their own account, or the only remaining administrator
- **THEN** the deletion SHALL be rejected and nothing SHALL be removed
