# Tasks

## 1. Dependabot configuration

- [x] 1.1 Add a `cargo` ecosystem entry to `.github/dependabot.yml` alongside the
  existing `github-actions` one, on a weekly schedule.
- [x] 1.2 Group routine updates — patch and minor — into a single pull request
  per run. Leave major updates ungrouped so they are reviewed on their own.
- [x] 1.3 Keep the existing `github-actions` entry exactly as it is, including its
  comment explaining why SHA pins need it. That configuration works; this change
  extends the same treatment, it does not revisit it.
- [x] 1.4 Comment the new entry with the reason, matching the existing one's
  style: the unconfigured ecosystem accumulated six-week-old conflicted PRs while
  the configured one stayed current.

## 2. Advisory check in CI

- [x] 2.1 Add an advisory check to `.github/workflows/ci.yml` that consults the
  RustSec database and fails the build on a known advisory.
- [x] 2.2 Run it against the lockfile so transitive dependencies are covered.
  Directly-declared dependencies are the minority of the exposure.
- [x] 2.3 Run it as its own job rather than a step inside the existing `test`
  job, so an advisory failure is distinguishable at a glance from a test failure.
  They call for different responses.
- [x] 2.4 Make it run on schedule as well as on push, so a newly published
  advisory against unchanged code produces a failure. An advisory is news about
  code already shipped; a check that only fires on dependency changes is silent
  exactly when it matters.
- [x] 2.5 Cache the advisory database consistently with how the workflow already
  caches Rust artifacts, so the new job does not materially lengthen CI.

## 3. Boundaries

- [x] 3.1 Do not enable auto-merge for dependency updates.
- [x] 3.2 Do not add an ignore list or advisory allowlist as part of this change.
  If a specific advisory needs to be waived later, that waiver should be a
  deliberate decision with its reasoning recorded, not a pre-built mechanism
  waiting to be filled in.
- [x] 3.3 Do not modify `deploy.yml` or `release.yml`.

## 4. Verification

- [x] 4.1 Confirm the advisory job fails when a dependency with a known advisory
  is present — verify against a deliberately vulnerable pin in a scratch branch,
  and do not merge that branch.
- [x] 4.2 Confirm it passes on the current tree, or, if it does not, report what
  it found rather than adjusting the check to make it pass. A first run that
  fails is the check working.
- [x] 4.3 Confirm the grouped cargo configuration produces one pull request for
  multiple routine updates rather than one each.

## Verification results

4.1 — On a scratch branch (`scratch/a61-audit-verification`, deleted, never
pushed) with `time = "=0.1.45"` pinned into the root manifest, `cargo audit`
went from 7 vulnerabilities to 8, named `RUSTSEC-2020-0071 time 0.1.45`, and
exited 1.

4.2 — **The check does not pass on the current tree.** Per the task, this is
reported rather than worked around; no ignore list or allowlist was added, so
the advisory job will be red on merge. `cargo audit` (v0.21, RustSec db of
2026-08-13) reports 7 vulnerabilities and 11 warnings:

| Advisory | Crate | Patched |
| --- | --- | --- |
| RUSTSEC-2026-0213 | ammonia 4.1.3 | >=4.1.4 |
| RUSTSEC-2026-0141 | lettre 0.11.21 | >=0.11.22 |
| RUSTSEC-2026-0185 | quinn-proto 0.11.14 | >=0.11.15 |
| RUSTSEC-2023-0071 | rsa 0.9.10 | none available |
| RUSTSEC-2026-0104 | rustls-webpki 0.101.7 | >=0.103.13 |
| RUSTSEC-2026-0098 | rustls-webpki 0.101.7 | >=0.103.12 |
| RUSTSEC-2026-0099 | rustls-webpki 0.101.7 | >=0.103.12 |

Five of the seven — `rustls-webpki` (×3), `rsa`, `quinn-proto` — are declared in
no manifest in the workspace, which is the transitive-coverage scenario holding
in practice rather than in principle. Resolving them is dependency work outside
this change, which only establishes the signal.

4.3 — Verified statically: `.github/dependabot.yml` validates against
schemastore's `dependabot-2.0.json`, whose `groups.*` object has
`additionalProperties: false` and an `update-types` enum of exactly
`major | minor | patch`, so a mistyped key or level would have been rejected.
The grouped-versus-single behaviour is the schema's own stated semantics: the
sibling `exclude-patterns` is documented as "if a dependency is excluded from a
group, Dependabot will continue to raise single pull requests" — group members
batch, non-members do not, so `minor` + `patch` batch and `major` stays
per-crate. Observing a live grouped PR is GitHub-side and will first be visible
on the weekly run after merge.
