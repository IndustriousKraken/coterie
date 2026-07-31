# backup-and-restore Specification

## ADDED Requirements

### Requirement: A backup is a single bundle containing the database and every upload root

A backup artifact SHALL contain the vacuumed database snapshot together with the
contents of every upload root — the public uploads directory and the private
uploads directory — in one timestamped archive.

Backing up the database alone SHALL be treated as incomplete, because the database
holds references to files it does not contain: submission rows name attachment
paths and event and announcement rows name image paths. A database-only restore
produces rows pointing at files that do not exist, which surfaces as broken images
and missing attachments rather than as an error, so the operator has no signal
that the restore was partial.

The bundle SHALL be self-contained: restoring it onto a clean host, together with
the configuration file and a binary, SHALL yield a working instance. This is what
makes the same artifact usable for migration as for recovery, and migration is a
first-class use — an operator moving hosts SHALL NOT have to assemble the pieces
by hand.

Any new upload root added in future SHALL be added to the bundle in the same
change that introduces it. Omitting one fails silently at restore time, which is
the worst moment to discover it.

#### Scenario: A backup contains uploads as well as the database

- **WHEN** a backup runs on an instance holding submission attachments and event
  images
- **THEN** the resulting artifact SHALL contain the database snapshot and the
  contents of both upload roots

#### Scenario: Restoring a bundle onto a clean host yields a working instance

- **WHEN** an operator restores a bundle onto a host with configuration and a
  binary but no data
- **THEN** the instance SHALL serve its previously stored images and attachments,
  not merely its database rows

#### Scenario: A database-only artifact is not a valid backup

- **WHEN** an artifact contains the database but no upload roots
- **THEN** it SHALL NOT be considered a complete backup, because the rows it
  restores reference files it does not carry

### Requirement: The database snapshot is application-consistent, not a file copy

The database portion of a backup SHALL be produced with `VACUUM INTO`, yielding a
single self-contained file in one atomic SQLite operation.

The live database file SHALL NOT be copied directly. In WAL mode that file is
incomplete without its `-wal` and `-shm` siblings, so a plain copy of a running
database can restore to a torn state. `VACUUM INTO` makes consistency a property
of how the artifact was produced rather than of when it happened to be taken.

#### Scenario: A backup taken while the service is running is consistent

- **WHEN** a backup runs against a live database receiving writes
- **THEN** the resulting snapshot SHALL be a self-contained, integrity-checkable
  database file requiring no WAL replay to restore

### Requirement: Restore is an executable procedure, not only a document

The system SHALL provide a restore script that accepts a backup bundle and
restores it, rather than relying solely on a documented sequence of manual
commands.

The script SHALL, in order: verify the bundle is readable and contains the
expected components before changing anything; stop the service; move the existing
database and upload roots aside rather than deleting them; restore the bundle's
contents; correct ownership so the service account can read them; run an integrity
check on the restored database; and start the service.

Displaced existing data SHALL be retained, not removed, so a restore that turns
out to be the wrong snapshot is itself reversible. A restore is performed under
time pressure by someone who has usually not done it before, which is the
condition under which an eight-step manual procedure produces mistakes.

Verification SHALL precede any destructive step, so a corrupt or truncated bundle
fails before the running instance has been disturbed.

The manual procedure SHALL remain documented as the fallback for cases where the
script cannot run.

#### Scenario: A corrupt bundle is rejected before anything is touched

- **WHEN** the restore script is given a truncated or unreadable bundle
- **THEN** it SHALL fail before stopping the service or moving any existing data

#### Scenario: Replaced data is retained, not deleted

- **WHEN** a restore replaces an existing database and upload roots
- **THEN** the displaced files SHALL be preserved so the operator can reverse the
  restore

#### Scenario: A restored database is integrity-checked before the service starts

- **WHEN** the restore script has put the database in place
- **THEN** it SHALL run an integrity check and SHALL NOT start the service if that
  check fails

### Requirement: Backups run by default on a provisioned deployment

A deployment created by the provisioning path SHALL have the backup timer
installed and enabled, so that scheduled backups run without a separate operator
action.

Shipping a backup script that nothing schedules SHALL NOT be treated as delivering
backups. The failure mode is silent and total: the feature appears present in the
repository, the operator reasonably assumes it is active, and the absence is
discovered only when a restore is needed. A production instance ran for three
weeks in exactly that state before this was noticed.

The deploy documentation SHALL state how an existing deployment installs the timer
retroactively, because provisioning covers only new deployments.

#### Scenario: A freshly provisioned host has a scheduled backup

- **WHEN** an operator completes the provisioning path
- **THEN** the backup timer SHALL be installed and enabled, and a scheduled backup
  SHALL occur without further action

#### Scenario: The absence of a schedule is detectable

- **WHEN** an operator checks whether backups are configured
- **THEN** the presence of the enabled timer SHALL be the answer, discoverable
  through the service manager rather than inferred from files existing on disk

### Requirement: A backup can be taken on demand

An operator SHALL be able to produce a complete bundle at any time by running the
backup script directly, without waiting for or disturbing the schedule.

An on-demand run SHALL produce the same artifact shape as a scheduled run, so the
migration path and the recovery path exercise identical code. A separate
"migration export" would be a second mechanism that is used rarely and therefore
tested rarely.

#### Scenario: An operator takes a bundle before migrating hosts

- **WHEN** an operator runs the backup script manually ahead of moving to another
  provider
- **THEN** it SHALL produce a complete bundle equivalent to a scheduled one, usable
  as the migration artifact
