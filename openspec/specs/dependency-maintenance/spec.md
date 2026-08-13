# dependency-maintenance Specification

## Purpose
TBD - created by archiving change a61-dependency-cadence-and-audit. Update Purpose after archive.
## Requirements
### Requirement: Every dependency ecosystem the project uses has an update cadence

The project SHALL configure scheduled dependency updates for every ecosystem it
depends on, not only for its CI actions.

Configuring one ecosystem and not another produces a predictable asymmetry: the
configured one stays current and the unconfigured one accumulates. On this
repository the GitHub Actions ecosystem, which is configured, carried four clean
and current update PRs while the Rust ecosystem, which was not, carried PRs six
weeks old and conflicted. The unconfigured ecosystem is the one the application
actually links.

Updates SHALL be grouped so that a period's routine bumps arrive as a single
review rather than one pull request per package. Ungrouped updates on a project
that reviews in batches produce a queue that is never empty and therefore never
triaged, which is the state that let security-relevant updates sit beside
cosmetic ones indistinguishably.

Updates that change a major version SHALL NOT be grouped with routine ones. A
major bump can carry breaking API changes and needs to be read on its own.

#### Scenario: The Rust ecosystem is scheduled

- **WHEN** a new version of a crate the project depends on is published
- **THEN** a scheduled update SHALL be raised for it, without requiring a
  security advisory to exist

#### Scenario: Routine updates arrive grouped

- **WHEN** several routine updates are available in the same period
- **THEN** they SHALL be proposed together rather than as one pull request each

#### Scenario: A major bump is not grouped

- **WHEN** an available update changes a major version
- **THEN** it SHALL be proposed separately from the routine group

### Requirement: CI fails on a known-vulnerable dependency

Continuous integration SHALL consult a vulnerability advisory database on every
build and SHALL fail when a dependency with a known advisory is present.

Without this, a vulnerable dependency is discovered only if an automated update
happens to be raised for it, and a reviewer looking at that pull request has
nothing telling them it matters more than a routine bump. Detection SHALL NOT
depend on a human correctly guessing which of several open dependency updates is
the security one.

The check SHALL be evaluated against the resolved dependency graph — the
lockfile — and not only against directly declared dependencies. Most exposure
comes from transitive dependencies: of the Rust update PRs open on this
repository when this was written, two were for crates that appear nowhere in the
project's manifest.

A build SHALL fail on a newly published advisory affecting existing code, even
when nothing in that build changed. An advisory is news about code already
shipped, and a check that only ran when dependencies changed would stay silent
precisely when it matters.

#### Scenario: A vulnerable dependency fails the build

- **WHEN** a dependency in the resolved graph has a known advisory
- **THEN** the build SHALL fail and name the affected dependency

#### Scenario: A transitive dependency is covered

- **WHEN** the vulnerable dependency is not declared in the project's manifest
  but appears in the resolved graph
- **THEN** the check SHALL still fail the build

#### Scenario: A new advisory fails an unchanged build

- **WHEN** an advisory is published against a dependency and the project is built
  again with no change of its own
- **THEN** that build SHALL fail

