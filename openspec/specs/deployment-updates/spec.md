# deployment-updates Specification

## Purpose
TBD - created by archiving change a38-coterie-provision-update. Update Purpose after archive.
## Requirements
### Requirement: Updates install a prebuilt release and never build on the host

`coterie-provision update` SHALL obtain the new version by downloading the
prebuilt `x86_64-unknown-linux-musl` release tarball published by CI for the
target tag, verifying its SHA256 before use. It SHALL NOT compile Coterie from
source on the host and SHALL NOT invoke a compiler or `cargo`.

#### Scenario: Update downloads and verifies the prebuilt artifact

- **WHEN** an update runs for a resolved target tag
- **THEN** it SHALL download that tag's release tarball and its `.sha256`,
  verify the checksum, and install the binaries from the extracted tarball
- **AND** it SHALL NOT run any build/compile step on the host

#### Scenario: Checksum mismatch aborts the update

- **WHEN** the downloaded tarball's SHA256 does not match the published checksum
- **THEN** the update SHALL abort before stopping the service or swapping any
  file, and exit non-zero

### Requirement: Update target defaults to the latest stable release and is overridable by tag

With no tag argument, `update` SHALL resolve the latest GitHub release that is
not a prerelease. A `--tag <vX.Y.Z>` argument SHALL pin an exact tag, which is
the mechanism for both rollback and installing a specific version. Prereleases
SHALL be excluded from default resolution.

#### Scenario: No tag resolves to latest stable

- **WHEN** `update` is invoked with no tag
- **THEN** it SHALL select the most recent release whose prerelease flag is false

#### Scenario: Explicit tag is honored

- **WHEN** `update --tag v1.2.3` is invoked
- **THEN** it SHALL target release `v1.2.3` regardless of which release is latest

#### Scenario: Latest release is a prerelease

- **WHEN** the most recent release is flagged as a prerelease and `update` is
  invoked with no tag
- **THEN** it SHALL skip the prerelease and select the most recent stable release

### Requirement: Update is idempotent on the installed version

`update` SHALL make no changes and exit success when the resolved target tag
already equals the currently installed version recorded in the deployment's
`VERSION` file, and SHALL NOT restart the service in that case.

#### Scenario: Already on the target version

- **WHEN** the resolved target equals the installed `VERSION`
- **THEN** `update` SHALL exit `0` without stopping the service, swapping files,
  or taking a snapshot

### Requirement: A database snapshot is taken before the binary is swapped

Before stopping the service or moving any installed file, `update` SHALL create
a timestamped snapshot of the live database (a SQLite `VACUUM INTO` copy). If the
snapshot cannot be created, `update` SHALL abort before making any change.

#### Scenario: Snapshot precedes the swap

- **WHEN** an update proceeds past version and checksum checks
- **THEN** it SHALL create the database snapshot BEFORE the service is stopped or
  any binary/file is swapped

#### Scenario: Snapshot failure aborts the update

- **WHEN** the pre-update snapshot fails
- **THEN** `update` SHALL abort with a non-zero exit and SHALL NOT have stopped
  the service or swapped any file

### Requirement: The previous binary is retained for rollback

`update` SHALL preserve the previously-installed Coterie binary (and a record of
its version) so a failed update can be reverted without re-downloading from
GitHub.

#### Scenario: Previous binary retained after swap

- **WHEN** the new binary is promoted
- **THEN** the prior binary SHALL be retained on disk (e.g. as `coterie.prev`)
  for rollback

### Requirement: A failed health check rolls back to the previous binary

After restarting the service, `update` SHALL run the `/health` smoke test within
the established retry budget. If the smoke test fails, `update` SHALL restore the
retained previous binary, restart the service, and exit non-zero with operator
guidance. That guidance SHALL state that if a database migration already ran,
restoring the pre-update snapshot may be required, because restoring the binary
does not undo schema changes.

#### Scenario: Healthy after restart

- **WHEN** the service is healthy within the smoke-test budget after restart
- **THEN** `update` SHALL report the new version and exit `0`

#### Scenario: Unhealthy after restart triggers rollback

- **WHEN** the smoke test does not pass within its budget after restart
- **THEN** `update` SHALL restore the previous binary, restart the service, exit
  non-zero, and print guidance that the pre-update snapshot may need to be
  restored if a migration already ran

### Requirement: Update never modifies operator configuration or data

`update` SHALL NOT modify the deployment's `.env` file or the live database file,
other than the pre-update snapshot it creates. It MAY refresh `.env.example` so
operators can diff it against their live `.env` for newly-introduced settings.

The boundary SHALL be drawn by provenance, not by an enumerated list of paths: a
file that arrives in the release tarball is a release artifact and MAY be
refreshed by `update`, while a file the operator authored — `.env` — and the live
database SHALL NOT be. Stating the rule this way is deliberate. The previous
enumeration silently excluded the bundled `deploy/` scripts, which are release
artifacts by every test that matters and were nonetheless never refreshed on any
instance after its first install.

#### Scenario: Configuration left intact

- **WHEN** an update completes
- **THEN** the live `.env` and database file SHALL be unchanged (apart from the
  pre-update snapshot), while `.env.example` MAY be refreshed to the new version

#### Scenario: An operator-authored file is not a release artifact

- **WHEN** an update completes on a deployment whose `.env` differs from the
  release's `.env.example`
- **THEN** `.env` SHALL be untouched regardless of how far it has diverged

### Requirement: Update side-effects are behind testable abstractions

The update path SHALL route all process execution, filesystem access,
network/release retrieval, and database-snapshot work through the same
`SystemCommand` / filesystem / release-fetch abstractions used by the install
flow, so the full update flow — including the rollback path — is unit- and
integration-testable with fakes and no real network, process, filesystem, or
database access.

#### Scenario: Full update flow is driven by fakes in tests

- **WHEN** the update flow runs under test with the test-support fakes
- **THEN** both the success path and the unhealthy-rollback path SHALL be
  exercisable without performing any real network, process, filesystem, or
  database operation

### Requirement: Update refreshes the bundled deployment scripts

`update` SHALL refresh the deployment's `deploy/` directory from the release
being installed, so the ops scripts beside a binary are the ones that shipped
with it. Without this, `deploy/` retains whatever release first provisioned the
instance, and an operator following current documentation is directed at scripts
that may be arbitrarily old or absent — a restore procedure naming a script that
does not exist on the box is the failure mode this prevents.

A refreshed file SHALL NOT be destroyed. Where `update` replaces a script, the
previous contents SHALL be preserved alongside it, matching the retention the
binary swap already performs for `coterie.prev`. Local modification of these
scripts is a supported operator action, so replacing one without leaving a copy
would discard operator work.

`update` SHALL report which deployment scripts it changed, naming them, so the
operator learns that an ops script's behavior has moved. A script silently
gaining or losing capability between releases is not discoverable by any other
means the deployment offers.

Refreshing a systemd unit file under `deploy/` SHALL NOT install, enable,
reload, or start anything. `update` places files inside the install directory
only; whether a unit is active on the host remains an explicit operator or
installer action.

#### Scenario: Deployment scripts match the installed release

- **WHEN** an update to a release whose `deploy/backup.sh` differs from the
  installed one completes
- **THEN** the deployment's `deploy/backup.sh` SHALL be the release's version

#### Scenario: A script new to the release appears

- **WHEN** the target release contains a `deploy/` script that the deployment does
  not have
- **THEN** that script SHALL be present after the update

#### Scenario: The replaced script is retained

- **WHEN** `update` replaces a deployment script that the operator had modified
- **THEN** the previous contents SHALL remain available on disk beside the new
  file

#### Scenario: Changed scripts are named in the output

- **WHEN** an update refreshes one or more deployment scripts
- **THEN** the output SHALL name the scripts that changed

#### Scenario: Refreshing a unit file does not activate it

- **WHEN** an update refreshes `deploy/coterie-backup.timer` on a host where that
  timer is not enabled
- **THEN** the timer SHALL remain not enabled, and nothing under
  `/etc/systemd/system` SHALL be written by the update

