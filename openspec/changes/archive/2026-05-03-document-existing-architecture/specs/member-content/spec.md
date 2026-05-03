## ADDED Requirements

### Requirement: Members see all events and announcements (public + members-only)

The member portal SHALL provide:
- `GET /portal/events` — events page.
- `GET /portal/announcements` — announcements page.
- `GET /portal/api/events/list` — HTMX events list fragment.
- `GET /portal/api/announcements/list` — HTMX announcements list fragment.

Members SHALL see both public and members-only content; the public/private flag affects only the `/public/*` surface.

#### Scenario: Members-only event is visible inside the portal

- **WHEN** an authenticated member views the events page
- **THEN** members-only events SHALL appear alongside public ones

### Requirement: Members can RSVP to events

`POST /portal/api/events/:id/rsvp` and `POST /portal/api/events/:id/cancel` SHALL allow Active/Honorary members to manage their RSVP. The handlers SHALL call `event_repo.register_attendance` / `cancel_attendance` and return an updated HTMX button fragment.

#### Scenario: RSVP is CSRF-protected

- **WHEN** an HTMX RSVP request arrives without `X-CSRF-Token`
- **THEN** the top-level CSRF layer SHALL reject it with 403

#### Scenario: RSVP changes are NOT currently audited

- **WHEN** a member RSVPs or cancels their RSVP
- **THEN** no `audit_logs` row SHALL be written today; this is observed behavior. (Whether to audit RSVP transitions is a policy question for a follow-up change; today's spec captures truth.)
