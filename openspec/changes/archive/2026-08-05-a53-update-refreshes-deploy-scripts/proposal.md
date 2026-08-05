# Change: Update refreshes the bundled deployment scripts

## Why

`coterie-provision update` replaces the `coterie` and `seed` binaries, `static/`,
`migrations/`, `.env.example`, and `VERSION`. It does not touch `deploy/`. Only
the first-install branch of `release-deploy.sh` ever copies those scripts. So an
instance's `/opt/coterie/deploy/` is frozen at whatever release provisioned it,
no matter how many times it is updated afterwards.

Observed on the production instance on 2026-08-04: the box was running v1.0.20
with a `deploy/` directory dated **9 July** — four months stale. Concretely, it
held the pre-a50 `backup.sh`, which snapshots the database and **not** the upload
roots, and it did not contain `restore.sh` at all, because that file did not
exist when the instance was provisioned. An operator following the current
`RESTORE.md` on that box would have been told to run a script that was not there.

The consequence is worse than one stale script. Anything an operator installs
*from* `deploy/` — the backup systemd units are the live example — can only ever
be as new as the day the instance was first provisioned. A fix shipped in a
release silently never reaches an existing operator, and nothing anywhere reports
the mismatch: `VERSION` says v1.0.20, the scripts beside it are from a release
four months older, and the two look equally current on disk.

This is specified as a **change** rather than filed as an issue because the
current behavior does not violate its spec. `deployment-updates` states that
update SHALL NOT modify `.env` or the database and MAY refresh `.env.example`; it
says nothing at all about `deploy/`. The code matches the spec. The spec is what
is incomplete.

## What Changes

- `update` refreshes `deploy/` from the release, the same way it already
  refreshes `.env.example` — these are release artifacts that ship with the
  binary, not operator configuration.
- A replaced script is preserved as a sibling rather than destroyed. The
  first-install path's own comment says the scripts are copied individually "so
  operators can pin local changes if needed", which means a local edit is a
  supported thing to have made. Silently overwriting one is a data-loss bug, so
  the previous file is kept beside the new one, matching the `coterie.prev`
  convention the binary swap already uses.
- Update reports which scripts changed, so an operator learns that (for example)
  `backup.sh` now bundles uploads without having to diff two directories they had
  no reason to suspect were different.
- The distinction is stated in canon: files that arrive in the release tarball
  are refreshed on update; files the operator authored (`.env`) and the live
  database are not. That boundary is currently implied by a list of examples
  rather than by a rule, which is why `deploy/` fell through it.

Non-goals:

- **No behavior change to what the scripts do.** This change moves files; it does
  not touch `backup.sh`, `restore.sh`, or the systemd units.
- **Update still does not install or enable anything.** Refreshing
  `coterie-backup.timer` on disk does not enable it — an instance provisioned
  before the wizard installed timers still needs the operator to install them
  once. Making update reach into `/etc/systemd/system` is a much larger claim on
  the host and is deliberately not proposed here.
- **No version stamping of individual scripts.** `VERSION` already records the
  release; a per-file marker would be a second source of truth for the same fact.
