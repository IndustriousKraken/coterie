# Tasks

## 1. The check

- [ ] 1.1 `deploy/coterie-provision/src/update.rs`: after the placement and
  version write, run a conformance check and collect its findings. It runs after
  the update has otherwise succeeded, so a failure to inspect the host cannot
  affect the update's outcome.
- [ ] 1.2 First check: for each unit file the release ships under `deploy/`, is
  a unit of that name enabled on the host? `coterie-backup.timer` is the case
  that motivated this — it shipped in a release, was never enabled on an instance
  provisioned before the wizard installed it, and no backup ran for months.
- [ ] 1.3 Structure it as a list of checks, each returning an optional finding
  with a message and a resolving command, so adding one later is adding an entry
  rather than editing the reporting logic.
- [ ] 1.4 Derive the unit list from what is actually present in the release's
  `deploy/` directory, not from a hardcoded list. A hardcoded list is how a unit
  shipped later goes unchecked.
- [ ] 1.5 Route host inspection through the existing `SystemCommand` abstraction.
  `deployment-updates` requires the update flow be drivable by fakes; a direct
  process call here would break that for the whole flow.
- [ ] 1.6 A check that cannot determine the host's state SHALL NOT report a
  discrepancy. Reporting "backup timer not enabled" because the query failed is
  worse than reporting nothing — it sends an operator to fix something that may
  not be broken.

## 2. Reporting

- [ ] 2.1 Print each finding with its resolving command. An operator reading the
  output should be able to act without opening a README.
- [ ] 2.2 Print nothing — no header, no "all clear" line — when there are no
  findings. The section's presence is the signal, which only works if it stays
  absent otherwise.
- [ ] 2.3 Emit through the same `Output` sink the rest of `update` uses.
- [ ] 2.4 Exit zero regardless of findings.

## 3. Boundaries

- [ ] 3.1 Enable nothing, start nothing, reload nothing. `deployment-updates`
  already states that placing a unit file does not activate it; this check must
  not become the thing that quietly does. A comment should say why, because
  "it already knows what's wrong, why not just fix it" is the obvious next
  thought for whoever reads this.
- [ ] 3.2 Write nothing outside the install directory.
- [ ] 3.3 Do not check operator configuration — whether `.env` is complete, or
  whether the backup destination sits on a sensible disk. Those are judgments
  about intent; this reports facts the release can state without knowing the
  deployment's intentions.

## 4. Tests

- [ ] 4.1 A host with a shipped unit not enabled produces a finding naming the
  unit and the enabling command.
- [ ] 4.2 A conformant host produces no output for this at all — assert the
  absence of a header, not just the absence of findings.
- [ ] 4.3 Both cases exit zero.
- [ ] 4.4 The check issues no `systemctl enable`, `start`, or `reload` — assert
  via the `SystemCommand` fake. This is the guard that keeps 3.1 true as the code
  grows.
- [ ] 4.5 A unit added to the release's `deploy/` is checked without a code
  change, proving 1.4.
- [ ] 4.6 A failed host query yields no finding rather than a false one.
- [ ] 4.7 The full update flow, including this check, still runs under fakes with
  no real process execution.

## 5. Documentation

- [ ] 5.1 In `docs/deploy/OPS.md`, note that update reports host discrepancies and
  does not fix them, and that silence means nothing was found.
