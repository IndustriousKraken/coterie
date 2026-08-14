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

### Requirement: A dependency is removed rather than waived where it is unused

An advisory against a dependency the application does not use SHALL be resolved
by removing that dependency where removal is possible, in preference to recording
a waiver.

Dependencies arrive unused most often through a package's default feature set
rather than through a deliberate choice. This application declares a SQLite-only
database layer yet links MySQL and PostgreSQL drivers, because the database
crate's defaults were never narrowed — and the MySQL driver is the sole reason an
advisory with no available fix appears in the audit at all. An advisory reading
"no fixed upgrade is available" is not automatically a forced waiver; it may be a
dependency that should not be present.

Removal SHALL be preferred because a waived dependency remains compiled, remains
linked, and remains subject to whatever is found in it next, while the waiver
itself becomes an entry in a list that is not re-read. Removal is permanent and
shrinks the surface rather than annotating it.

Where a dependency is genuinely required and no fixed version exists, a waiver
SHALL record the advisory identifier, why the code path is not reachable in this
application, and a date at which the decision is revisited. A waiver SHALL NOT be
a blanket suppression, and SHALL NOT be added to silence a finding that removal
or an upgrade would resolve.

#### Scenario: An advisory in an unused optional component is removed

- **WHEN** an advisory affects a dependency present only through a package's
  default features and unusable by this application
- **THEN** the dependency SHALL be removed from the resolved graph, and the
  advisory SHALL NOT be waived

#### Scenario: A required dependency with no fix is waived explicitly

- **WHEN** an advisory affects a dependency the application requires and for which
  no fixed version exists
- **THEN** a waiver SHALL record the advisory identifier, the reachability
  reasoning, and a revisit date

#### Scenario: A waiver is not used in place of an available fix

- **WHEN** an advisory has an available fixed version or can be removed
- **THEN** it SHALL NOT be waived

### Requirement: The advisory check does not remain failing

The advisory check SHALL be returned to passing rather than left failing while
findings are triaged.

A check that fails on every build stops carrying information. Contributors learn
that the job is always red, and the next genuine advisory arrives into a signal
nobody reads — the same outcome as not having the check, reached while appearing
to have one. The cost of a permanently failing check is therefore not the
inconvenience; it is the loss of the thing the check was added to provide.

Findings SHALL be resolved by upgrade, removal, or explicit waiver. Reachability
SHALL inform the order in which they are addressed and the depth of scrutiny each
receives, and SHALL NOT be a reason to leave one unresolved indefinitely.

Severity SHALL NOT be the sole basis for that ordering. Severity scores describe
a vulnerability in general, not its reachability in this application: the
highest-scored finding in the first run was inapplicable because it affected a
TLS backend this project does not build, while a lower-scored one sat in the
component that sanitizes publicly served member content.

#### Scenario: The check passes once findings are resolved

- **WHEN** every outstanding finding has been upgraded, removed, or waived with a
  recorded reason
- **THEN** the advisory check SHALL pass

#### Scenario: Reachability, not severity alone, orders the work

- **WHEN** findings differ in both severity score and reachability in this
  application
- **THEN** the ordering SHALL account for reachability rather than following
  severity alone

