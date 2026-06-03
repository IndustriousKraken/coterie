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
