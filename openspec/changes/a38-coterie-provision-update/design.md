## Context

`deploy/release-deploy.sh` already performs a mechanically-correct in-place
update: idempotent `VERSION` check, atomic binary swap (`.new` + `mv`), wholesale
replacement of `static`/`migrations`, and it never touches `.env` or the data
directory. What it lacks is the safety scaffolding that matters in production:

1. No database snapshot before the swap. The new binary runs `sqlx::migrate!`
   on startup (`src/main.rs`), so an update is also a schema migration with no
   fresh backup to fall back to.
2. No post-restart verification. It runs `systemctl status` (which never fails
   the script), so a crash-looping release exits `0` while the instance is down.

`coterie-provision` is the natural home for the fix. `a24-provisioning-wizard`
already established that side-effecting deploy logic lives in this binary behind
testable traits rather than in bash, precisely because the autocoder can
validate Rust with `cargo test` but cannot validate bash behavior. The binary
already has: `github_api::fetch_releases` + `version_selector` (release
resolution), `SystemCommand` (service/process control with a fake), a filesystem
abstraction, atomic placement helpers, and `install::smoke_test` (polls
`/health` within a 30s budget, with a test-configurable budget). `rusqlite` with
the `bundled` feature is already a dependency, so a `VACUUM INTO` snapshot can be
done in-process — no `sqlite3` CLI dependency on the host.

## Goals / Non-Goals

**Goals:**
- One hardened update path that is safe to run unattended against a live org.
- A pre-update snapshot and an automatic rollback so a bad release can't leave an
  instance both down and un-recoverable.
- Full unit/integration coverage via the existing fakes (success + rollback),
  mirroring `tests/install_flow.rs`.
- Reuse, not reinvention: the same traits and smoke test the install flow uses.

**Non-Goals:**
- **Building from source on the host.** Updates install the prebuilt
  musl-static artifact CI already produced. Compiling on a single-core VPS is
  out of scope and explicitly disallowed.
- **Deploying untagged `master` commits / a prerelease "edge" channel.** A
  release artifact only exists for a tag. Shipping an untagged commit means
  cutting a tag so CI builds it. A `--prerelease`/`--channel` flag is a possible
  future extension, not part of this change.
- **Down-migrations / automatic schema rollback.** `sqlx` migrations are
  forward-only. Binary rollback handles the "won't boot before any migration
  ran" case; the pre-update snapshot covers the "a migration already ran" case.
- **Zero-downtime / blue-green deploys.** A short restart window is acceptable at
  this scale.
- **Multi-arch.** `x86_64-unknown-linux-musl` only, matching the published
  release assets.

## Decisions

### D1. Logic in `coterie-provision`, shell is a thin bootstrap

The backup / health-check / rollback logic goes in Rust where it is testable.
`deploy/update.sh` becomes a thin bootstrap that mirrors `deploy/provision.sh`:
download the `coterie-provision` binary for the requested (or latest) release,
verify its checksum, and `exec coterie-provision update "$@"`. `release-deploy.sh`
is reduced to delegating to `update.sh` so there is one code path, not two.

### D2. Prebuilt-only; never compile on the host

`update` downloads `coterie-<tag>-x86_64-linux-musl.tar.gz` (the same asset
`release-deploy.sh` consumes today) and installs the binaries from it. It never
invokes `cargo`/a compiler. This directly answers the single-core-VPS concern:
the build already happened in CI on tag; the server only ever downloads.

### D3. Default to latest stable release; `--tag` overrides

With no argument, `update` resolves the latest **non-prerelease** GitHub release.
`--tag <vX.Y.Z>` pins an exact tag — this is both the rollback mechanism and the
"I want this specific version" mechanism. Prereleases are excluded from the
default so an `-rc` tag is never auto-pulled. (`version_selector` already parses
the releases list; this adds a "latest stable" selection and a tag lookup.)

### D4. Snapshot before swap, abort on snapshot failure

Before stopping the service or moving any file, `update` writes a timestamped
`VACUUM INTO` snapshot of the live database (in-process via `rusqlite`,
consistent with what `deploy/backup.sh` does out-of-process). If the snapshot
fails, `update` aborts before making any change — better to refuse the update
than to migrate with no safety net.

### D5. Retain the previous binary

The swap writes the new binary alongside, then moves the current binary to
`coterie.prev` (and records the previous `VERSION`) before promoting the new one.
This makes rollback instant and offline — it does not depend on GitHub still
serving the old release.

### D6. Health-check with automatic rollback

After restart, `update` runs the existing `smoke_test` (service active +
`/health` 200 within the 30s budget). On success it removes the staged
`.prev`-era scratch and reports the new version. On failure it restores
`coterie.prev`, restarts, and exits **non-zero** with operator guidance —
including an explicit note that **if a migration already ran, restoring the
pre-update snapshot (D4) may be required**, because binary rollback does not undo
schema changes.

### D7. Idempotent on the installed version

If the resolved target equals the current `/opt/coterie/VERSION`, `update` makes
no changes and exits `0` (matching `release-deploy.sh`'s current behavior).

### D8. Everything behind the existing traits

All process, filesystem, network, and database-snapshot access goes through the
`SystemCommand` / filesystem / release-fetch abstractions (adding a snapshot
seam where needed) so the `update_flow` test can drive the full success and
rollback paths with fakes and no real network/process/FS/DB — the same approach
`tests/install_flow.rs` uses for install.

## Risks / Trade-offs

- **Brief downtime** during stop → swap → migrate → restart. Acceptable at this
  scale; documented.
- **Smoke-test false negative** on a slow boot → mitigated by reusing the 30s
  retry budget already tuned for install.
- **Snapshot cost** (time + disk) on every update → acceptable; it is the safety
  net the whole change exists to provide. Snapshot retention/rotation is left to
  the existing `backup.sh` timer; `update` only adds the pre-update snapshot.

## Migration Plan

Single change:

1. Add the `Update` subcommand + `update` module in `coterie-provision`, reusing
   `github_api`/`version_selector`/`fs_ops`/`system`/`smoke_test`.
2. Add release selection (latest-stable + tag) and the in-process snapshot seam.
3. Implement swap-with-retain, restart, smoke-test, and rollback.
4. Add `deploy/update.sh` (thin bootstrap) and reduce `release-deploy.sh` to
   delegate.
5. Add the README `## Update` section.
6. Add the `update_flow` integration test (success + rollback) and unit tests
   for release selection + idempotency.
7. `cargo build`, `cargo test -p coterie-provision --features test-support`,
   `cargo clippy --deny warnings`, `cargo fmt --check`, `bash -n` the scripts,
   `openspec validate`.
