## ADDED Requirements

### Requirement: Admin members page links to the CSV export

The admin members page (`/portal/admin/members`) SHALL include a visible "Download CSV" link that points at `/portal/admin/members/export`. The link SHALL preserve the current filter query string (e.g., if the page is filtered to `?status=Active`, the link points at `/portal/admin/members/export?status=Active`).

#### Scenario: Filter state is preserved in the export link

- **WHEN** an admin visits `/portal/admin/members?status=Expired&type=annual`
- **THEN** the page renders a "Download CSV" link with `href="/portal/admin/members/export?status=Expired&type=annual"`

#### Scenario: Link is admin-only (lives on an admin-only page)

- **WHEN** a non-admin somehow reaches the link
- **THEN** the export endpoint itself rejects the request via `require_admin_redirect`
