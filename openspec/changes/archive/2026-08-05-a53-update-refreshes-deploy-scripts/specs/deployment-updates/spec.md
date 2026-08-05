# deployment-updates Specification Delta

## MODIFIED Requirements

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

## ADDED Requirements

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
