//! Integration tests driving `update::run_with_paths` against the
//! FakeSystem + FakeFs + FakeSnapshotter + FakeReleaseFetcher seams.
//! These exercise the full update orchestration — success, rollback,
//! idempotency, checksum-mismatch, and snapshot-failure — without any
//! real network, process, filesystem, or database access.

use coterie_provision::fs_ops::FileSystem;
use coterie_provision::output::CaptureOutput;
use coterie_provision::system::CommandOutput;
use coterie_provision::test_support::{FakeFs, FakeReleaseFetcher, FakeSnapshotter, FakeSystem};
use coterie_provision::update::{self, UpdateArgs, UpdatePaths};
use std::path::{Path, PathBuf};
use std::time::Duration;

const INSTALL: &str = "/opt/coterie";
const WORK: &str = "/work";
const DB: &str = "/var/lib/coterie/coterie.db";
const NOW: &str = "20260603-120000";
const SHA_FILE: &str = "/work/coterie-v1.1.0-x86_64-linux-musl.tar.gz.sha256";
const HEALTH: &[&str] = &["-fsSL", "http://127.0.0.1:8080/health"];

fn releases_fixture() -> &'static str {
    include_str!("fixtures/github_releases.json")
}

fn paths() -> UpdatePaths {
    UpdatePaths {
        install_dir: PathBuf::from(INSTALL),
        db_path: PathBuf::from(DB),
        work_dir: PathBuf::from(WORK),
    }
}

fn base_args() -> UpdateArgs {
    UpdateArgs {
        tag: None,
        install_dir: Some(PathBuf::from(INSTALL)),
        skip_root_check: true,
        // Keep the smoke-test poll loop fast in tests.
        smoke_test_interval: Some(Duration::from_millis(1)),
        smoke_test_budget: Some(Duration::from_millis(50)),
    }
}

/// Populate the install dir (currently on v1.0.0) and the "downloaded +
/// extracted" stage dir so the swap can proceed against fakes.
fn stage_world(fs: &FakeFs) {
    // Installed state.
    fs.put(Path::new("/opt/coterie/VERSION"), b"v1.0.0\noldsha\n");
    fs.put(Path::new("/opt/coterie/coterie"), b"old-coterie");
    fs.put(Path::new("/opt/coterie/seed"), b"old-seed");
    fs.create_dir_all(Path::new("/opt/coterie/static")).unwrap();
    fs.put(Path::new("/opt/coterie/static/app.css"), b"old-css");
    fs.create_dir_all(Path::new("/opt/coterie/migrations"))
        .unwrap();
    fs.put(Path::new("/opt/coterie/migrations/001.sql"), b"old-mig");
    // Operator config + live data the update must NOT touch.
    fs.put(Path::new("/opt/coterie/.env"), b"SECRET=keep-me");
    fs.put(Path::new("/var/lib/coterie/coterie.db"), b"DBDATA");
    // Installed deploy/ scripts: one stale, one byte-identical to the
    // release, one operator-authored file the release does not carry.
    fs.create_dir_all(Path::new("/opt/coterie/deploy")).unwrap();
    fs.put(Path::new("/opt/coterie/deploy/backup.sh"), b"old-backup");
    fs.put(Path::new("/opt/coterie/deploy/coterie.service"), b"unit");
    fs.put(Path::new("/opt/coterie/deploy/local-only.sh"), b"operator");

    // Staged (post-extraction) release contents.
    fs.put(
        Path::new("/work/coterie-v1.1.0-x86_64-linux-musl/coterie"),
        b"new-coterie",
    );
    fs.put(
        Path::new("/work/coterie-v1.1.0-x86_64-linux-musl/seed"),
        b"new-seed",
    );
    fs.put(
        Path::new("/work/coterie-v1.1.0-x86_64-linux-musl/.env.example"),
        b"NEW_SETTING=1\n",
    );
    fs.put(
        Path::new("/work/coterie-v1.1.0-x86_64-linux-musl/VERSION"),
        b"v1.1.0\nnewsha\n",
    );
    fs.create_dir_all(Path::new("/work/coterie-v1.1.0-x86_64-linux-musl/static"))
        .unwrap();
    fs.put(
        Path::new("/work/coterie-v1.1.0-x86_64-linux-musl/static/app.css"),
        b"new-css",
    );
    fs.create_dir_all(Path::new(
        "/work/coterie-v1.1.0-x86_64-linux-musl/migrations",
    ))
    .unwrap();
    fs.put(
        Path::new("/work/coterie-v1.1.0-x86_64-linux-musl/migrations/001.sql"),
        b"new-mig",
    );
    // The release's deploy/: backup.sh changed, restore.sh + the timer
    // are new, coterie.service is byte-identical to the installed one.
    // coterie-provision/ is the wizard's source tree, which ships in the
    // tarball but is not an ops script.
    fs.create_dir_all(Path::new("/work/coterie-v1.1.0-x86_64-linux-musl/deploy"))
        .unwrap();
    fs.put(
        Path::new("/work/coterie-v1.1.0-x86_64-linux-musl/deploy/backup.sh"),
        b"new-backup",
    );
    fs.put(
        Path::new("/work/coterie-v1.1.0-x86_64-linux-musl/deploy/restore.sh"),
        b"new-restore",
    );
    fs.put(
        Path::new("/work/coterie-v1.1.0-x86_64-linux-musl/deploy/coterie-backup.timer"),
        b"timer-unit",
    );
    fs.put(
        Path::new("/work/coterie-v1.1.0-x86_64-linux-musl/deploy/coterie.service"),
        b"unit",
    );
    fs.create_dir_all(Path::new(
        "/work/coterie-v1.1.0-x86_64-linux-musl/deploy/coterie-provision",
    ))
    .unwrap();
    fs.put(
        Path::new("/work/coterie-v1.1.0-x86_64-linux-musl/deploy/coterie-provision/Cargo.toml"),
        b"[package]",
    );

    // The "downloaded" checksum file (curl is faked, so we stage it).
    fs.put(
        Path::new(SHA_FILE),
        b"deadbeefcafe  coterie-v1.1.0-x86_64-linux-musl.tar.gz\n",
    );
}

/// Make `sha256sum <tarball>` report the matching hash so verification
/// passes. Args carry a dynamic temp path, so match by command name.
fn checksum_matches(sys: &FakeSystem) {
    sys.respond_to_cmd(
        "sha256sum",
        CommandOutput {
            status: 0,
            stdout: "deadbeefcafe  /work/coterie-v1.1.0-x86_64-linux-musl.tar.gz\n".to_string(),
            stderr: String::new(),
        },
    );
}

/// Index of the first recorded call to `cmd` (optionally requiring an
/// arg substring). Panics if not found.
fn first_call_index(sys: &FakeSystem, cmd: &str, arg_contains: Option<&str>) -> usize {
    sys.calls
        .borrow()
        .iter()
        .position(|c| {
            c.cmd == cmd
                && arg_contains
                    .map(|needle| c.args.iter().any(|a| a.contains(needle)))
                    .unwrap_or(true)
        })
        .unwrap_or_else(|| panic!("expected a `{cmd}` call (arg ~ {arg_contains:?})"))
}

// ---------------------------------------------------------------------
// 10.2 — success path, ordered side-effects
// ---------------------------------------------------------------------

#[test]
fn success_path_orders_snapshot_stop_swap_start_smoke() {
    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    let snap = FakeSnapshotter::new();
    let fetch = FakeReleaseFetcher::new(releases_fixture());
    let out = CaptureOutput::new();
    stage_world(&fs);
    checksum_matches(&sys);

    let code = update::run_with_paths(base_args(), &sys, &fs, &snap, &fetch, &out, &paths(), NOW)
        .expect("update should succeed");
    assert_eq!(code, 0, "success exits 0");

    // Snapshot was taken exactly once, to the timestamped destination.
    assert_eq!(snap.call_count(), 1, "exactly one pre-update snapshot");
    let (db, dest) = snap.calls.borrow()[0].clone();
    assert_eq!(db, PathBuf::from(DB));
    assert_eq!(
        dest,
        PathBuf::from("/var/lib/coterie/coterie-pre-update-20260603-120000.db")
    );

    // Previous binary retained; new binary promoted.
    assert_eq!(
        fs.get(Path::new("/opt/coterie/coterie.prev")).as_deref(),
        Some(&b"old-coterie"[..]),
        "previous binary retained as coterie.prev"
    );
    assert_eq!(
        fs.get(Path::new("/opt/coterie/coterie")).as_deref(),
        Some(&b"new-coterie"[..]),
        "new binary promoted into place"
    );

    // static/migrations replaced wholesale; VERSION + .env.example refreshed.
    assert_eq!(
        fs.get(Path::new("/opt/coterie/static/app.css")).as_deref(),
        Some(&b"new-css"[..])
    );
    assert_eq!(
        fs.get(Path::new("/opt/coterie/migrations/001.sql"))
            .as_deref(),
        Some(&b"new-mig"[..])
    );
    assert_eq!(
        fs.get(Path::new("/opt/coterie/VERSION")).as_deref(),
        Some(&b"v1.1.0\nnewsha\n"[..])
    );
    assert_eq!(
        fs.get(Path::new("/opt/coterie/.env.example")).as_deref(),
        Some(&b"NEW_SETTING=1\n"[..])
    );

    // The ordered systemd dance: verify+extract precede stop, stop
    // precedes start, start precedes the /health smoke test.
    let i_sha = first_call_index(&sys, "sha256sum", None);
    let i_tar = first_call_index(&sys, "tar", None);
    let i_stop = first_call_index(&sys, "systemctl", Some("stop"));
    let i_start = first_call_index(&sys, "systemctl", Some("start"));
    let i_health = first_call_index(&sys, "curl", Some("/health"));
    assert!(
        i_sha < i_stop,
        "checksum verified before the service is stopped"
    );
    assert!(
        i_tar < i_stop,
        "tarball extracted before the service is stopped"
    );
    assert!(
        i_stop < i_start,
        "service stopped before it is started again"
    );
    assert!(i_start < i_health, "service started before the smoke test");
}

#[test]
fn success_path_never_writes_env_or_database() {
    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    let snap = FakeSnapshotter::new();
    let fetch = FakeReleaseFetcher::new(releases_fixture());
    let out = CaptureOutput::new();
    stage_world(&fs);
    checksum_matches(&sys);

    update::run_with_paths(base_args(), &sys, &fs, &snap, &fetch, &out, &paths(), NOW)
        .expect("update should succeed");

    // .env and the live DB are byte-for-byte unchanged.
    assert_eq!(
        fs.get(Path::new("/opt/coterie/.env")).as_deref(),
        Some(&b"SECRET=keep-me"[..]),
        "live .env must be untouched"
    );
    assert_eq!(
        fs.get(Path::new("/var/lib/coterie/coterie.db")).as_deref(),
        Some(&b"DBDATA"[..]),
        "live database file must be untouched"
    );

    // And no fs op ever targeted .env or the DB file.
    use coterie_provision::test_support::FsOp;
    let touched_protected = fs.ops.borrow().iter().any(|op| {
        let p = match op {
            FsOp::Write(p, _)
            | FsOp::Append(p, _)
            | FsOp::Chmod(p, _)
            | FsOp::Chown(p, _, _)
            | FsOp::RemoveFile(p)
            | FsOp::RemoveDirAll(p) => Some(p.clone()),
            FsOp::Rename(_, to) => Some(to.clone()),
            FsOp::CopyFile(_, to) => Some(to.clone()),
            FsOp::CopyDirAll(_, to) => Some(to.clone()),
            _ => None,
        };
        p.map(|p| p == Path::new("/opt/coterie/.env") || p == Path::new(DB))
            .unwrap_or(false)
    });
    assert!(
        !touched_protected,
        "no write/chmod/chown/remove targeted .env or the DB"
    );
}

// ---------------------------------------------------------------------
// 10.3 — rollback on smoke-test failure
// ---------------------------------------------------------------------

#[test]
fn unhealthy_after_restart_rolls_back_to_previous_binary() {
    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    let snap = FakeSnapshotter::new();
    let fetch = FakeReleaseFetcher::new(releases_fixture());
    let out = CaptureOutput::new();
    stage_world(&fs);
    checksum_matches(&sys);
    // The /health smoke test always returns an HTTP error (curl exit 22).
    sys.respond_to(
        "curl",
        HEALTH,
        CommandOutput {
            status: 22,
            stdout: String::new(),
            stderr: "curl: (22) 500".to_string(),
        },
    );

    let err = update::run_with_paths(base_args(), &sys, &fs, &snap, &fetch, &out, &paths(), NOW)
        .expect_err("a failed smoke test must surface an error");

    // The previous binary is restored over the bad new one.
    assert_eq!(
        fs.get(Path::new("/opt/coterie/coterie")).as_deref(),
        Some(&b"old-coterie"[..]),
        "rollback restores the previous binary"
    );
    // VERSION reverted to the previous tag.
    assert_eq!(
        fs.get(Path::new("/opt/coterie/VERSION")).as_deref(),
        Some(&b"v1.0.0\n"[..]),
        "rollback restores the previous VERSION"
    );
    // The service was restarted on the previous binary.
    assert!(
        sys.called_with("systemctl", &["restart", "coterie"]),
        "service restarted after rollback"
    );
    // Rollback is a binary+VERSION operation: the refreshed scripts stay
    // put, with the pre-update copy still beside them for an operator who
    // wants the old one back.
    assert_eq!(
        fs.get(Path::new("/opt/coterie/deploy/backup.sh.prev"))
            .as_deref(),
        Some(&b"old-backup"[..]),
        "the pre-update backup.sh remains recoverable after a rollback"
    );
    // Operator guidance mentions the snapshot + migration caveat.
    let msg = format!("{err:#}");
    assert!(
        msg.contains("snapshot"),
        "guidance must mention the snapshot; got: {msg}"
    );
    assert!(
        msg.contains("migration"),
        "guidance must mention migrations; got: {msg}"
    );
    assert!(
        msg.contains("coterie-pre-update-20260603-120000.db"),
        "guidance must point at the snapshot file; got: {msg}"
    );
}

// ---------------------------------------------------------------------
// 10.4 — idempotency + snapshot-failure-aborts
// ---------------------------------------------------------------------

#[test]
fn already_on_target_is_a_no_op() {
    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    let snap = FakeSnapshotter::new();
    let fetch = FakeReleaseFetcher::new(releases_fixture());
    let out = CaptureOutput::new();
    stage_world(&fs);
    // Pretend we're already on the latest stable (v1.1.0).
    fs.put(Path::new("/opt/coterie/VERSION"), b"v1.1.0\nsomesha\n");
    checksum_matches(&sys);

    let code = update::run_with_paths(base_args(), &sys, &fs, &snap, &fetch, &out, &paths(), NOW)
        .expect("idempotent run returns success");

    assert_eq!(code, 0, "already-on-target exits 0");
    assert_eq!(snap.call_count(), 0, "no snapshot when already on target");
    assert_eq!(
        sys.calls.borrow().len(),
        0,
        "no system commands (no stop/start/download) when already on target"
    );
    // The early return wins over every placement step, scripts included.
    assert_eq!(
        fs.get(Path::new("/opt/coterie/deploy/backup.sh"))
            .as_deref(),
        Some(&b"old-backup"[..]),
        "already-on-target refreshes no deploy script"
    );
    assert!(fs
        .get(Path::new("/opt/coterie/deploy/restore.sh"))
        .is_none());
    assert!(out.contains("Already on v1.1.0"));
}

#[test]
fn snapshot_failure_aborts_before_any_change() {
    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    let snap = FakeSnapshotter::new();
    let fetch = FakeReleaseFetcher::new(releases_fixture());
    let out = CaptureOutput::new();
    stage_world(&fs);
    checksum_matches(&sys);
    snap.fail_next();

    let err = update::run_with_paths(base_args(), &sys, &fs, &snap, &fetch, &out, &paths(), NOW)
        .expect_err("snapshot failure must abort the update");

    assert_eq!(snap.call_count(), 1, "the snapshot was attempted");
    // Service was never stopped and the binary was never swapped.
    assert!(
        !sys.called_with("systemctl", &["stop", "coterie"]),
        "must not stop the service when the snapshot fails"
    );
    assert!(
        fs.get(Path::new("/opt/coterie/coterie.prev")).is_none(),
        "no binary swap when the snapshot fails"
    );
    assert_eq!(
        fs.get(Path::new("/opt/coterie/coterie")).as_deref(),
        Some(&b"old-coterie"[..]),
        "binary left untouched when the snapshot fails"
    );
    assert!(format!("{err:#}").contains("snapshot"));
}

#[test]
fn checksum_mismatch_aborts_before_snapshot_or_stop() {
    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    let snap = FakeSnapshotter::new();
    let fetch = FakeReleaseFetcher::new(releases_fixture());
    let out = CaptureOutput::new();
    stage_world(&fs);
    // sha256sum reports a different hash than the staged .sha256 file.
    sys.respond_to_cmd(
        "sha256sum",
        CommandOutput {
            status: 0,
            stdout: "00000000ffff  /work/coterie-v1.1.0-x86_64-linux-musl.tar.gz\n".to_string(),
            stderr: String::new(),
        },
    );

    let err = update::run_with_paths(base_args(), &sys, &fs, &snap, &fetch, &out, &paths(), NOW)
        .expect_err("a checksum mismatch must abort the update");

    assert_eq!(
        snap.call_count(),
        0,
        "no snapshot taken on checksum mismatch"
    );
    assert!(
        !sys.called_with("systemctl", &["stop", "coterie"]),
        "service not stopped on checksum mismatch"
    );
    assert!(
        fs.get(Path::new("/opt/coterie/coterie.prev")).is_none(),
        "no swap on checksum mismatch"
    );
    let msg = format!("{err:#}");
    assert!(
        msg.contains("checksum"),
        "error must mention the checksum; got: {msg}"
    );
}

// ---------------------------------------------------------------------
// Release resolution: explicit tag + unknown tag
// ---------------------------------------------------------------------

#[test]
fn explicit_tag_is_honored() {
    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    let snap = FakeSnapshotter::new();
    let fetch = FakeReleaseFetcher::new(releases_fixture());
    let out = CaptureOutput::new();
    stage_world(&fs);
    // Already on v1.0.0; pinning --tag v1.0.0 must be a no-op even though
    // v1.1.0 is the latest stable.
    let mut args = base_args();
    args.tag = Some("v1.0.0".to_string());

    let code = update::run_with_paths(args, &sys, &fs, &snap, &fetch, &out, &paths(), NOW)
        .expect("pinned tag resolves");
    assert_eq!(code, 0);
    assert_eq!(snap.call_count(), 0, "pinned tag equals installed → no-op");
    assert!(out.contains("Already on v1.0.0"));
}

#[test]
fn unknown_tag_errors_clearly() {
    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    let snap = FakeSnapshotter::new();
    let fetch = FakeReleaseFetcher::new(releases_fixture());
    let out = CaptureOutput::new();
    stage_world(&fs);
    let mut args = base_args();
    args.tag = Some("v9.9.9".to_string());

    let err = update::run_with_paths(args, &sys, &fs, &snap, &fetch, &out, &paths(), NOW)
        .expect_err("an unknown tag must error");
    assert!(format!("{err:#}").contains("v9.9.9"));
    assert_eq!(snap.call_count(), 0);
}

// ---------------------------------------------------------------------
// a53 — update refreshes the bundled deployment scripts
// ---------------------------------------------------------------------

/// Run a successful update over the standard staged world and hand back
/// the fakes for assertion.
fn updated_world() -> (FakeSystem, FakeFs, CaptureOutput) {
    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    let snap = FakeSnapshotter::new();
    let fetch = FakeReleaseFetcher::new(releases_fixture());
    let out = CaptureOutput::new();
    stage_world(&fs);
    checksum_matches(&sys);
    // Staging the world uses the same seam, so drop its ops — the
    // assertions below are about what the update itself did.
    fs.ops.borrow_mut().clear();
    update::run_with_paths(base_args(), &sys, &fs, &snap, &fetch, &out, &paths(), NOW)
        .expect("update should succeed");
    (sys, fs, out)
}

#[test]
fn changed_script_is_refreshed_and_the_previous_one_retained() {
    let (_sys, fs, out) = updated_world();

    assert_eq!(
        fs.get(Path::new("/opt/coterie/deploy/backup.sh"))
            .as_deref(),
        Some(&b"new-backup"[..]),
        "deploy/backup.sh is the release's version after an update"
    );
    assert_eq!(
        fs.get(Path::new("/opt/coterie/deploy/backup.sh.prev"))
            .as_deref(),
        Some(&b"old-backup"[..]),
        "the replaced script is retained beside the new one"
    );
    assert!(
        out.contains("deploy/backup.sh (previous kept as deploy/backup.sh.prev)"),
        "the .prev is called out on that file's line; got:\n{}",
        out.joined()
    );
}

#[test]
fn identical_script_is_not_rewritten_and_is_not_reported() {
    let (_sys, fs, out) = updated_world();

    assert!(
        fs.get(Path::new("/opt/coterie/deploy/coterie.service.prev"))
            .is_none(),
        "a byte-identical script produces no .prev"
    );
    assert!(
        !out.contains("coterie.service"),
        "an unchanged script is not reported; got:\n{}",
        out.joined()
    );
    // ...and it was never rewritten, so repeated updates do not churn.
    use coterie_provision::test_support::FsOp;
    let rewritten = fs.ops.borrow().iter().any(|op| {
        matches!(op, FsOp::CopyFile(_, to) | FsOp::Write(to, _)
            if to == Path::new("/opt/coterie/deploy/coterie.service"))
    });
    assert!(!rewritten, "an unchanged script is not rewritten");
}

#[test]
fn script_new_to_the_release_is_created_and_reported() {
    let (_sys, fs, out) = updated_world();

    assert_eq!(
        fs.get(Path::new("/opt/coterie/deploy/restore.sh"))
            .as_deref(),
        Some(&b"new-restore"[..]),
        "a script the deployment lacks is created"
    );
    assert!(
        fs.get(Path::new("/opt/coterie/deploy/restore.sh.prev"))
            .is_none(),
        "nothing was replaced, so no .prev"
    );
    assert!(
        out.contains("deploy/restore.sh (new)"),
        "the new script is named in the output; got:\n{}",
        out.joined()
    );
}

#[test]
fn operator_authored_script_absent_from_the_release_survives() {
    let (_sys, fs, _out) = updated_world();

    assert_eq!(
        fs.get(Path::new("/opt/coterie/deploy/local-only.sh"))
            .as_deref(),
        Some(&b"operator"[..]),
        "a file only the operator has is left alone — this is what a \
         wholesale directory replace would have deleted"
    );
    // The wizard's source tree ships in the tarball's deploy/ but is not
    // an ops script; only top-level files are placed.
    assert!(
        fs.get(Path::new(
            "/opt/coterie/deploy/coterie-provision/Cargo.toml"
        ))
        .is_none(),
        "subdirectories of the release's deploy/ are not copied"
    );
}

#[test]
fn shell_scripts_keep_their_executable_bit() {
    let (_sys, fs, _out) = updated_world();

    use coterie_provision::test_support::FsOp;
    let chmodded = |p: &str| {
        fs.ops
            .borrow()
            .iter()
            .any(|op| matches!(op, FsOp::Chmod(path, 0o755) if path == Path::new(p)))
    };
    assert!(
        chmodded("/opt/coterie/deploy/backup.sh"),
        "backup.sh is 0755"
    );
    assert!(
        chmodded("/opt/coterie/deploy/restore.sh"),
        "restore.sh is 0755"
    );
}

#[test]
fn update_never_activates_a_unit_or_writes_outside_the_install_dir() {
    let (sys, fs, _out) = updated_world();

    // The refreshed coterie-backup.timer landed inside the install dir...
    assert_eq!(
        fs.get(Path::new("/opt/coterie/deploy/coterie-backup.timer"))
            .as_deref(),
        Some(&b"timer-unit"[..])
    );
    // ...and nothing enabled, reloaded, or installed it. The only
    // systemctl verbs an update may use are the service stop/start it
    // already performs.
    for call in sys.calls.borrow().iter() {
        if call.cmd == "systemctl" {
            assert!(
                matches!(
                    call.args.first().map(String::as_str),
                    Some("stop") | Some("start") | Some("restart")
                ),
                "update must not run `systemctl {}`",
                call.args.join(" ")
            );
        }
        assert!(
            !call.args.iter().any(|a| a.contains("/etc/systemd/system")),
            "update must not touch /etc/systemd/system: {} {}",
            call.cmd,
            call.args.join(" ")
        );
    }

    // Every filesystem mutation targets the install dir.
    use coterie_provision::test_support::FsOp;
    for op in fs.ops.borrow().iter() {
        let dest = match op {
            FsOp::Write(p, _)
            | FsOp::Append(p, _)
            | FsOp::CreateDirAll(p)
            | FsOp::Chmod(p, _)
            | FsOp::Chown(p, _, _)
            | FsOp::RemoveFile(p)
            | FsOp::RemoveDirAll(p) => Some(p.clone()),
            FsOp::Rename(_, to) | FsOp::CopyFile(_, to) | FsOp::CopyDirAll(_, to) => {
                Some(to.clone())
            }
            FsOp::Read(_) => None,
        };
        if let Some(dest) = dest {
            assert!(
                dest.starts_with(INSTALL),
                "update wrote outside {INSTALL}: {}",
                dest.display()
            );
        }
    }
}
