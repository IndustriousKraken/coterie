# Tasks

Two gaps, both silent: backups do not include uploads, and backups are not
running. Either alone makes the feature not protect a deployment.

## 1. Bundle both upload roots

- [ ] 1.1 `deploy/backup.sh`: after `VACUUM INTO`, produce a single timestamped
  archive containing the snapshot plus `uploads/` and `private-uploads/` from the
  data dir. Keep `VACUUM INTO` — a plain copy of a WAL database can restore torn.
- [ ] 1.2 Derive the upload roots from the data dir rather than naming them
  absolutely, so a non-default `COTERIE__SERVER__DATA_DIR` still backs up.
- [ ] 1.3 Handle a missing or empty upload root as normal, not an error — a fresh
  instance has neither.
- [ ] 1.4 Keep retention, weekly/monthly promotion, and the optional S3 push
  operating on the bundle.
- [ ] 1.5 Note the growth ceiling in the script: uploads are re-archived in full
  each run, which is fine at current scale and wasteful once an org has hundreds
  of megabytes of images. Name the upgrade path (`--db-only` for frequent runs, or
  hardlink dedup between slots) so the next person does not have to rediscover it.
- [ ] 1.6 **Never archive the backup directory into a bundle.** Its default
  (`COTERIE_BACKUP_DIR`, `{data_dir}/backups`) sits *inside* the data dir, so a
  future "just tar the whole data dir" simplification would nest each backup
  inside the next and grow without bound. Archive the two upload roots by name,
  and if the backup dir resolves inside the data dir, exclude it explicitly rather
  than relying on the enumeration staying narrow.
- [ ] 1.7 Document that `COTERIE_BACKUP_DIR` should point at a **separate volume**
  where one is available. The default keeps backups on the same disk as the data
  they protect, so a single disk failure takes both. This needs no code — the
  variable already exists — but the deploy docs currently never say to set it.

## 2. Restore script

- [ ] 2.1 New `deploy/restore.sh` taking a bundle path.
- [ ] 2.2 Verify first: the bundle is readable and contains the expected
  components. Fail before touching anything if not.
- [ ] 2.3 Then: stop the service; move the existing database **and** both upload
  roots aside (do not delete); extract; `chown` to the service account; run
  `PRAGMA integrity_check`; start the service only if it passes.
- [ ] 2.4 Refuse to run as a non-root user with a clear message rather than
  failing halfway through with a permissions error.
- [ ] 2.5 Print where the displaced data went, so reversing the restore is
  obvious rather than archaeological.

## 3. Provisioning installs the timer

- [ ] 3.1 Wizard install step: copy `coterie-backup.service` and
  `coterie-backup.timer` into place and enable the timer. Route the systemd and
  filesystem calls through the wizard's existing side-effect traits, per
  `provisioning-wizard`'s "Side-effecting code is behind testable traits" — so
  this is implementable and unit-testable in the sandbox with a fake, and no
  `systemctl` is invoked during the build.
- [ ] 3.2 Idempotent — a re-run detects an installed timer and leaves it alone.
- [ ] 3.3 Non-interactive equivalent, per the wizard's existing rule that every
  prompt has one.
- [ ] 3.4 `--dry-run` shows the backup-install step like any other.

## 4. Documentation

- [ ] 4.1 `docs/deploy/RESTORE.md`: restore via the script as the primary path;
  keep the manual steps as the fallback; cover uploads, which it currently does
  not mention at all.
- [ ] 4.2 Add the retroactive install for existing deployments — provisioning only
  covers new hosts, and every deployment made before this change has no timer.
- [ ] 4.3 `MIGRATION.md`: point at the bundle as the migration artifact rather
  than its current hand-assembled tar steps. It already correctly names both
  upload roots; the bundle supersedes doing it by hand.
- [ ] 4.4 State in `RESTORE.md` that DO-style disk snapshots are disaster recovery
  and this is operational recovery plus portability — so an operator does not
  conclude one replaces the other.

## 5. Tests

- [ ] 5.1 A bundle taken from an instance with attachments and images contains
  all three components.
- [ ] 5.2 A truncated bundle is rejected with the service still running and data
  untouched.
- [ ] 5.3 Restore preserves displaced data.
- [ ] 5.4 Backup and restore both succeed with a non-default data dir — set
  `COTERIE__SERVER__DATA_DIR` to a temp path rather than the default. A hardcoded
  `/var/lib/coterie` in either script would pass every default-path test and fail
  on any deployment using the older `/opt/coterie/data` layout.

## 6. End-to-end round trip

The roadmap's outstanding 1.2 item is "one live restore test". That does not need
a cloud host — it needs *a* host, and the build sandbox is one. Run the whole
cycle locally against a temp data dir and tear it down afterwards. A restore
procedure nobody has executed has unknown defects, and an incident is the wrong
time to find them.

- [ ] 6.1 Stand up an instance against a temp data dir: run migrations, create a
  member, store a submission attachment and an event image so all three bundle
  components are non-empty.
- [ ] 6.2 Run `backup.sh` against it and assert a bundle is produced.
- [ ] 6.3 Destroy the data dir entirely — database and both upload roots.
- [ ] 6.4 Run `restore.sh` with the bundle, then start the instance and assert:
  the database passes `integrity_check`, the member row is back, the attachment
  is fetchable through its gated route, and the image is served by
  `/uploads/:filename`. Restoring rows is not the assertion — restoring a
  *working instance* is.
- [ ] 6.5 Tear the temp data dir down so the test leaves nothing behind.
- [ ] 6.6 Wire it into the suite so it runs on every change, not once. This is the
  test that would have caught a database-only backup, which is the defect this
  whole change exists to fix.
