# Change: Rust dependencies get an update cadence and an advisory check

## Why

`.github/dependabot.yml` declares one ecosystem: `github-actions`. There is no
`cargo` entry, so **nothing schedules Rust dependency updates.** The only cargo
PRs that appear are the ones Dependabot's security alerts raise on their own.

The effect is visible on the repository right now. Four cargo PRs are open; the
two oldest were raised on 2026-06-27 and have sat for six weeks, both now
conflicted with no CI run, because nothing gives them a cadence and nothing keeps
them rebased. The four GitHub Actions PRs — the ecosystem that *is* configured —
are all current, clean, and green. The configured half works; the unconfigured
half rots.

Worse, no workflow runs an advisory check. `ci.yml`, `deploy.yml`, and
`release.yml` contain no `cargo audit`, no `cargo-deny`, nothing that consults
the RustSec database. So a vulnerable transitive crate is detected only if
Dependabot happens to open a PR for it, and when one appears there is no signal
distinguishing "this fixes an advisory" from "this is a routine patch bump." That
is exactly the judgment call that has left security-relevant PRs sitting for
weeks alongside cosmetic ones.

The gap is asymmetric in a way worth naming. The existing `dependabot.yml`
comment explains that Actions are configured because *"without this, SHA pins go
stale silently and miss security patches."* That reasoning applies with at least
equal force to the crates the application actually links.

## What Changes

- **A `cargo` ecosystem entry**, so Rust dependencies get the same scheduled
  attention Actions already get.
- **Grouped updates**, so a week's patch and minor bumps arrive as one pull
  request rather than one per crate. The current state — eight open PRs, six
  weeks stale — is what ungrouped updates produce on a repository that reviews in
  batches. Major bumps stay ungrouped, because those need reading.
- **An advisory check in CI**, consulting the RustSec database on every build, so
  a known-vulnerable dependency fails the build instead of waiting to be noticed.
  This is the part that turns dependency updates from a queue someone has to
  triage into a signal that arrives on its own.
- **The advisory check runs against the lockfile**, so it covers transitive
  dependencies. Most of what an application is exposed to is not in its
  `Cargo.toml`; two of the four open cargo PRs are for crates that appear
  nowhere in it.

## What this does not do

- **It does not enable auto-merge.** Dependency updates land by the same review
  every other change goes through.
- **It does not pin or vendor dependencies.** Whether to vendor is a separate
  question with a much larger answer.
- **It does not change the Actions configuration**, which works.
- **It does not decide the outstanding PRs.** Those are review decisions; this
  changes the conditions that let them accumulate.
