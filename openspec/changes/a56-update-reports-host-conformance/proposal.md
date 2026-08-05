# Change: Update reports what the host is missing

## Why

`update` now refreshes `deploy/` so the ops scripts beside a binary are the ones
that shipped with it, and it deliberately installs and enables nothing —
`deployment-updates` states that placing a unit file "SHALL NOT install, enable,
reload, or start anything." That boundary is correct. Reaching into
`/etc/systemd/system` from an update is a claim on the host an update should not
make.

But it leaves a gap that has already cost this project real data protection. A
release ships a capability, the files land in `deploy/`, and the capability is
absent in operation because nothing on the host ever enabled it. Nothing reports
the difference. `VERSION` says the current release. The scripts sit beside it,
current and readable. Everything looks installed.

That is exactly what happened to backups. `coterie-backup.service` and
`coterie-backup.timer` shipped in a release. The instance had been provisioned
before the wizard learned to install them, so they were never enabled, and **no
backup ran for months** — discovered only because someone thought to check
`systemctl list-unit-files`. The same shape recurred on the companion marketing
site days later: a feature merged, deployed, and returning not-found on every
request because its host-side install had never been run.

The common factor is not that installing requires a manual step. It is that
**nothing compares what the release expects to what the host has.** An operator
has no way to ask "is this instance actually in the state this release assumes"
short of knowing what to look for.

## What Changes

- After an update completes, `update` checks whether the host is in the state the
  installed release expects, and names each discrepancy along with the command
  that resolves it.
- The first and motivating check is whether units the release ships are enabled.
  The check is written so more can be added as releases ship more.
- Reporting is not installing. `update` still enables nothing, starts nothing,
  and writes nothing outside the install directory. The boundary
  `deployment-updates` draws is unchanged; this change only makes the far side of
  it visible.
- A discrepancy does not fail the update. The update itself succeeded, and
  exiting non-zero for an advisory finding is how operators learn to ignore exit
  codes.
- A conformant host produces no output at all for this. The section appearing is
  itself the signal, which is only true if it stays absent the rest of the time.

## Why not just install it

Because an update that enables units is an update that can start services an
operator deliberately disabled, on a host whose layout it cannot see. The backup
timer is a good example: an operator may run backups from cron, from a hypervisor
snapshot, or not at all on a staging box. Enabling it on their behalf would be
the wrong call often enough to matter.

Telling them has none of that risk and nearly all of the value. The failure being
fixed is not "the operator declined to enable it" — it is "nobody knew it was not
enabled."

## What this does not do

- **It does not enable, start, reload, or install anything.**
- **It does not fail the update**, ever, on the basis of a conformance finding.
- **It does not check operator configuration.** Whether `.env` is complete, or a
  backup destination is on a sensible disk, are judgments about intent. This
  checks facts the release can state without knowing anything about the
  deployment's intentions.
