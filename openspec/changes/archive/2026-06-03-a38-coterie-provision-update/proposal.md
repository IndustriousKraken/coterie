## Why

Coterie has a working update path (`deploy/release-deploy.sh`) but it is
unhardened in two ways that matter the moment it runs against a real org's
data: it takes **no database snapshot before the swap**, and it does **no
post-restart health check** — `systemctl status` doesn't fail the script if the
new version crash-loops. Because the new binary runs `sqlx::migrate!` on
startup, a bad release or migration can take an instance offline with no fresh
backup and no automatic recovery, and the script still exits `0`.

The durable fix belongs in `coterie-provision` (testable Rust), not in
unvalidatable bash — the same conclusion reached for the install flow in
`a24-provisioning-wizard`. The provision binary already has every piece an
update needs (release resolution, the `SystemCommand`/filesystem traits, the
`/health` smoke test, atomic file placement), all exercised by fakes in
`tests/install_flow.rs`.

## What Changes

- **New `coterie-provision update` subcommand.** It resolves the target
  release (default: latest **stable** GitHub release; `--tag <vX.Y.Z>` pins a
  specific tag for rollback or a chosen version), downloads the **prebuilt
  musl-static release tarball** published by CI, verifies its SHA256, takes a
  pre-update database snapshot, retains the previously-installed binary, swaps
  atomically, restarts the service, runs the existing `/health` smoke test, and
  **rolls back to the previous binary if the smoke test fails**. It never
  compiles Coterie from source on the host and never touches `.env` or the data
  file (beyond the snapshot it creates).
- **Idempotent**: if the resolved target already matches the installed
  `VERSION`, it makes no changes and exits success.
- **Thin `deploy/update.sh` bootstrap** mirroring `deploy/provision.sh`:
  download `coterie-provision`, verify, `exec coterie-provision update`. Heavy
  logic moves out of bash and into the testable binary.
- **`deploy/release-deploy.sh` is reduced to delegate** to the bootstrap so
  there is a single hardened code path rather than two divergent ones.
- **README gains an `## Update` section** directly under `## Deploy`,
  documenting the one-line update command and `--tag`.
- Reuses the `SystemCommand` / filesystem / release-fetch abstractions and the
  `smoke_test` from the install flow; the new flow is unit- and
  integration-tested with the existing fakes (success path and rollback path).

## Capabilities

### New Capabilities
- `deployment-updates`: the behavioral contract for in-place updates of an
  installed Coterie instance — prebuilt-only (never build on host), default to
  latest stable release with tag override, snapshot-before-swap,
  previous-binary retention, health-check-with-automatic-rollback, idempotent
  on the installed version, and config/data safety.

### Modified Capabilities
<!-- None. The update path reuses provisioning-wizard's traits and smoke test
     but does not change any of its requirements. -->

## Impact

- **Code**: `deploy/coterie-provision/` — new `update` module + `Update`
  subcommand in `main.rs`; reuses `github_api`, `version_selector`, `fs_ops`,
  `system` (`SystemCommand`), and `install::smoke_test`. Pre-update snapshot via
  the already-present `rusqlite` (`bundled`) `VACUUM INTO` — no new host
  dependency.
- **Scripts**: new `deploy/update.sh` (thin bootstrap); `deploy/release-deploy.sh`
  simplified to delegate.
- **Docs**: `README.md` `## Update` section.
- **Tests**: new `update_flow` integration test (feature-gated like
  `install_flow`) plus unit tests for release selection and idempotency.
- **No change** to the main `coterie` binary. No new runtime dependency on the
  server beyond what `provision.sh` already requires.
