## Context

The skew between `master` docs/changelog and an older installed release is
unavoidable — `master` is always ahead. The workable strategy is not to prevent
skew but to make it harmless:

- **Changelog / "what's new":** being ahead is fine, even useful, as long as it
  is version-segmented and labeled. An append-only, version-headed changelog
  reads as an upgrade prompt ("v1.4.0 added X you don't have"), not confusion.
- **Instructional state (the binary's own version):** must be knowable at
  runtime so an operator/member can always find docs that match their build.

Today the binary can't report its real version (`CARGO_PKG_VERSION` is the
constant `0.1.0`), and there's no curated changelog. This change closes both.
The autocoder already has a superior changelog generator (archived OpenSpec
changes summarized and impact-ranked by an LLM); this change gives that
generator a checked-in target (`CHANGELOG.md`) and wires releases to consume it.

## Goals / Non-Goals

**Goals:**
- The running version is reported at `/health` and `/api`, surfaced in the
  portal, and linked to its release page when it is a real release build.
- Version derivation is a pure, fully unit-tested function with a sane dev
  fallback.
- A `CHANGELOG.md` format contract exists for the generator to write into, and
  the release body is sourced from it.
- The README discloses that it tracks the development version.

**Non-Goals:**
- **Implementing the changelog generator.** That lives in the autocoder. This
  change only defines the `CHANGELOG.md` contract and consumes it at release.
- **A versioned docs site** (mkdocs/Docusaurus). GitHub already serves docs at a
  tag for free; revisit only if install volume makes skew a support burden.
- **Bundling the docs into the release tarball.** Reasonable follow-up, but out
  of scope here to keep the change focused.
- **`coterie-provision` version reporting.** Out of scope; this targets the main
  `coterie` binary's operator/member-facing surfaces.
- **Fabricating a release tag for non-release builds.** `build.rs` stamps a
  commit SHA for dev/staging builds, but never a tag — a tag asserts "a release
  exists," which only the release workflow may claim.

## Decisions

### D1. Version via `option_env!`, derivation as a pure function

`src/version.rs` exposes:

```rust
const RELEASE_TAG: Option<&str> = option_env!("COTERIE_VERSION");
const RELEASE_SHA: Option<&str> = option_env!("COTERIE_GIT_SHA");

/// Pure — all inputs passed in so it is fully unit-testable.
pub fn display_version(tag: Option<&str>, sha: Option<&str>, crate_ver: &str) -> String { ... }
```

- Release build (`COTERIE_VERSION=v1.2.3 COTERIE_GIT_SHA=abc1234`): `"v1.2.3 (abc1234)"`.
- Dev build with a known SHA: `"0.1.0-dev (abc1234)"`.
- Dev build with no git info: `"0.1.0-dev"`.

The public accessor passes the `option_env!` consts into `display_version`. Unit
tests exercise the pure function directly for all branches without needing build
env manipulation.

So that untagged builds (local dev, the staging `deploy.yml`) still embed a real
commit, a small `build.rs` probes `git rev-parse --short HEAD` and emits it as
`COTERIE_GIT_SHA` — but only when the process env hasn't already set it, so the
release workflow's value always wins. `build.rs` is a graceful no-op when git or
`.git` is absent (e.g. a source archive), leaving the value `None`. It does not
fabricate a tag: a tag means "a release exists," which only the release workflow
can assert.

### D2. Release link only when it's a real release

The portal version line links to `https://github.com/IndustriousKraken/coterie/releases/tag/<tag>`
only when `RELEASE_TAG` is `Some`. Dev builds render the `-dev` string with no
link (there is no release page for an untagged build). Whether a release link is
appropriate is therefore decided by "is a tag embedded," not by parsing the
version string.

### D3. `/health` and `/api` report the embedded version

Both handlers replace `env!("CARGO_PKG_VERSION")` with the `version` accessor.
This is a value change to an existing field, not a shape change, so no consumer
contract breaks.

### D4. `CHANGELOG.md` — Keep a Changelog, finalized before the tag

Format: a top `## [Unreleased]` section, then `## [vX.Y.Z] — YYYY-MM-DD`
sections newest-first, each immutable once tagged. Release flow (performed by the
autocoder/maintainer at release prep, not by this change's implementer): write
the new version section, commit, then tag that commit — so the tagged commit
carries its own finalized entry. Only `[Unreleased]` is ever ahead of every tag;
everything under a version header is accurate for that version forever.

### D5. Release body sourced from the changelog

`release.yml`'s `softprops/action-gh-release@v2` step sets `body_path` to a file
containing the `CHANGELOG.md` section whose header matches the tag, extracted by
a small step (e.g. `awk` between the matching `## [<tag>]` header and the next
`## [` header). `generate_release_notes` is kept as the appended detail / fallback
when no matching section is found, so a release never ends up with an empty body.

### D7. Version-pinned documentation links in the portal

Since the repo is public, the portal can link operators straight to the repo
docs — but pinned to the running version, never to `master`, or it reintroduces
the exact skew this change exists to kill. A helper:

```rust
/// Most specific ref we can resolve for the running build.
pub fn reference() -> &'static str {
    RELEASE_TAG.or(RELEASE_SHA).unwrap_or("master")
}

pub fn docs_url(path: &str) -> String {
    format!("https://github.com/IndustriousKraken/coterie/blob/{}/{path}", reference())
}
```

resolves the ref by priority **tag → SHA → master**: a release build pins to its
tag, an untagged build (local/staging) pins to its commit SHA, and only a build
with no git info at all falls back to `master`. Because GitHub resolves a tag or
SHA regardless of which branch it points to, this directly covers the "I tagged
a prerelease in a feature branch before merging to master" case — `blob/<tag>/…`
serves the docs as they exist at that commit, even though they aren't on `master`
yet. The SHA tier extends the same guarantee to untagged staging/local builds.
The footer version is the "about this version" affordance (links to the release
page, which only exists for a real tag). Documentation links are **contextual, not a global nav item** —
they appear on the admin page that configures the feature, since the repo docs
are operator content, not member content. Today the only feature with a
dedicated single-topic guide is Stripe, so `templates/admin/billing_settings.html`
links to `docs/deploy/STRIPE-SETUP.md` via `docs_url`. Discord/UniFi have no dedicated
guide (only scattered coverage inside the deploy walkthroughs), so they get no
link rather than a fabricated dead one — if a dedicated guide is added later,
that page can link to it the same way.

### D6. README development banner

A one-line banner at the top of `README.md`: the docs track the latest
development version; readers on an older release should consult that release's
notes or browse the repo at their tag. This routes people to GitHub's
already-free per-tag docs without standing up any infrastructure.

## Risks / Trade-offs

- **Workflow changes are weakly validatable by the implementer** (no local GHA
  run). Mitigation: the YAML edits are small and the changelog-extraction is a
  self-contained shell step that `bash -n` can syntax-check; behavioral
  confirmation happens on the next real tag (an operator action, not a spec
  task).
- **`option_env!` is compile-time**, so the release build step MUST set the env
  vars or releases silently show `-dev`. Mitigation: the env vars are set on the
  same `cargo build` line that produces the shipped binary, and a follow-up
  could assert the embedded version in a post-build CI check (out of scope).
- **Changelog discipline is process, not code.** The format contract + the
  release wiring are in scope; keeping `[Unreleased]` tidy is a maintainer habit.

## Migration Plan

Single change:

1. Add `src/version.rs` (pure `display_version` + `reference` + `docs_url` +
   `option_env!` accessors); wire it into `lib.rs`. Add a `build.rs` stamping
   `COTERIE_GIT_SHA` from git when the env hasn't already set it (no-op without
   git/`.git`).
2. Update `/health` and `/api` handlers to report it.
3. Add the portal footer/version line + release link (dev fallback), the
   `docs_url` helper (ref priority tag → SHA → master), and the version-pinned
   Stripe setup-guide link on the billing settings page.
4. Add `CHANGELOG.md` scaffold (header + `[Unreleased]`).
5. Edit `release.yml`: pass `COTERIE_VERSION`/`COTERIE_GIT_SHA` to the main
   build; set the release `body_path` from the tag's changelog section.
6. Add the README dev banner.
7. Unit tests for `display_version`; integration assertion for `/health`.
8. `cargo build`, `cargo test`, `cargo clippy --deny warnings`,
   `cargo fmt --check`, `bash -n` any new script, `openspec validate`.
