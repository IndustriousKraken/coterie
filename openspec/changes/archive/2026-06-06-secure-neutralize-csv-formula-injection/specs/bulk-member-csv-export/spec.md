## ADDED Requirements

### Requirement: Exported CSV neutralizes spreadsheet formula injection

The member roster export SHALL neutralize spreadsheet formula injection (CWE-1236) in its member-controlled free-text columns (`email`, `username`, `full_name`, `discord_id`, `notes`), which originate from the unauthenticated `POST /public/signup` endpoint and other member-editable surfaces with no character restrictions.

Specifically, when one of these field values begins with a formula-trigger character (`=`, `+`, `-`, `@`) or with a control character a spreadsheet may treat as starting a formula (TAB, CR, LF), the CSV writer SHALL prefix the value with a single quote (`'`) inside the RFC 4180 double-quoting so spreadsheet applications render the cell as literal text rather than evaluating it.

Numeric, timestamp, enum, and boolean columns (e.g. `id`, `status`, `joined_at`, `dues_paid_until`, `is_admin`, `bypass_dues`, `email_verified_at`) are not member-controlled and SHALL NOT be altered by this neutralization.

#### Scenario: Formula-leading member name is neutralized

- **WHEN** a member's `full_name` is `=HYPERLINK("http://evil.example","click")` and an admin exports the roster
- **THEN** the corresponding CSV field SHALL begin with a single quote immediately after the opening double-quote (e.g. `"'=HYPERLINK(...)"`) so a spreadsheet opens it as text, not a formula

#### Scenario: Ordinary values are not prefixed

- **WHEN** a member's `full_name` is `O'Brien, Sean` (no leading formula trigger)
- **THEN** the CSV field SHALL be quoted per RFC 4180 with no single quote injected after the opening double-quote, and the internal apostrophe SHALL be preserved unchanged
