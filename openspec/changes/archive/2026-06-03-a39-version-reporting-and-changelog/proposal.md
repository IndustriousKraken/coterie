## Why

Docs and the changelog on `master` always describe the latest version, but an
operator may be running an older tagged release — so they see features and
instructions that don't exist in their build. The fix isn't to keep docs in
lock-step (impossible); it's to **always give the running instance a correct
anchor to its own version**, and to make the change history version-segmented so
"ahead" reads as an upgrade prompt rather than confusion.

Two concrete gaps block that today:

1. The binary reports `env!("CARGO_PKG_VERSION")` (`0.1.0`) at `/health` and
   `/api` (`src/api/handlers/root.rs`) — it does **not** know its release tag at
   runtime, even though the release tarball bundles a `VERSION` file (tag + SHA)
   that nothing reads back. So no one can reliably tell what version is running.
2. There is no checked-in `CHANGELOG.md`. Release notes are GitHub's raw
   auto-generated PR list (`generate_release_notes: true`), which is weaker than
   the autocoder's generator (archived specs summarized and impact-ranked by an
   LLM).

## What Changes

- **Build-time version embedding.** Add a `version` module exposing a pure
  `display_version(...)` function plus values read via
  `option_env!("COTERIE_VERSION")` / `option_env!("COTERIE_GIT_SHA")`. Release
  builds embed the real tag + short SHA; a small `build.rs` stamps the commit SHA
  for untagged (local/staging) builds when the env hasn't set one, so dev builds
  read `0.1.0-dev (abc1234)` rather than a bare `-dev`. `build.rs` never invents
  a tag.
- **Report the real version.** `/health` and `/api` report the embedded version
  instead of `CARGO_PKG_VERSION`.
- **Surface it in the portal** footer as an "about this version" affordance,
  linked to its release page (`releases/tag/<tag>`) when a real release tag is
  embedded; dev builds show a plain `-dev` string with no link.
- **Version-pinned documentation links in the portal.** A `docs_url(path)`
  helper builds repo-doc links against the most specific ref for the build —
  release tag, else commit SHA, else `master` — so a linked doc matches the
  running build (and, since a tag/SHA resolves regardless of branch, this covers
  docs that only exist on a not-yet-merged tagged branch). The Stripe/billing
  settings page links to its setup guide via that helper; config pages with no
  dedicated guide get no link (no fabricated targets).
- **`CHANGELOG.md`** in Keep a Changelog format: an `[Unreleased]` section plus
  one immutable, dated, version-headed section per release. This is the target
  the autocoder's changelog generator writes into and a human can hand-edit.
- **Release body sourced from the changelog.** The release workflow publishes
  the `CHANGELOG.md` section matching the tag as the release body (GitHub's
  auto-notes become a fallback / appended detail, not the primary content).
- **README development banner** noting the docs track the latest version and
  routing older-version readers to their tag / release.

## Capabilities

### New Capabilities
- `release-versioning`: a deployed instance and its users can always identify
  the running version and reach the version-accurate change history — covers
  build-time version embedding, in-app version reporting + release linking, the
  `CHANGELOG.md` format contract, the changelog-sourced release body, and the
  docs self-identifying as tracking the development version.

### Modified Capabilities
<!-- None. `/health` and `/api` change which value they report for an existing
     field, not the shape of any specified requirement. -->

## Impact

- **Code**: new `src/version.rs` (pure `display_version` + `reference` +
  `docs_url` + `option_env!` values) and a new `build.rs` (git-SHA stamping);
  `src/api/handlers/root.rs` (`/health`, `/api` use it);
  portal base template + its render context (`BaseContext`) for the
  footer/version line; `templates/admin/billing_settings.html` gains a
  version-pinned Stripe setup-guide link.
- **CI**: `.github/workflows/release.yml` — pass `COTERIE_VERSION` +
  `COTERIE_GIT_SHA` to the main `cargo build`, and set the release body from the
  tag's `CHANGELOG.md` section.
- **Docs**: new `CHANGELOG.md`; README dev banner.
- **Tests**: unit tests for `display_version`; an integration assertion that
  `/health` reports the embedded value.
- **No change** to `coterie-provision` version reporting (out of scope) and no
  change to the database, auth, or admin surfaces.
