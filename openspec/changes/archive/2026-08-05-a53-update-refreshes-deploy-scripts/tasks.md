# Tasks

## 1. Refresh the scripts

- [x] 1.1 `deploy/coterie-provision/src/update.rs`: extend the placement step
  (`swap_binaries`, or a sibling called from the same point) to copy the release's
  `deploy/` contents into `<install_dir>/deploy/`. Place it after the binary swap
  and before the `VERSION` write, so a failed placement leaves `VERSION` naming
  the old release rather than claiming a version the tree does not match.
- [x] 1.2 Copy file-by-file, not by replacing the directory. `static/` and
  `migrations/` are replaced wholesale because they are wholly release-owned;
  `deploy/` may hold operator files that are not in the tarball, and a wholesale
  replace would delete them.
- [x] 1.3 Before overwriting a file whose contents differ, write the existing
  contents to a sibling `<name>.prev`, matching the `coterie.prev` convention in
  the same module. A file whose contents are byte-identical is not rewritten and
  produces no `.prev`, so repeated updates do not churn.
- [x] 1.4 Preserve the executable bit on `*.sh` — `backup.sh` and `restore.sh`
  are invoked directly by the systemd unit and by operators.
- [x] 1.5 Route every filesystem operation through the existing `FileSystem`
  abstraction. `deployment-updates` canon requires the update path be drivable by
  fakes with no real filesystem access, and a direct `std::fs` call here would
  break that for the whole flow.

## 2. Reporting

- [x] 2.1 Collect the names of scripts added or changed and emit them through the
  same `Output` sink the rest of `update` uses. Name the files; a bare count tells
  the operator nothing actionable.
- [x] 2.2 Say nothing when nothing changed. An update that refreshes no script
  should not print a section about scripts.
- [x] 2.3 Where a `.prev` was written, say so on that file's line, so an operator
  who had pinned a local edit finds out immediately rather than discovering it the
  next time the script runs.

## 3. Boundaries

- [x] 3.1 Do not touch `/etc/systemd/system` or invoke `systemctl` from `update`.
  Refreshing `deploy/coterie-backup.timer` places a file inside the install dir
  and nothing more. The wizard's `install_backup_timer` remains the only code that
  activates units.
- [x] 3.2 Do not write `.env`. The existing guard stays exactly as it is.
- [x] 3.3 The idempotent-on-installed-version early return keeps precedence: an
  update that exits because the target equals `VERSION` refreshes nothing,
  including scripts.

## 4. Tests

- [x] 4.1 With fakes: a release whose `deploy/backup.sh` differs leaves the
  install dir holding the release's version, and a `backup.sh.prev` holding the
  old one.
- [x] 4.2 A release script byte-identical to the installed one produces no
  `.prev` and no report line.
- [x] 4.3 A script present in the release but absent from the install dir is
  created.
- [x] 4.4 A file present in the install dir's `deploy/` but absent from the
  release is left alone — this is the operator-authored case that a wholesale
  directory replace would have deleted.
- [x] 4.5 The rollback path is unaffected: a failed smoke test restores the
  previous binary, and the test asserts the update did not leave `VERSION`
  claiming the new release.
- [x] 4.6 Assert no `systemctl` invocation and no write outside the install dir
  occurs during an update, using the existing `SystemCommand` fake. This is the
  guard that keeps 3.1 true as the code changes.
- [x] 4.7 Executable bits survive the copy for `*.sh`.

## 5. Documentation

- [x] 5.1 `docs/deploy/OPS.md`: state that `deploy/` now tracks the installed
  release, and that a locally-modified script is preserved as `.prev` when
  replaced.
- [x] 5.2 Note in the same place that refreshed systemd unit files are not
  activated by an update — an instance provisioned before the backup timer existed
  still installs it once by hand, per `RESTORE.md`. This is the exact gap that
  left a four-month-old `backup.sh` in production, so it is worth saying plainly
  rather than leaving an operator to infer it.
