# a50-backups-complete-and-running

## Why

Coterie's backup feature is written but does not protect a deployment. Two
independent gaps, found while checking the production box on 2026-07-31:

**It isn't running.** `deploy/backup.sh`, `coterie-backup.service` and
`coterie-backup.timer` ship in the repo and sit in `/opt/coterie/deploy/` on the
server, but nothing installs them. `systemctl list-unit-files | grep coterie`
returns `coterie.service` and nothing else; no timer is scheduled. The
provisioning wizard — the documented first-deploy path — never references the
backup units at all, so **no wizard-provisioned deployment has backups** unless an
operator knew to install them by hand. Production has run since 2026-07-09 with
no automated database backup.

**It wouldn't be complete if it were.** `backup.sh` snapshots only the database.
It never touches `uploads/` or `private-uploads/`. Restoring from a backup would
bring back submission rows whose attachment files no longer exist and event rows
whose images 404. `RESTORE.md` documents the database restore and does not mention
uploads at all — so an operator following it exactly ends up with a
half-restored instance and no indication anything is missing.

`MIGRATION.md` already gets this right — it tars **both** upload roots and even
warns that naming only `uploads` "is how it would silently fail to migrate." The
backup path never learned the same lesson.

This is a change rather than an issue because the fix alters what a backup *is*
(its contents and artifact shape) and adds a step to provisioning.

## What Changes

- **A backup is one bundle**: the vacuumed database plus both upload roots, in a
  single timestamped archive. One artifact restores to a working instance, which
  is what makes it usable for migration as well as recovery.
- **`deploy/restore.sh` exists.** Restoring is currently eight manual steps in a
  markdown file, each an opportunity to fumble a path while the service is down.
  A script that verifies the bundle, stops the service, moves the current state
  aside rather than deleting it, restores, fixes ownership, and integrity-checks
  the database is the difference between a procedure and a hope.
- **Provisioning installs and enables the backup timer** so a fresh deployment is
  protected by default rather than by remembering.
- **`RESTORE.md` covers uploads** and points at the script, keeping the manual
  steps as the fallback for when the script cannot run.

## Impact

- **Spec:** new capability `backup-and-restore` (5 ADDED requirements). MODIFIED
  `provisioning-wizard` — the wizard's install steps gain the backup timer.
- **Code:** `deploy/backup.sh` (bundle both upload roots), new
  `deploy/restore.sh`, `docs/deploy/RESTORE.md`, and the provisioning wizard's
  install sequence plus its non-interactive equivalent.
- **Operator action after this lands:** existing deployments still need the units
  installed once — provisioning only covers new ones. The change includes that
  step in the deploy docs so it is discoverable rather than folklore.

## On DigitalOcean's droplet backups

Worth stating plainly, because "the host already backs up the disk" is a
reasonable thing to assume and it covers less than it appears to.

DO backups are **good for one failure**: the droplet is gone and you need it back.
They are a poor fit for the others:

- **Granularity.** Restoring one file, or yesterday's database, means standing up
  a whole droplet from an image and copying out of it.
- **Consistency.** A disk snapshot of a live SQLite database in WAL mode is
  crash-consistent, not application-consistent. SQLite usually recovers, but
  "usually" is doing real work in that sentence. `VACUUM INTO` produces a
  guaranteed-consistent file by construction.
- **Retention depth.** Weekly snapshots with a handful of slots will not answer
  "a member was deleted three weeks ago."
- **Same failure domain.** Same provider, same account, same billing relationship
  as the thing being protected.
- **Portability — the one that matters here.** A DO snapshot restores to a DO
  droplet. It is not a migration artifact. A tarball of database plus uploads
  restores anywhere, which is precisely the stated goal.

So: keep DO backups as disaster recovery, and treat this feature as operational
recovery and portability. They are complements, not substitutes, and neither
covers the other's cases.

## Deferred

- **Uploads are re-archived in full on every run.** Fine at the current scale
  (8 image files); wasteful once an org accumulates hundreds of megabytes of event
  images across 7 daily + 4 weekly + 12 monthly slots. The upgrade path is a
  `--db-only` mode for frequent runs with a less frequent full bundle, or
  hardlink-based deduplication between slots. Not built now because it is
  speculative and the simple version is correct.
- **Real-systemd behaviour is still unverified.** The end-to-end round trip runs
  against a temp data dir, which proves the bundle restores to a working instance
  but not that the timer installs correctly or that stop/start sequencing behaves
  on a host with real systemd. Those paths are covered only by trait-level unit
  tests with fakes. The roadmap's 1.2 note about a live host applies to that
  remainder, which is now much smaller than "has restore ever worked at all".
