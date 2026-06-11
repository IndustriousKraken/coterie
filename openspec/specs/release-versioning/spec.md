# release-versioning Specification

## Purpose
TBD - created by archiving change a39-version-reporting-and-changelog. Update Purpose after archive.
## Requirements
### Requirement: The running binary reports its embedded release version

The binary SHALL report the version embedded at build time — not the hardcoded
crate version — at both `GET /health` and `GET /api`. Release builds embed the
release tag and short commit SHA via the `COTERIE_VERSION` and `COTERIE_GIT_SHA`
build-time environment variables; when those are absent (development builds),
the reported value falls back to the crate version with a `-dev` marker.

#### Scenario: Release build reports tag and SHA

- **WHEN** the binary is built with `COTERIE_VERSION` and `COTERIE_GIT_SHA` set
- **THEN** `GET /health` and `GET /api` SHALL report a version derived from the
  tag and short SHA (e.g. `v1.2.3 (abc1234)`)

#### Scenario: Development build reports a dev marker

- **WHEN** the binary is built without a release tag
- **THEN** the reported version SHALL be the crate version suffixed with `-dev`,
  including the short commit SHA when build-time git info is available (e.g.
  `0.1.0-dev (abc1234)`), and SHALL NOT claim a release tag

### Requirement: Version derivation is a pure, unit-tested function

Version-string derivation SHALL be a pure function whose inputs (the optional
tag, the optional SHA, and the crate version) are all passed as arguments, so it
is fully unit-testable without manipulating build-time environment variables.

#### Scenario: All derivation branches are unit-tested

- **WHEN** the version function is tested
- **THEN** there SHALL be unit tests covering tag-plus-SHA, tag-without-SHA, and
  the no-tag dev fallback, asserting the exact rendered string for each

### Requirement: The portal surfaces the running version and links to its release

The portal SHALL display the running version to viewers, and SHALL link it to
that version's release page (`releases/tag/<tag>`) when a release tag is embedded.
For development builds (no embedded tag) the portal SHALL show the `-dev` version
string with no release link, since no release page exists for an untagged build.

#### Scenario: Release build links to the release page

- **WHEN** the portal renders for a build with an embedded release tag
- **THEN** the displayed version SHALL link to that tag's GitHub release page

#### Scenario: Dev build shows version without a link

- **WHEN** the portal renders for a development build
- **THEN** it SHALL show the `-dev` version string and SHALL NOT render a release
  link

### Requirement: Portal documentation links are pinned to the running version

The portal SHALL build documentation links against the most specific Git ref it
can identify for the running build — the release tag when one is embedded,
otherwise the build's commit SHA, otherwise the default branch (`master`) — so a
linked document matches the running build rather than always showing the latest.
Because a Git tag or SHA resolves on GitHub independently of which branch it
lives on, this also makes docs that exist only on a not-yet-merged branch
reachable from a build tagged or stamped on that branch.

#### Scenario: Release build pins doc links to the tag

- **WHEN** a documentation link is rendered for a build with an embedded release
  tag
- **THEN** its URL SHALL reference `blob/<tag>/<path>` for that tag

#### Scenario: Untagged build pins doc links to the commit SHA

- **WHEN** a documentation link is rendered for a build with no release tag but a
  known commit SHA (e.g. a staging or local build)
- **THEN** its URL SHALL reference `blob/<sha>/<path>` for that commit

#### Scenario: Build with no tag or SHA falls back to the default branch

- **WHEN** a documentation link is rendered for a build with neither a tag nor a
  known commit SHA
- **THEN** its URL SHALL reference `blob/master/<path>`

### Requirement: Admin configuration pages link to their operator guide

An admin configuration page SHALL link to its feature's dedicated operator setup
guide using the version-pinned documentation URL where such a guide exists; the
Stripe/billing settings page SHALL link to `docs/deploy/STRIPE-SETUP.md`, and a
configuration page whose feature has no dedicated guide SHALL NOT render a
fabricated documentation link.

#### Scenario: Stripe settings page links to its guide

- **WHEN** an admin views the Stripe/billing settings page
- **THEN** the page SHALL show a version-pinned link to `docs/deploy/STRIPE-SETUP.md`

#### Scenario: No guide means no link

- **WHEN** an admin views a configuration page whose feature has no dedicated
  operator guide (e.g. Discord, UniFi)
- **THEN** the page SHALL NOT render a documentation link rather than point at a
  nonexistent document

### Requirement: The project maintains a versioned changelog

The repository SHALL contain a `CHANGELOG.md` in Keep a Changelog format: a
top `## [Unreleased]` section followed by dated, version-headed sections
(`## [vX.Y.Z] — YYYY-MM-DD`) ordered newest-first. Each version section SHALL be
treated as immutable once its tag is published, so a reader on any version sees
their section plus clearly-labeled newer sections.

#### Scenario: Changelog has the required structure

- **WHEN** `CHANGELOG.md` is present
- **THEN** it SHALL contain an `[Unreleased]` section and use
  `## [vX.Y.Z] — YYYY-MM-DD` headers for released versions, newest-first

### Requirement: Releases publish the changelog section for the tag

The release workflow SHALL set the GitHub Release body from the `CHANGELOG.md`
section whose header matches the tag being released, rather than relying on
GitHub's auto-generated notes as the primary content. If no matching section is
found, the workflow SHALL fall back to generated notes so the release body is
never empty.

#### Scenario: Release body comes from the changelog

- **WHEN** a tag with a matching `CHANGELOG.md` section is released
- **THEN** the release body SHALL be that section's content

#### Scenario: Missing changelog section falls back

- **WHEN** a released tag has no matching `CHANGELOG.md` section
- **THEN** the workflow SHALL fall back to generated release notes rather than
  publishing an empty body

### Requirement: Docs disclose that they track the development version

The `README.md` SHALL state that its contents track the latest development
version and SHALL point readers on an older release to that release's notes (or
to browsing the repository at their tag) for version-accurate documentation.

#### Scenario: README carries a development-version notice

- **WHEN** a reader opens `README.md` on the default branch
- **THEN** a near-the-top notice SHALL tell them the docs track the development
  version and how to find docs matching an older installed release

