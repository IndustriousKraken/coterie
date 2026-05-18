## ADDED Requirements

### Requirement: Admin members page links to the bulk import flow

The admin members page (`/portal/admin/members`) SHALL include a visible "Bulk import" button or link that navigates to `/portal/admin/members/import`. The import page renders a form with a file input and a brief format reminder listing the required and optional columns.

#### Scenario: Bulk-import entry point is reachable from the members page

- **WHEN** an admin visits `/portal/admin/members`
- **THEN** the page SHALL render a "Bulk import" affordance alongside the existing "New Member" affordance

#### Scenario: Format reminder lists required and optional columns

- **WHEN** an admin visits `/portal/admin/members/import`
- **THEN** the page SHALL display the required columns (`email`, `username`, `full_name`, `membership_type_slug`) and the optional ones (`status`, `notes`, `discord_id`) clearly enough that a first-time user knows what to put in their CSV
