# Tasks

## 1. Repository

- [x] 1.1 `MemberRepository::delete(id)` (DELETE, NotFound on zero rows)
  and `count_payments_for_member` guard support (service-side query is
  acceptable given `MemberService` holds the pool).

## 2. Service

- [x] 2.1 `member_service/admin_actions.rs`:
  `send_password_reset(actor_id, member_id)` — shared token helper +
  `email::templates` reset email + audit.
- [x] 2.2 `delete(actor_id, member_id)` — guards (self, last admin,
  payments present), delete, `delete_member` audit, friendly mapping of
  residual FK violations.

## 3. Handlers + routes + template

- [x] 3.1 `admin/members/quick_actions.rs`: POST reset-password →
  admin_alert fragment; DELETE member → HX-Redirect to the members list
  on success, fragment on error.
- [x] 3.2 Register both routes; point the Quick Action buttons at an
  inline result div.

## 4. Tests

- [x] 4.1 Service: reset creates a token + audit entry.
- [x] 4.2 Service: delete happy path removes the member and audits;
  payments guard, self guard, last-admin guard each reject.
- [x] 4.3 Regenerate member-detail template goldens.
