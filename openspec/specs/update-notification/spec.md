# update-notification Specification

## Purpose
TBD - created by archiving change a40-update-available-banner. Update Purpose after archive.
## Requirements
### Requirement: The system checks for a newer stable release in the background

The system SHALL check for newer releases only in a background task, never in the
request path, caching the latest stable release for the banner to read. The check
SHALL run on an initial-delay-then-daily schedule and SHALL select the most
recent release whose prerelease flag is false.

#### Scenario: Background check caches the latest stable release

- **WHEN** the background check runs and the GitHub releases list is available
- **THEN** it SHALL cache the latest non-prerelease release (its tag and notes
  URL) for the banner to read, without any release lookup occurring during page
  rendering

#### Scenario: GitHub failure degrades gracefully

- **WHEN** the releases list cannot be fetched or parsed
- **THEN** the cached value SHALL be left unchanged, no error SHALL surface to
  users, and no banner SHALL be shown on account of the failure

### Requirement: Admins see an update-available banner when behind

The portal SHALL show an "update available" banner to admins when the cached
latest stable release is newer than the running release, linking to that
release's notes and to the update instructions. The banner SHALL NOT appear when
the running release is at or ahead of the latest known stable release.

#### Scenario: Newer stable release shows the banner to an admin

- **WHEN** an admin views the portal and the cached latest stable tag is newer
  than the running release tag
- **THEN** a banner SHALL be shown linking to the release notes and the update
  steps

#### Scenario: Up-to-date instance shows no banner

- **WHEN** the running release tag is equal to or newer than the latest known
  stable tag
- **THEN** no update banner SHALL be shown

### Requirement: The banner is scoped to admins and release builds

The banner SHALL be shown only to admin sessions, and only when the running
build has an embedded release tag. Members SHALL never see it, and a
development or untagged build (no embedded release tag) SHALL never trigger it.

#### Scenario: Members never see the banner

- **WHEN** a non-admin member views the portal while a newer release exists
- **THEN** no update banner SHALL be shown to them

#### Scenario: Development builds never trigger the banner

- **WHEN** the running build has no embedded release tag
- **THEN** no update check comparison SHALL flag it as behind and no banner SHALL
  be shown

### Requirement: Release-version comparison is a pure, unit-tested function

Version comparison SHALL be a pure function taking the candidate and running
tags as arguments (so it is unit-testable without build-time state), comparing
`vX.Y.Z` semantics with any leading `v` stripped, and ignoring tags it cannot
parse rather than erroring. Prereleases SHALL be excluded from "latest stable"
selection via the release's prerelease flag.

#### Scenario: Newer semantic version is detected across digit boundaries

- **WHEN** comparing `v1.10.0` (candidate) against `v1.9.0` (running)
- **THEN** the function SHALL report the candidate as newer (not a string compare)

#### Scenario: Unparseable tags are ignored

- **WHEN** a tag cannot be parsed as a version
- **THEN** the comparator SHALL ignore it rather than error, and it SHALL NOT be
  treated as newer

### Requirement: The update check is opt-out via setting

The check SHALL be gated by an `updates.check_enabled` setting that defaults to
enabled; when it is disabled, the background task SHALL perform no GitHub fetch
and the banner SHALL never render. The setting SHALL be editable from the admin
settings UI.

#### Scenario: Disabling the setting stops checks and the banner

- **WHEN** `updates.check_enabled` is set to false
- **THEN** the background task SHALL skip the GitHub fetch and no banner SHALL be
  shown regardless of any previously cached value

### Requirement: The banner is dismissible until a newer version ships

The banner SHALL be dismissible, recording the dismissed release tag, and SHALL
stay hidden while the dismissed tag equals the current latest-known stable tag.
When a release newer than the dismissed one appears, the banner SHALL reappear.

#### Scenario: Dismissed banner stays hidden for that version

- **WHEN** an admin dismisses the banner for latest tag `v1.4.0`
- **THEN** the banner SHALL stay hidden while `v1.4.0` remains the latest known

#### Scenario: A newer release re-shows the banner

- **WHEN** the latest known stable tag advances to `v1.5.0` after `v1.4.0` was
  dismissed
- **THEN** the banner SHALL be shown again

