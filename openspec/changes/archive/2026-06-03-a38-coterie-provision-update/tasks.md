## 1. `update` subcommand skeleton

- [x] 1.1 In `deploy/coterie-provision/src/main.rs`, add an `Update(Box<UpdateCli>)`
      variant to the `Command` enum with a doc comment ("Update an installed
      Coterie instance to a released version.").
- [x] 1.2 Define `UpdateCli` with `--tag <vX.Y.Z>` (`Option<String>`, env
      `COTERIE_PROVISION_UPDATE_TAG`) and an `--install-dir` override
      (`Option<PathBuf>`, default `/opt/coterie`) so tests can point at a temp
      dir. Wire the match arm to `update::run(...)`.
- [x] 1.3 Add `pub mod update;` to `deploy/coterie-provision/src/lib.rs`.

## 2. Release resolution (default latest stable, tag override)

- [x] 2.1 In `src/version_selector.rs`, add a function that, given the parsed
      releases, returns the latest release whose `prerelease` flag is false
      (latest **stable**), and a function that finds a release by exact tag.
- [x] 2.2 In `update::run`, resolve the target: if `--tag` is set, look it up;
      otherwise pick latest stable. Reuse `github_api::fetch_releases`. Error
      clearly if no stable release exists or the requested tag is not found.

## 3. Download + verify the prebuilt artifact

- [x] 3.1 Build the asset + checksum URLs deterministically from the tag
      (`coterie-<tag>-x86_64-unknown-linux-musl.tar.gz` and its `.sha256`),
      matching the names produced by `.github/workflows/release.yml`.
- [x] 3.2 Download both into a temp dir (behind the existing fetch/`SystemCommand`
      seam so tests can stub them), verify the SHA256, and abort non-zero on
      mismatch before any service or filesystem change. Extract the tarball.

## 4. Pre-update database snapshot

- [x] 4.1 Add a snapshot seam (e.g. a `Snapshotter` trait or a method on the
      exec abstraction) with a real impl that runs SQLite `VACUUM INTO` against
      the live DB via the already-present `rusqlite` (`bundled`) dependency,
      writing a timestamped file. Use a timestamp passed in / injected so the
      flow stays deterministic under test (no `Utc::now()` inside the seam).
- [x] 4.2 Call the snapshot BEFORE stopping the service or swapping any file. If
      it errors, abort non-zero with context and make no further changes.

## 5. Swap with previous-binary retention

- [x] 5.1 Stop the service via `SystemCommand` (systemd path; reuse the install
      flow's service-control helper).
- [x] 5.2 Place the new binaries: install the new `coterie`/`seed` alongside,
      move the current `coterie` to `coterie.prev` (recording the prior
      `VERSION`), then promote the new binary. Replace `static`/`migrations`
      wholesale. Refresh `.env.example`. Write the new `VERSION`. Never write
      `.env` or the database file.

## 6. Restart, smoke test, and rollback

- [x] 6.1 Start the service via `SystemCommand`, then call the existing
      `install::smoke_test` (or extract it to a shared location if needed) to
      poll `/health` within `SMOKE_TEST_BUDGET`.
- [x] 6.2 On smoke-test success, finish and report the installed version.
- [x] 6.3 On smoke-test failure, restore `coterie.prev`, restart the service,
      and return a non-zero result whose message tells the operator that if a
      migration already ran they may need to restore the pre-update snapshot
      (binary rollback does not undo schema changes).

## 7. Idempotency and config safety

- [x] 7.1 Before downloading, read the deployment's `VERSION`; if it equals the
      resolved target tag, make no changes and return success (no snapshot, no
      service restart).
- [x] 7.2 Confirm in code/review that `.env` and the live database file are
      never written by the update path (snapshot output is a separate file).

## 8. Thin bootstrap + script consolidation

- [x] 8.1 Add `deploy/update.sh` mirroring `deploy/provision.sh`: resolve the
      tag (default latest stable via the GitHub `releases/latest` redirect, no
      `python3`), download the `coterie-provision` binary + checksum, verify,
      and `exec ./coterie-provision update "$@"`. Forward `--tag` verbatim.
- [x] 8.2 Reduce `deploy/release-deploy.sh` to delegate to `update.sh` (or to
      `coterie-provision update`) so there is a single hardened update path.
      Keep its existing positional-tag interface working as a passthrough.

## 9. README

- [x] 9.1 Update the existing `## Update` section in `README.md` (currently
      documenting the interim `release-deploy.sh`) to use the `update.sh`
      bootstrap / `coterie-provision update`. Keep the `--tag` override and the
      "prebuilt release, never build on the host" note, and add that the DB is
      snapshotted automatically before applying.

## 10. Tests

- [x] 10.1 Add unit tests in `version_selector` for: latest-stable selection,
      skipping a prerelease, and exact-tag lookup (hit + miss).
- [x] 10.2 Add a feature-gated integration test `tests/update_flow.rs`
      (`required-features = ["test-support"]`, registered in `Cargo.toml` like
      `install_flow`) that drives `update::run` with fakes and asserts the
      ordered side-effects of the success path: version check → snapshot →
      stop → swap (with `coterie.prev` retained) → start → smoke test → success.
- [x] 10.3 Add a rollback test: a fake whose smoke test fails causes the previous
      binary to be restored, the service restarted, and a non-zero result with
      the snapshot-guidance message.
- [x] 10.4 Add tests for idempotency (target == installed `VERSION` → no
      side-effects) and snapshot-failure-aborts-before-any-change.

## 11. Validation

- [x] 11.1 `cargo build` — clean.
- [x] 11.2 `cargo test -p coterie-provision --features test-support` — all pass,
      including the new unit and `update_flow` tests.
- [x] 11.3 `cargo clippy -p coterie-provision --features test-support -- --deny warnings` — clean.
- [x] 11.4 `cargo fmt --check` — clean.
- [x] 11.5 `bash -n deploy/update.sh` and `bash -n deploy/release-deploy.sh` — clean.
- [x] 11.6 `openspec validate a38-coterie-provision-update` — clean.
