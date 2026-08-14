# dependency-maintenance Specification Delta

## ADDED Requirements

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
