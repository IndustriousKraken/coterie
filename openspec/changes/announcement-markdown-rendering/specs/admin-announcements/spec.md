# admin-announcements Specification

## MODIFIED Requirements

### Requirement: User-supplied announcement content is escaped on render

Announcement content SHALL be authored in Markdown and rendered for display
through the shared sanitized Markdown pipeline (see the `announcement-markdown`
capability), NOT emitted as raw HTML. The pipeline SHALL neutralize
raw/embedded HTML — an admin who types raw HTML SHALL NOT have it emitted as
live markup — and SHALL restrict output to a sanitized safe subset with safe
link schemes, so a stored XSS via announcements is prevented. The raw Markdown
SHALL remain the stored source of truth, and rendering SHALL happen at read time
on every display surface (admin views, member portal, and public feeds).

#### Scenario: Script tag in body is escaped

- **WHEN** an admin saves an announcement whose body contains `<script>alert(1)</script>`
- **THEN** rendered pages SHALL display the literal text, not execute the script

#### Scenario: Markdown syntax renders as a safe subset

- **WHEN** an admin saves an announcement whose body contains `*italic*` and `~~struck~~`
- **THEN** rendered pages SHALL show emphasized and struck-through text (for
  example `<em>` and `<del>`), not the literal Markdown punctuation

#### Scenario: A dangerous link scheme is neutralized

- **WHEN** an admin saves body text `[click](javascript:alert(1))`
- **THEN** the rendered output SHALL NOT produce a live `javascript:` link
