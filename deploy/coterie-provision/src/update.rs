//! `coterie-provision update` — hardened in-place update of an installed
//! Coterie instance.
//!
//! See openspec change `a38` for the full spec. Summary of the ordered
//! steps this module performs:
//!
//! 1. Resolve the target tag: latest **stable** release by default, or
//!    the exact `--tag` if supplied (rollback / pinned version).
//! 2. Idempotency: if the target already equals the installed
//!    `VERSION`, exit `0` without taking a snapshot or touching the
//!    service.
//! 3. Download the prebuilt release tarball + its `.sha256`, verify the
//!    checksum, and abort before any change on mismatch. Never compiles
//!    on the host.
//! 4. Take a pre-update `VACUUM INTO` database snapshot BEFORE stopping
//!    the service or swapping any file. Abort on snapshot failure.
//! 5. Stop the service, place the new binaries (retaining the previous
//!    `coterie` as `coterie.prev`), replace `static`/`migrations`
//!    wholesale, refresh `.env.example` and the bundled `deploy/` ops
//!    scripts (each replaced file retained as `<name>.prev`), and write
//!    the new `VERSION`. Never writes `.env` or the live database, and
//!    never installs, enables, or reloads a systemd unit.
//! 6. Start the service and run the `/health` smoke test. On failure,
//!    restore `coterie.prev`, restart, and exit non-zero with guidance
//!    that the pre-update snapshot may need restoring if a migration
//!    already ran.
//!
//! Every side effect routes through the `SystemCommand` / `FileSystem` /
//! `ReleaseFetcher` / `Snapshotter` seams so the full flow — including
//! rollback — is exercisable with fakes and no real network, process,
//! filesystem, or database access.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::fs_ops::FileSystem;
use crate::install::{self, SMOKE_TEST_BUDGET, SMOKE_TEST_INTERVAL};
use crate::output::Output;
use crate::system::SystemCommand;
use crate::version_selector;

pub const REPO: &str = "IndustriousKraken/coterie";
pub const DEFAULT_INSTALL_DIR: &str = "/opt/coterie";
pub const DEFAULT_DB_PATH: &str = "/var/lib/coterie/coterie.db";
/// Target-triple suffix of the published Coterie release tarball. Matches
/// the asset name produced by `.github/workflows/release.yml`
/// (`coterie-<tag>-x86_64-linux-musl.tar.gz`) and consumed by
/// `release-deploy.sh`.
const TARGET_TRIPLE_SUFFIX: &str = "x86_64-linux-musl";

// --------------------------------------------------------------------------
// Seams
// --------------------------------------------------------------------------

/// Seam for the pre-update database snapshot. The real impl runs SQLite
/// `VACUUM INTO` in-process via `rusqlite`; tests stub it so the flow is
/// driven without a real database. The destination filename (timestamp
/// and all) is computed by the caller, so the seam itself never reads
/// the clock and the flow stays deterministic under test.
pub trait Snapshotter {
    fn snapshot(&self, db_path: &Path, dest: &Path) -> Result<()>;
}

pub struct RealSnapshotter;

impl RealSnapshotter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RealSnapshotter {
    fn default() -> Self {
        Self::new()
    }
}

impl Snapshotter for RealSnapshotter {
    fn snapshot(&self, db_path: &Path, dest: &Path) -> Result<()> {
        // Open read-write but NOT create: an update is for an existing
        // instance, so a missing DB is an error rather than a reason to
        // snapshot an empty file.
        use rusqlite::OpenFlags;
        let conn =
            rusqlite::Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_WRITE)
                .with_context(|| format!("opening live database {}", db_path.display()))?;
        // `VACUUM INTO` writes a single self-contained snapshot file in
        // one atomic SQLite op (no WAL siblings), matching deploy/backup.sh.
        let dest_str = dest.to_string_lossy().replace('\'', "''");
        conn.execute_batch(&format!("VACUUM INTO '{dest_str}'"))
            .with_context(|| format!("VACUUM INTO {}", dest.display()))?;
        Ok(())
    }
}

/// Seam for fetching the GitHub releases list used to resolve the target
/// tag. The real impl delegates to [`crate::github_api::fetch_releases`];
/// tests return a fixture without going over the network.
pub trait ReleaseFetcher {
    fn fetch_releases(&self, repo: &str) -> Result<String>;
}

pub struct RealReleaseFetcher;

impl RealReleaseFetcher {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RealReleaseFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl ReleaseFetcher for RealReleaseFetcher {
    fn fetch_releases(&self, repo: &str) -> Result<String> {
        crate::github_api::fetch_releases(repo)
    }
}

// --------------------------------------------------------------------------
// Inputs / paths
// --------------------------------------------------------------------------

/// Parsed CLI inputs (independent of clap). `main.rs` converts the clap
/// struct into this.
#[derive(Debug, Clone, Default)]
pub struct UpdateArgs {
    /// Pin an exact release tag. `None` → latest stable.
    pub tag: Option<String>,
    /// Override the install dir (default `/opt/coterie`). Tests point
    /// this at a temp dir.
    pub install_dir: Option<PathBuf>,
    /// Test-only escape hatch. The CLI never sets this. Production always
    /// enforces the root check.
    #[doc(hidden)]
    pub skip_root_check: bool,
    /// Test-only override for the smoke-test per-iteration sleep.
    #[doc(hidden)]
    pub smoke_test_interval: Option<Duration>,
    /// Test-only override for the smoke-test total budget.
    #[doc(hidden)]
    pub smoke_test_budget: Option<Duration>,
}

/// Filesystem paths the update touches. Production uses the well-known
/// `/opt/coterie` + `/var/lib/coterie` paths plus a freshly-created temp
/// dir; tests inject all three so the flow runs against fakes.
#[derive(Debug, Clone)]
pub struct UpdatePaths {
    pub install_dir: PathBuf,
    pub db_path: PathBuf,
    pub work_dir: PathBuf,
}

extern "C" {
    fn geteuid() -> u32;
}

fn is_root() -> bool {
    // SAFETY: geteuid is a thread-safe POSIX call with no preconditions.
    unsafe { geteuid() == 0 }
}

// --------------------------------------------------------------------------
// Entry points
// --------------------------------------------------------------------------

/// Production entry point. Wires the real seams, creates a temp work dir
/// for the download, stamps the snapshot timestamp, and delegates to
/// [`run_with_paths`]. Returns an exit-code-friendly `Result` (Ok(0) for
/// success-or-already-done, Err for failures/rollback).
pub fn run<S, F, Sn, Fe, O>(
    args: UpdateArgs,
    sys: &S,
    fs: &F,
    snap: &Sn,
    fetch: &Fe,
    output: &O,
) -> Result<i32>
where
    S: SystemCommand,
    F: FileSystem,
    Sn: Snapshotter,
    Fe: ReleaseFetcher,
    O: Output,
{
    let install_dir = args
        .install_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_INSTALL_DIR));
    let work = tempfile::Builder::new()
        .prefix("coterie-update-")
        .tempdir()
        .context("creating temp work dir for the update download")?;
    let paths = UpdatePaths {
        install_dir,
        db_path: PathBuf::from(DEFAULT_DB_PATH),
        work_dir: work.path().to_path_buf(),
    };
    let now = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    run_with_paths(args, sys, fs, snap, fetch, output, &paths, &now)
    // `work` (and its contents) is removed when this scope ends.
}

/// Testable core. Production calls [`run`] (which supplies a real work
/// dir + timestamp); tests call this directly with injected paths and a
/// fixed `now` string.
#[allow(clippy::too_many_arguments)]
pub fn run_with_paths<S, F, Sn, Fe, O>(
    args: UpdateArgs,
    sys: &S,
    fs: &F,
    snap: &Sn,
    fetch: &Fe,
    output: &O,
    paths: &UpdatePaths,
    now: &str,
) -> Result<i32>
where
    S: SystemCommand,
    F: FileSystem,
    Sn: Snapshotter,
    Fe: ReleaseFetcher,
    O: Output,
{
    if !args.skip_root_check && !is_root() {
        return Err(anyhow!(
            "coterie-provision update must run as root (try sudo)"
        ));
    }

    // --- 1. Resolve target tag ---------------------------------------
    let json = fetch
        .fetch_releases(REPO)
        .context("fetching the release list from GitHub")?;
    let releases = version_selector::parse_releases(&json)?;
    let target_tag = match args.tag.as_deref() {
        Some(tag) => {
            // Validate the tag actually exists so we fail clearly here
            // rather than on a 404 mid-download.
            version_selector::find_by_tag(&releases, tag).ok_or_else(|| {
                anyhow!("requested tag `{tag}` was not found among the published releases")
            })?;
            tag.to_string()
        }
        None => version_selector::select_default_stable(&releases)
            .ok_or_else(|| anyhow!("no stable (non-prerelease) release found to update to"))?
            .tag_name
            .clone(),
    };
    output.println(&format!("Update target resolved to {target_tag}."));

    // --- 2. Idempotency (before downloading anything) ----------------
    let version_path = paths.install_dir.join("VERSION");
    let installed = read_installed_version(fs, &version_path);
    if installed.as_deref() == Some(target_tag.as_str()) {
        output.println(&format!("Already on {target_tag}; nothing to do."));
        return Ok(0);
    }

    // --- 3. Download the prebuilt artifact + checksum ----------------
    let stage_name = format!("coterie-{target_tag}-{TARGET_TRIPLE_SUFFIX}");
    let tarball_name = format!("{stage_name}.tar.gz");
    let sha_name = format!("{tarball_name}.sha256");
    let base_url = format!("https://github.com/{REPO}/releases/download/{target_tag}");
    let tarball_url = format!("{base_url}/{tarball_name}");
    let sha_url = format!("{base_url}/{sha_name}");
    let tarball_path = paths.work_dir.join(&tarball_name);
    let sha_path = paths.work_dir.join(&sha_name);

    download(sys, &tarball_url, &tarball_path, output)?;
    download(sys, &sha_url, &sha_path, output)?;

    // --- 4. Verify checksum (abort before any service/file change) ---
    verify_checksum(sys, fs, &tarball_path, &sha_path).context("verifying release checksum")?;
    output.println("Checksum verified.");

    // --- 5. Extract --------------------------------------------------
    extract(sys, &tarball_path, &paths.work_dir, output)?;
    let stage_dir = paths.work_dir.join(&stage_name);

    // --- 6. Pre-update snapshot, BEFORE stop/swap --------------------
    let snapshot_dest = snapshot_dest_path(&paths.db_path, now);
    output.println(&format!(
        "Snapshotting database {} -> {}",
        paths.db_path.display(),
        snapshot_dest.display()
    ));
    snap.snapshot(&paths.db_path, &snapshot_dest).with_context(|| {
        format!(
            "pre-update database snapshot to {} failed; aborting before stopping the service or swapping any file",
            snapshot_dest.display()
        )
    })?;

    // --- 7. Stop the service -----------------------------------------
    output.println("Stopping coterie service...");
    run_checked(
        sys,
        "systemctl",
        &["stop", "coterie"],
        "systemctl stop coterie",
    )?;

    // --- 8. Swap binaries, retaining the previous one ----------------
    swap_binaries(fs, &stage_dir, &paths.install_dir, &target_tag, output)?;

    // --- 9. Restart, smoke test, rollback on failure -----------------
    let interval = args.smoke_test_interval.unwrap_or(SMOKE_TEST_INTERVAL);
    let budget = args.smoke_test_budget.unwrap_or(SMOKE_TEST_BUDGET);
    output.println("Starting coterie service...");
    let start_out = sys.run("systemctl", &["start", "coterie"])?;
    let health = if start_out.success() {
        output.println(&format!(
            "Service started; smoke testing /health (polling up to {}s)...",
            budget.as_secs()
        ));
        install::smoke_test(sys, interval, budget)
    } else {
        Err(anyhow!(
            "systemctl start coterie failed (exit {}): {}",
            start_out.status,
            start_out.stderr
        ))
    };

    match health {
        Ok(()) => {
            output.println(&format!(
                "Update complete. Coterie is now running {target_tag}."
            ));
            Ok(0)
        }
        Err(e) => {
            rollback(fs, sys, &paths.install_dir, installed.as_deref(), output);
            Err(anyhow!(
                "update to {target_tag} failed its post-restart health check ({e}); \
                 restored the previous binary and restarted the service.\n\
                 IMPORTANT: binary rollback does NOT undo schema changes. If a database \
                 migration already ran under {target_tag}, restore the pre-update snapshot at \
                 {} (copy it back over {}) before the instance can be considered recovered.",
                snapshot_dest.display(),
                paths.db_path.display()
            ))
        }
    }
}

// --------------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------------

/// Read the first non-empty line of the install dir's `VERSION` file.
/// The release tarball's `VERSION` is `<tag>\n<sha>`, so the first line
/// is the installed tag.
fn read_installed_version<F: FileSystem>(fs: &F, version_path: &Path) -> Option<String> {
    if !fs.is_file(version_path) {
        return None;
    }
    fs.read_to_string(version_path)
        .ok()
        .and_then(|s| s.lines().next().map(|l| l.trim().to_string()))
        .filter(|s| !s.is_empty())
}

/// Build the timestamped snapshot destination alongside the live DB.
fn snapshot_dest_path(db_path: &Path, now: &str) -> PathBuf {
    let dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    dir.join(format!("coterie-pre-update-{now}.db"))
}

fn download<S: SystemCommand, O: Output>(
    sys: &S,
    url: &str,
    dest: &Path,
    output: &O,
) -> Result<()> {
    output.println(&format!("Downloading {url}"));
    let dest_str = dest.to_string_lossy();
    let out = sys.run("curl", &["-sfL", "-o", dest_str.as_ref(), url])?;
    if !out.success() {
        return Err(anyhow!(
            "download of {url} failed (curl exit {}): {}",
            out.status,
            out.stderr
        ));
    }
    Ok(())
}

/// Verify the tarball against its published `.sha256`. Parses the
/// expected hash from the checksum file (first whitespace token) and
/// compares it to `sha256sum <tarball>`'s output, avoiding any
/// working-directory dependence.
fn verify_checksum<S, F>(sys: &S, fs: &F, tarball: &Path, sha_file: &Path) -> Result<()>
where
    S: SystemCommand,
    F: FileSystem,
{
    let recorded = fs
        .read_to_string(sha_file)
        .with_context(|| format!("reading checksum file {}", sha_file.display()))?;
    let expected = recorded
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow!("checksum file {} was empty", sha_file.display()))?;
    let tarball_str = tarball.to_string_lossy();
    let out = sys.run("sha256sum", &[tarball_str.as_ref()])?;
    if !out.success() {
        return Err(anyhow!(
            "sha256sum failed (exit {}): {}",
            out.status,
            out.stderr
        ));
    }
    let actual = out
        .stdout
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow!("sha256sum produced no output for {}", tarball.display()))?;
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(anyhow!(
            "checksum mismatch for {}: expected {expected}, computed {actual}",
            tarball.display()
        ));
    }
    Ok(())
}

fn extract<S: SystemCommand, O: Output>(
    sys: &S,
    tarball: &Path,
    into: &Path,
    output: &O,
) -> Result<()> {
    output.println(&format!("Extracting {}", tarball.display()));
    let tb = tarball.to_string_lossy();
    let into_s = into.to_string_lossy();
    let out = sys.run("tar", &["xzf", tb.as_ref(), "-C", into_s.as_ref()])?;
    if !out.success() {
        return Err(anyhow!(
            "tar extraction of {} failed (exit {}): {}",
            tarball.display(),
            out.status,
            out.stderr
        ));
    }
    Ok(())
}

fn run_checked<S: SystemCommand>(sys: &S, cmd: &str, args: &[&str], desc: &str) -> Result<()> {
    let out = sys.run(cmd, args)?;
    if !out.success() {
        return Err(anyhow!(
            "{desc} failed (exit {}): {}\n{}",
            out.status,
            out.stdout,
            out.stderr
        ));
    }
    Ok(())
}

/// Place the new binaries (retaining the prior `coterie` as
/// `coterie.prev`), replace `static`/`migrations` wholesale, refresh
/// `.env.example` and `deploy/`, and record the new `VERSION`. Never
/// writes `.env` or the live database file.
fn swap_binaries<F: FileSystem, O: Output>(
    fs: &F,
    stage_dir: &Path,
    install_dir: &Path,
    target_tag: &str,
    output: &O,
) -> Result<()> {
    let coterie = install_dir.join("coterie");
    let coterie_new = install_dir.join("coterie.new");
    let coterie_prev = install_dir.join("coterie.prev");
    let seed = install_dir.join("seed");
    let seed_new = install_dir.join("seed.new");

    // Place the new binaries alongside first. copy_file (not rename) is
    // cross-device-safe — the work dir may be on a different mount.
    fs.copy_file(&stage_dir.join("coterie"), &coterie_new)
        .context("staging the new coterie binary")?;
    fs.chmod(&coterie_new, 0o755)?;
    if fs.is_file(&stage_dir.join("seed")) {
        fs.copy_file(&stage_dir.join("seed"), &seed_new)
            .context("staging the new seed binary")?;
        fs.chmod(&seed_new, 0o755)?;
    }

    // Retain the currently-installed coterie binary for instant,
    // offline rollback.
    if fs.is_file(&coterie) {
        fs.rename(&coterie, &coterie_prev)
            .context("retaining the previous coterie binary as coterie.prev")?;
        output.println(&format!(
            "Retained previous binary as {}",
            coterie_prev.display()
        ));
    }

    // Promote the staged binaries (same-dir renames are atomic).
    fs.rename(&coterie_new, &coterie)
        .context("promoting the new coterie binary")?;
    if fs.is_file(&seed_new) {
        fs.rename(&seed_new, &seed)
            .context("promoting the new seed binary")?;
    }

    // Replace static + migrations wholesale.
    for sub in ["static", "migrations"] {
        let dest = install_dir.join(sub);
        let src = stage_dir.join(sub);
        if fs.is_dir(&dest) {
            fs.remove_dir_all(&dest)
                .with_context(|| format!("removing the old {sub} directory"))?;
        }
        if fs.is_dir(&src) {
            fs.copy_dir_all(&src, &dest)
                .with_context(|| format!("installing the new {sub} directory"))?;
        }
    }

    // Refresh .env.example so operators can diff for new settings. This
    // NEVER touches the live .env.
    let example_src = stage_dir.join(".env.example");
    if fs.is_file(&example_src) {
        fs.copy_file(&example_src, &install_dir.join(".env.example"))
            .context("refreshing .env.example")?;
    }

    // Refresh the bundled ops scripts so `deploy/` matches the release
    // beside it. Before the VERSION write: a failure here leaves VERSION
    // naming the old release rather than one the tree doesn't match.
    refresh_deploy_scripts(fs, stage_dir, install_dir, output)?;

    // Record the new version (copy the tarball's VERSION when present so
    // the embedded commit SHA is preserved; otherwise write the tag).
    let version_src = stage_dir.join("VERSION");
    let version_dest = install_dir.join("VERSION");
    if fs.is_file(&version_src) {
        fs.copy_file(&version_src, &version_dest)
            .context("writing the new VERSION")?;
    } else {
        fs.write(&version_dest, format!("{target_tag}\n").as_bytes())
            .context("writing the new VERSION")?;
    }

    // Re-assert ownership on the files we placed. Deliberately excludes
    // .env and the database file — those are never touched.
    for item in [
        "coterie",
        "seed",
        "static",
        "migrations",
        "deploy",
        ".env.example",
        "VERSION",
    ] {
        let p = install_dir.join(item);
        if fs.exists(&p) {
            fs.chown(&p, "coterie", "coterie")
                .with_context(|| format!("chown coterie:coterie {}", p.display()))?;
        }
    }
    Ok(())
}

/// Refresh `<install_dir>/deploy/` from the release's `deploy/` so the
/// ops scripts beside a binary are the ones that shipped with it.
///
/// File-by-file, never wholesale: `deploy/` may hold operator-authored
/// files that the tarball does not carry, and replacing the directory
/// would delete them. Only top-level regular files are copied — the
/// tarball's `deploy/` also carries the `coterie-provision` source tree,
/// which is not an ops script.
///
/// A replaced file's previous contents are retained beside it as
/// `<name>.prev`, matching the `coterie.prev` convention above: a local
/// edit to these scripts is a supported operator action. A byte-identical
/// file is not rewritten, so repeated updates do not churn.
///
/// Placing a systemd unit here does NOT install, enable, reload, or start
/// it. This writes inside the install dir and nothing else.
fn refresh_deploy_scripts<F: FileSystem, O: Output>(
    fs: &F,
    stage_dir: &Path,
    install_dir: &Path,
    output: &O,
) -> Result<()> {
    let src_dir = stage_dir.join("deploy");
    if !fs.is_dir(&src_dir) {
        return Ok(());
    }
    let dest_dir = install_dir.join("deploy");
    fs.create_dir_all(&dest_dir)
        .with_context(|| format!("creating {}", dest_dir.display()))?;

    let mut refreshed: Vec<String> = Vec::new();
    for src in fs
        .read_dir(&src_dir)
        .with_context(|| format!("listing {}", src_dir.display()))?
    {
        if !fs.is_file(&src) {
            continue;
        }
        let Some(name) = src.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let dest = dest_dir.join(name);
        let replaced = fs.is_file(&dest);
        if replaced {
            // ponytail: these are shell scripts and unit files — text.
            // An unreadable one compares as "differs" and is refreshed.
            let unchanged = matches!(
                (fs.read_to_string(&src), fs.read_to_string(&dest)),
                (Ok(incoming), Ok(current)) if incoming == current
            );
            if unchanged {
                continue;
            }
            let prev = dest_dir.join(format!("{name}.prev"));
            fs.copy_file(&dest, &prev)
                .with_context(|| format!("retaining the previous {name} as {name}.prev"))?;
        }
        fs.copy_file(&src, &dest)
            .with_context(|| format!("refreshing deploy/{name}"))?;
        if name.ends_with(".sh") {
            // backup.sh / restore.sh are invoked directly by the systemd
            // unit and by operators, so the exec bit has to survive.
            fs.chmod(&dest, 0o755)?;
        }
        refreshed.push(if replaced {
            format!("  deploy/{name} (previous kept as deploy/{name}.prev)")
        } else {
            format!("  deploy/{name} (new)")
        });
    }

    if !refreshed.is_empty() {
        output.println("Refreshed deployment scripts:");
        for line in &refreshed {
            output.println(line);
        }
    }
    Ok(())
}

/// Restore the retained previous binary and restart. Best-effort: logs
/// warnings rather than erroring, because the caller is already
/// returning an error describing the failed update.
fn rollback<F: FileSystem, S: SystemCommand, O: Output>(
    fs: &F,
    sys: &S,
    install_dir: &Path,
    prev_version: Option<&str>,
    output: &O,
) {
    output.println("Health check failed — rolling back to the previous binary.");
    let coterie = install_dir.join("coterie");
    let coterie_prev = install_dir.join("coterie.prev");
    if fs.is_file(&coterie_prev) {
        // Overwrite the bad new binary with the retained previous one.
        if let Err(e) = fs.rename(&coterie_prev, &coterie) {
            output.println(&format!(
                "WARNING: failed to restore {} from coterie.prev: {e}",
                coterie.display()
            ));
        }
    } else {
        output.println(&format!(
            "WARNING: no {} to restore from — manual recovery required",
            coterie_prev.display()
        ));
    }
    // Restore the recorded previous version string.
    if let Some(prev) = prev_version {
        let _ = fs.write(&install_dir.join("VERSION"), format!("{prev}\n").as_bytes());
    }
    // Bring the previous binary back up.
    match sys.run("systemctl", &["restart", "coterie"]) {
        Ok(out) if out.success() => output.println("Restarted coterie on the previous binary."),
        Ok(out) => output.println(&format!(
            "WARNING: systemctl restart coterie after rollback exited {}: {}",
            out.status, out.stderr
        )),
        Err(e) => output.println(&format!(
            "WARNING: failed to invoke systemctl restart after rollback: {e}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::FakeFs;
    use std::path::Path;

    #[test]
    fn snapshot_dest_is_timestamped_alongside_db() {
        let dest = snapshot_dest_path(Path::new("/var/lib/coterie/coterie.db"), "20260603-120000");
        assert_eq!(
            dest,
            PathBuf::from("/var/lib/coterie/coterie-pre-update-20260603-120000.db")
        );
    }

    #[test]
    fn installed_version_reads_first_line_trimmed() {
        let fs = FakeFs::new();
        let p = Path::new("/opt/coterie/VERSION");
        fs.put(p, b"v1.0.0\nabcdef0\n");
        assert_eq!(read_installed_version(&fs, p).as_deref(), Some("v1.0.0"));
    }

    #[test]
    fn installed_version_none_when_missing() {
        let fs = FakeFs::new();
        assert!(read_installed_version(&fs, Path::new("/opt/coterie/VERSION")).is_none());
    }
}
