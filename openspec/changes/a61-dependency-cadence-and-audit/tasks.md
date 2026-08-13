# Tasks

## 1. Dependabot configuration

- [ ] 1.1 Add a `cargo` ecosystem entry to `.github/dependabot.yml` alongside the
  existing `github-actions` one, on a weekly schedule.
- [ ] 1.2 Group routine updates — patch and minor — into a single pull request
  per run. Leave major updates ungrouped so they are reviewed on their own.
- [ ] 1.3 Keep the existing `github-actions` entry exactly as it is, including its
  comment explaining why SHA pins need it. That configuration works; this change
  extends the same treatment, it does not revisit it.
- [ ] 1.4 Comment the new entry with the reason, matching the existing one's
  style: the unconfigured ecosystem accumulated six-week-old conflicted PRs while
  the configured one stayed current.

## 2. Advisory check in CI

- [ ] 2.1 Add an advisory check to `.github/workflows/ci.yml` that consults the
  RustSec database and fails the build on a known advisory.
- [ ] 2.2 Run it against the lockfile so transitive dependencies are covered.
  Directly-declared dependencies are the minority of the exposure.
- [ ] 2.3 Run it as its own job rather than a step inside the existing `test`
  job, so an advisory failure is distinguishable at a glance from a test failure.
  They call for different responses.
- [ ] 2.4 Make it run on schedule as well as on push, so a newly published
  advisory against unchanged code produces a failure. An advisory is news about
  code already shipped; a check that only fires on dependency changes is silent
  exactly when it matters.
- [ ] 2.5 Cache the advisory database consistently with how the workflow already
  caches Rust artifacts, so the new job does not materially lengthen CI.

## 3. Boundaries

- [ ] 3.1 Do not enable auto-merge for dependency updates.
- [ ] 3.2 Do not add an ignore list or advisory allowlist as part of this change.
  If a specific advisory needs to be waived later, that waiver should be a
  deliberate decision with its reasoning recorded, not a pre-built mechanism
  waiting to be filled in.
- [ ] 3.3 Do not modify `deploy.yml` or `release.yml`.

## 4. Verification

- [ ] 4.1 Confirm the advisory job fails when a dependency with a known advisory
  is present — verify against a deliberately vulnerable pin in a scratch branch,
  and do not merge that branch.
- [ ] 4.2 Confirm it passes on the current tree, or, if it does not, report what
  it found rather than adjusting the check to make it pass. A first run that
  fails is the check working.
- [ ] 4.3 Confirm the grouped cargo configuration produces one pull request for
  multiple routine updates rather than one each.
