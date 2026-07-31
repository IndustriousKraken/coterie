//! a50 — the backup bundle and the restore script, run for real.
//!
//! `deploy/backup.sh` and `deploy/restore.sh` are shell, so this drives
//! them as shell: a temp data dir, the actual scripts, and assertions on
//! what lands on disk. The last test is the whole round trip — back up a
//! populated instance, destroy the data dir, restore, and then serve the
//! restored attachment and image through the real routes. Restoring rows
//! is not the claim; restoring a *working instance* is, and a
//! database-only backup passes the first and fails the second.
//!
//! Everything runs under `COTERIE__SERVER__DATA_DIR` pointed at a temp
//! path: a hardcoded `/var/lib/coterie` in either script would pass any
//! default-path test and fail on every deployment using the older
//! `/opt/coterie/data` layout.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use chrono::{Duration, Utc};
use coterie::{
    api::state::{AppState, MoneyLimiter},
    domain::{
        CreateMemberRequest, Event, EventType, EventVisibility, MemberStatus, Submission,
        SubmissionStatus, SubmissionVisibility, UpdateMemberRequest,
    },
    repository::{EventRepository, SqliteEventRepository},
    web::uploads::{save_uploaded_document, save_uploaded_file},
};
use sqlx::{Executor, SqlitePool};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

mod common;

const PNG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0xDE, 0xAD];
const PDF: &[u8] = b"%PDF-1.4 the paper";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

// ---------------------------------------------------------------------
// Scaffolding
// ---------------------------------------------------------------------

/// A temp directory that removes itself, so a test run leaves nothing
/// behind even when an assertion blows up mid-way.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("coterie-{tag}-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn on_path(binary: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|d| d.join(binary).is_file()))
        .unwrap_or(false)
}

/// A `sqlite3`-CLI-compatible binary for the scripts to call: the real
/// one when the host has it, otherwise the committed python3 shim over
/// the same libsqlite3 (`VACUUM INTO` and `PRAGMA integrity_check` are
/// executed by SQLite either way). `None` means the host has neither and
/// the script tests cannot run at all.
///
/// The shims live in `tests/support/` rather than being written here:
/// writing a script and immediately exec'ing it races with every other
/// test thread's fork/exec and intermittently fails with ETXTBSY.
fn sqlite3_bin() -> Option<PathBuf> {
    if on_path("sqlite3") {
        return Some(PathBuf::from("sqlite3"));
    }
    if !on_path("python3") {
        return None;
    }
    Some(repo_root().join("tests/support/sqlite3"))
}

/// Stand-in for `systemctl` that records what the restore script asked
/// of the service manager, plus the log path it records into. "The
/// service was never stopped" is otherwise not an assertable claim in a
/// sandbox.
fn systemctl_shim(dir: &Path) -> (PathBuf, PathBuf) {
    (
        repo_root().join("tests/support/systemctl"),
        dir.join("systemctl.log"),
    )
}

fn systemctl_calls(log: &Path) -> String {
    fs::read_to_string(log).unwrap_or_default()
}

/// Run `deploy/backup.sh` against `data_dir`, writing bundles into
/// `backup_dir`.
fn run_backup(sqlite3: &Path, data_dir: &Path, backup_dir: &Path) -> Output {
    Command::new("bash")
        .arg(repo_root().join("deploy/backup.sh"))
        .env("COTERIE__SERVER__DATA_DIR", data_dir)
        .env("COTERIE_BACKUP_DIR", backup_dir)
        .env("SQLITE3", sqlite3)
        .output()
        .expect("run backup.sh")
}

/// Run `deploy/restore.sh <bundle>` against `data_dir` with a fake
/// service manager.
fn run_restore(
    sqlite3: &Path,
    systemctl: &(PathBuf, PathBuf),
    data_dir: &Path,
    bundle: &Path,
) -> Output {
    let (shim, log) = systemctl;
    Command::new("bash")
        .arg(repo_root().join("deploy/restore.sh"))
        .arg(bundle)
        .env("COTERIE__SERVER__DATA_DIR", data_dir)
        .env("SQLITE3", sqlite3)
        .env("COTERIE_SYSTEMCTL", shim)
        .env("COTERIE_TEST_SYSTEMCTL_LOG", log)
        // The sandbox is not root and the temp data dir is already ours.
        .env("COTERIE_RESTORE_ALLOW_NONROOT", "1")
        .output()
        .expect("run restore.sh")
}

fn stdout_of(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn only_bundle(backup_dir: &Path) -> PathBuf {
    let daily = backup_dir.join("daily");
    let mut entries: Vec<PathBuf> = fs::read_dir(&daily)
        .unwrap_or_else(|e| panic!("no daily dir at {}: {e}", daily.display()))
        .map(|e| e.expect("dir entry").path())
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "expected exactly one bundle in {}, got {entries:?}",
        daily.display()
    );
    entries.pop().unwrap()
}

fn bundle_members(bundle: &Path) -> String {
    let out = Command::new("tar")
        .arg("-tzf")
        .arg(bundle)
        .output()
        .expect("tar -tzf");
    assert!(out.status.success(), "tar -tzf failed: {}", stdout_of(&out));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A data dir with a database and one file in each upload root — the
/// shape the script tests need without standing up the whole app.
fn seed_data_dir(sqlite3: &Path, data_dir: &Path, marker: &str) {
    fs::create_dir_all(data_dir.join("uploads")).expect("uploads");
    fs::create_dir_all(data_dir.join("private-uploads")).expect("private-uploads");
    fs::write(data_dir.join("uploads/flyer.png"), marker).expect("write image");
    fs::write(data_dir.join("private-uploads/paper.pdf"), marker).expect("write attachment");
    let db = data_dir.join("coterie.db");
    sql(sqlite3, &db, "CREATE TABLE IF NOT EXISTS marker (v TEXT)");
    sql(
        sqlite3,
        &db,
        &format!("INSERT INTO marker VALUES ('{marker}')"),
    );
}

fn sql(sqlite3: &Path, db: &Path, statement: &str) -> String {
    let out = Command::new(sqlite3)
        .arg(db)
        .arg(statement)
        .output()
        .expect("run sqlite3");
    assert!(
        out.status.success(),
        "sqlite3 `{statement}` failed: {}",
        stdout_of(&out)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Skips (rather than fails) on a host with neither sqlite3 nor python3.
macro_rules! sqlite3_or_skip {
    () => {
        match sqlite3_bin() {
            Some(p) => p,
            None => {
                eprintln!("skipping: host has neither sqlite3 nor python3");
                return;
            }
        }
    };
}

// ---------------------------------------------------------------------
// The bundle
// ---------------------------------------------------------------------

#[test]
fn bundle_carries_the_database_and_both_upload_roots() {
    let tmp = TempDir::new("bundle");
    let sqlite3 = sqlite3_or_skip!();
    let data = tmp.path().join("data");
    let backups = tmp.path().join("backups");
    fs::create_dir_all(&data).unwrap();
    seed_data_dir(&sqlite3, &data, "original");

    let out = run_backup(&sqlite3, &data, &backups);
    assert!(out.status.success(), "backup failed: {}", stdout_of(&out));

    let members = bundle_members(&only_bundle(&backups));
    for expected in [
        "coterie.db",
        "uploads/flyer.png",
        "private-uploads/paper.pdf",
    ] {
        assert!(
            members.lines().any(|l| l == expected),
            "bundle is missing {expected}; members:\n{members}"
        );
    }
}

#[test]
fn a_fresh_instance_with_no_uploads_still_backs_up() {
    let tmp = TempDir::new("fresh");
    let sqlite3 = sqlite3_or_skip!();
    let data = tmp.path().join("data");
    let backups = tmp.path().join("backups");
    fs::create_dir_all(&data).unwrap();
    sql(
        &sqlite3,
        &data.join("coterie.db"),
        "CREATE TABLE marker (v TEXT)",
    );

    let out = run_backup(&sqlite3, &data, &backups);
    assert!(
        out.status.success(),
        "a missing upload root is normal, not an error: {}",
        stdout_of(&out)
    );
    let members = bundle_members(&only_bundle(&backups));
    for root in ["uploads/", "private-uploads/"] {
        assert!(
            members.lines().any(|l| l == root),
            "bundle should carry an empty {root}; members:\n{members}"
        );
    }
}

#[test]
fn the_backup_directory_is_never_archived_into_the_bundle() {
    let tmp = TempDir::new("nonest");
    let sqlite3 = sqlite3_or_skip!();
    let data = tmp.path().join("data");
    fs::create_dir_all(&data).unwrap();
    seed_data_dir(&sqlite3, &data, "original");
    // The default: backups live INSIDE the data dir.
    let backups = data.join("backups");

    let out = run_backup(&sqlite3, &data, &backups);
    assert!(out.status.success(), "backup failed: {}", stdout_of(&out));
    let members = bundle_members(&only_bundle(&backups));
    assert!(
        !members.contains("backups"),
        "a bundle must never contain the backup dir — that nests each \
         backup inside the next; members:\n{members}"
    );
}

// ---------------------------------------------------------------------
// Restore
// ---------------------------------------------------------------------

#[test]
fn a_truncated_bundle_is_rejected_before_anything_is_touched() {
    let tmp = TempDir::new("truncated");
    let sqlite3 = sqlite3_or_skip!();
    let systemctl = systemctl_shim(tmp.path());
    let log = systemctl.1.clone();
    let data = tmp.path().join("data");
    let backups = tmp.path().join("backups");
    fs::create_dir_all(&data).unwrap();
    seed_data_dir(&sqlite3, &data, "original");
    assert!(run_backup(&sqlite3, &data, &backups).status.success());

    let good = only_bundle(&backups);
    let truncated = tmp.path().join("truncated.tar.gz");
    let bytes = fs::read(&good).unwrap();
    fs::write(&truncated, &bytes[..bytes.len() / 2]).unwrap();

    let out = run_restore(&sqlite3, &systemctl, &data, &truncated);
    assert!(
        !out.status.success(),
        "a truncated bundle must be rejected: {}",
        stdout_of(&out)
    );
    assert_eq!(
        systemctl_calls(&log),
        "",
        "the service must still be running — nothing should have been stopped"
    );
    assert_eq!(
        sql(&sqlite3, &data.join("coterie.db"), "SELECT v FROM marker"),
        "original",
        "the existing database must be untouched"
    );
    assert!(
        data.join("uploads/flyer.png").exists() && data.join("private-uploads/paper.pdf").exists(),
        "the existing upload roots must be untouched"
    );
    assert!(
        !fs::read_dir(&data).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("pre-restore")),
        "nothing should have been displaced"
    );
}

#[test]
fn a_database_only_artifact_is_refused_as_incomplete() {
    let tmp = TempDir::new("dbonly");
    let sqlite3 = sqlite3_or_skip!();
    let systemctl = systemctl_shim(tmp.path());
    let log = systemctl.1.clone();
    let data = tmp.path().join("data");
    fs::create_dir_all(&data).unwrap();
    seed_data_dir(&sqlite3, &data, "original");

    // The pre-a50 artifact shape: the database and nothing else.
    let stage = tmp.path().join("stage");
    fs::create_dir_all(&stage).unwrap();
    fs::copy(data.join("coterie.db"), stage.join("coterie.db")).unwrap();
    let db_only = tmp.path().join("db-only.tar.gz");
    let tar = Command::new("tar")
        .arg("-czf")
        .arg(&db_only)
        .arg("-C")
        .arg(&stage)
        .arg("coterie.db")
        .output()
        .expect("tar");
    assert!(tar.status.success());

    let out = run_restore(&sqlite3, &systemctl, &data, &db_only);
    assert!(
        !out.status.success(),
        "a database-only artifact is not a complete backup: {}",
        stdout_of(&out)
    );
    assert!(
        stdout_of(&out).contains("uploads"),
        "the refusal should name what is missing: {}",
        stdout_of(&out)
    );
    assert_eq!(
        systemctl_calls(&log),
        "",
        "nothing should have been stopped"
    );
}

#[test]
fn restore_preserves_the_data_it_displaces() {
    let tmp = TempDir::new("displaced");
    let sqlite3 = sqlite3_or_skip!();
    let systemctl = systemctl_shim(tmp.path());
    let data = tmp.path().join("data");
    let backups = tmp.path().join("backups");
    fs::create_dir_all(&data).unwrap();

    // Bundle the "old" state, then move the instance on to a "current"
    // state that the restore will displace.
    seed_data_dir(&sqlite3, &data, "from-the-bundle");
    assert!(run_backup(&sqlite3, &data, &backups).status.success());
    let bundle = only_bundle(&backups);
    fs::remove_file(data.join("coterie.db")).unwrap();
    seed_data_dir(&sqlite3, &data, "current");
    fs::write(data.join("uploads/flyer.png"), "current").unwrap();

    let out = run_restore(&sqlite3, &systemctl, &data, &bundle);
    assert!(out.status.success(), "restore failed: {}", stdout_of(&out));

    // Restored content is in place...
    assert_eq!(
        fs::read_to_string(data.join("uploads/flyer.png")).unwrap(),
        "from-the-bundle"
    );
    // ...and the displaced content is retained, not deleted.
    let displaced = fs::read_dir(&data)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("pre-restore-")
        })
        .expect("displaced data should be kept in a pre-restore-* directory");
    assert_eq!(
        sql(
            &sqlite3,
            &displaced.join("coterie.db"),
            "SELECT v FROM marker"
        ),
        "current",
        "the displaced database must be recoverable"
    );
    assert_eq!(
        fs::read_to_string(displaced.join("uploads/flyer.png")).unwrap(),
        "current",
        "the displaced uploads must be recoverable"
    );
    assert!(
        displaced.join("private-uploads/paper.pdf").exists(),
        "the displaced private uploads must be recoverable"
    );
    assert!(
        stdout_of(&out).contains(&displaced.to_string_lossy().to_string()),
        "the script must print where the displaced data went: {}",
        stdout_of(&out)
    );
}

#[test]
fn backup_and_restore_work_against_a_non_default_data_dir() {
    // Every test here uses a temp COTERIE__SERVER__DATA_DIR, but this
    // one says so out loud: the default path appears nowhere in the
    // round trip, and the older /opt/coterie/data layout works too.
    let tmp = TempDir::new("nondefault");
    let sqlite3 = sqlite3_or_skip!();
    let systemctl = systemctl_shim(tmp.path());
    let log = systemctl.1.clone();
    let data = tmp.path().join("opt/coterie/data");
    let backups = tmp.path().join("mnt/backup-volume");
    fs::create_dir_all(&data).unwrap();
    seed_data_dir(&sqlite3, &data, "elsewhere");

    assert!(run_backup(&sqlite3, &data, &backups).status.success());
    let bundle = only_bundle(&backups);
    fs::remove_dir_all(&data).unwrap();

    let out = run_restore(&sqlite3, &systemctl, &data, &bundle);
    assert!(out.status.success(), "restore failed: {}", stdout_of(&out));
    assert_eq!(
        sql(&sqlite3, &data.join("coterie.db"), "SELECT v FROM marker"),
        "elsewhere"
    );
    assert!(data.join("uploads/flyer.png").exists());
    assert!(data.join("private-uploads/paper.pdf").exists());
    assert!(
        !PathBuf::from("/var/lib/coterie").exists(),
        "the scripts must not have created the default path on this host"
    );
    assert!(systemctl_calls(&log).contains("start coterie"));
}

// ---------------------------------------------------------------------
// The round trip: a bundle restores to a *working instance*
// ---------------------------------------------------------------------

async fn file_pool(db_path: &Path) -> SqlitePool {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .after_connect(|conn, _| {
            Box::pin(async move {
                conn.execute("PRAGMA foreign_keys = ON").await?;
                Ok(())
            })
        })
        .connect_with(opts)
        .await
        .expect("open file-backed sqlite");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrate");
    pool
}

/// `common::build_app_state` with the data dir pointed at a real
/// directory, so uploads land where the scripts look for them.
async fn state_on(pool: SqlitePool, data_dir: &Path) -> AppState {
    let base = common::build_app_state(pool).await;
    let mut settings = (*base.settings).clone();
    settings.server.data_dir = data_dir.to_string_lossy().into_owned();
    AppState::new(
        base.service_context.clone(),
        base.stripe.clone(),
        base.billing_service.clone(),
        Arc::new(settings),
        base.bot_challenge_verifier.clone(),
        MoneyLimiter(base.money_limiter.clone()),
    )
}

fn get(uri: &str, session: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header(header::COOKIE, format!("session={session}"))
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn a_bundle_restores_to_a_working_instance() {
    let tmp = TempDir::new("roundtrip");
    let sqlite3 = sqlite3_or_skip!();
    let systemctl = systemctl_shim(tmp.path());
    let log = systemctl.1.clone();
    let data = tmp.path().join("data");
    let backups = tmp.path().join("backups");
    fs::create_dir_all(&data).unwrap();

    // --- an instance with all three bundle components non-empty -------
    let pool = file_pool(&data.join("coterie.db")).await;
    let state = state_on(pool.clone(), &data).await;
    sqlx::query("UPDATE app_settings SET value = 'true' WHERE key = 'submissions.enabled'")
        .execute(&pool)
        .await
        .expect("enable submissions");

    let member = state
        .service_context
        .member_repo
        .create(CreateMemberRequest {
            email: "restored@example.com".into(),
            username: "restored".into(),
            full_name: "Restored Member".into(),
            password: "p4ssword_long_enough".into(),
            membership_type_id: None,
            ..Default::default()
        })
        .await
        .expect("create member");
    state
        .service_context
        .member_repo
        .update(
            member.id,
            UpdateMemberRequest {
                status: Some(MemberStatus::Active),
                ..Default::default()
            },
        )
        .await
        .expect("activate member");
    let (_, session) = state
        .service_context
        .auth_service
        .create_session(member.id, 24)
        .await
        .expect("create session");

    // A submission attachment (private root) ...
    let stored_pdf = save_uploaded_document(&state.settings.server.private_uploads_path(), PDF)
        .await
        .expect("save attachment");
    let now = Utc::now();
    let submission_id = state
        .service_context
        .submission_repo
        .create(Submission {
            id: Uuid::new_v4(),
            submitter_member_id: member.id,
            title: "Paper".into(),
            abstract_text: "Body".into(),
            visibility_requested: SubmissionVisibility::Members,
            attachment_path: Some(stored_pdf),
            preferred_start: None,
            timezone: "UTC".into(),
            duration_minutes: None,
            status: SubmissionStatus::Submitted,
            reviewer_note: None,
            decided_by: None,
            event_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("create submission")
        .id;

    // ... and a public event image (public root).
    let stored_png = save_uploaded_file(&state.settings.server.uploads_path(), "flyer.png", PNG)
        .await
        .expect("save image");
    let image_filename = stored_png
        .strip_prefix("uploads/")
        .expect("public prefix")
        .to_string();
    SqliteEventRepository::new(pool.clone())
        .create(Event {
            id: Uuid::new_v4(),
            title: "Lockpicking 101".into(),
            description: "Bring a padlock".into(),
            event_type: EventType::Workshop,
            event_type_id: None,
            visibility: EventVisibility::Public,
            start_time: now + Duration::days(7),
            end_time: None,
            timezone: "UTC".into(),
            location: None,
            max_attendees: None,
            rsvp_required: false,
            member_price_cents: 0,
            guest_price_cents: 0,
            guest_registration_enabled: false,
            image_url: Some(stored_png.clone()),
            created_by: member.id,
            created_at: now,
            updated_at: now,
            series_id: None,
            occurrence_index: None,
        })
        .await
        .expect("create event");

    // --- back it up while the instance is live ------------------------
    let out = run_backup(&sqlite3, &data, &backups);
    assert!(out.status.success(), "backup failed: {}", stdout_of(&out));
    let bundle = only_bundle(&backups);

    // --- destroy the data dir entirely --------------------------------
    pool.close().await;
    drop(state);
    fs::remove_dir_all(&data).expect("destroy data dir");
    assert!(!data.exists());

    // --- restore ------------------------------------------------------
    let out = run_restore(&sqlite3, &systemctl, &data, &bundle);
    assert!(out.status.success(), "restore failed: {}", stdout_of(&out));
    assert!(
        stdout_of(&out).contains("integrity_check: ok"),
        "restore must integrity-check the database: {}",
        stdout_of(&out)
    );
    let calls = systemctl_calls(&log);
    assert!(
        calls.contains("stop coterie") && calls.contains("start coterie"),
        "restore must stop the service before swapping the DB and start it after: {calls}"
    );

    // --- start the instance back up on the restored data --------------
    let pool = file_pool(&data.join("coterie.db")).await;
    let restored: Vec<(String,)> = sqlx::query_as("PRAGMA integrity_check")
        .fetch_all(&pool)
        .await
        .expect("integrity_check");
    assert_eq!(restored[0].0, "ok");

    let state = state_on(pool.clone(), &data).await;
    let app = coterie::web::create_web_routes(state.clone());

    let found = state
        .service_context
        .member_repo
        .find_by_email("restored@example.com")
        .await
        .expect("query member");
    assert!(found.is_some(), "the member row should be back");

    let resp = app
        .clone()
        .oneshot(get(
            &format!("/portal/submissions/{submission_id}/attachment"),
            &session,
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "the attachment must be fetchable through its gated route after a restore"
    );
    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .expect("read attachment body");
    assert_eq!(&body[..], PDF, "the attachment bytes must survive the trip");

    let resp = app
        .oneshot(get(&format!("/uploads/{image_filename}"), &session))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "the event image must be served by /uploads/:filename after a restore"
    );

    pool.close().await;
    // TempDir::drop tears the data dir down; nothing is left behind.
}

#[test]
fn a_corrupt_database_stops_the_restore_before_the_service_starts() {
    let tmp = TempDir::new("corruptdb");
    let sqlite3 = sqlite3_or_skip!();
    let systemctl = systemctl_shim(tmp.path());
    let log = systemctl.1.clone();
    let data = tmp.path().join("data");
    fs::create_dir_all(&data).unwrap();
    seed_data_dir(&sqlite3, &data, "current");

    // A well-formed bundle carrying a "database" that is not one. This
    // gets past the archive check, so the integrity check is the only
    // thing standing between it and a service started on garbage.
    let stage = tmp.path().join("stage");
    fs::create_dir_all(stage.join("uploads")).unwrap();
    fs::create_dir_all(stage.join("private-uploads")).unwrap();
    fs::write(stage.join("coterie.db"), vec![0x7f; 4096]).unwrap();
    let bad = tmp.path().join("bad.tar.gz");
    let tar = Command::new("tar")
        .arg("-czf")
        .arg(&bad)
        .arg("-C")
        .arg(&stage)
        .args(["coterie.db", "uploads", "private-uploads"])
        .output()
        .expect("tar");
    assert!(tar.status.success());

    let out = run_restore(&sqlite3, &systemctl, &data, &bad);
    assert!(
        !out.status.success(),
        "a database that fails integrity_check must fail the restore: {}",
        stdout_of(&out)
    );
    let calls = systemctl_calls(&log);
    assert!(calls.contains("stop coterie"), "calls: {calls}");
    assert!(
        !calls.contains("start coterie"),
        "the service must NOT be started on a failed integrity check: {calls}"
    );
    let displaced = fs::read_dir(&data)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("pre-restore-")
        })
        .expect("the displaced data is the way back from a bad restore");
    assert_eq!(
        sql(
            &sqlite3,
            &displaced.join("coterie.db"),
            "SELECT v FROM marker"
        ),
        "current"
    );
    assert!(
        stdout_of(&out).contains(&displaced.to_string_lossy().to_string()),
        "the failure must say where the previous data is: {}",
        stdout_of(&out)
    );
}

/// A bundle is input from a trust boundary — it may have come back from
/// offsite storage, or off a host someone else has had — and restore.sh
/// extracts it as root. A member that would write outside the restore
/// directory is refused during verification, while the instance is
/// still up, rather than by whichever tar the host happens to ship
/// midway through the restore.
#[test]
fn a_bundle_with_a_traversal_member_is_refused() {
    let tmp = TempDir::new("traversal");
    let sqlite3 = sqlite3_or_skip!();
    let systemctl = systemctl_shim(tmp.path());
    let log = systemctl.1.clone();
    let data = tmp.path().join("data");
    fs::create_dir_all(&data).unwrap();
    seed_data_dir(&sqlite3, &data, "original");

    // An otherwise-valid bundle carrying one booby-trapped member.
    // `-P` is what stops tar sanitising the name as it writes it.
    let stage = tmp.path().join("stage");
    fs::create_dir_all(stage.join("uploads")).unwrap();
    fs::create_dir_all(stage.join("private-uploads")).unwrap();
    fs::copy(data.join("coterie.db"), stage.join("coterie.db")).unwrap();
    fs::write(tmp.path().join("evil"), "pwned").unwrap();
    let booby_trapped = tmp.path().join("evil.tar.gz");
    let tar = Command::new("tar")
        .arg("-czPf")
        .arg(&booby_trapped)
        .arg("-C")
        .arg(&stage)
        .args(["coterie.db", "uploads", "private-uploads", "../evil"])
        .output()
        .expect("tar");
    assert!(tar.status.success(), "tar: {}", stdout_of(&tar));
    if !bundle_members(&booby_trapped)
        .lines()
        .any(|l| l.contains(".."))
    {
        eprintln!("skipping: this host's tar sanitises `..` at create time");
        return;
    }

    let out = run_restore(&sqlite3, &systemctl, &data, &booby_trapped);
    assert!(
        !out.status.success(),
        "a member that escapes the restore directory must be refused: {}",
        stdout_of(&out)
    );
    assert_eq!(
        systemctl_calls(&log),
        "",
        "the refusal must come before the service is stopped"
    );
    assert_eq!(
        sql(&sqlite3, &data.join("coterie.db"), "SELECT v FROM marker"),
        "original",
        "the existing database must be untouched"
    );
    assert!(
        !tmp.path().join("evil").is_dir()
            && fs::read_to_string(tmp.path().join("evil")).unwrap() == "pwned",
        "nothing should have been written through the traversal member"
    );
}

/// A failed integrity check on a fresh host must not send the operator
/// looking for a pre-restore-* directory: nothing was displaced, so it
/// was never created.
#[test]
fn a_failed_integrity_check_names_no_displaced_data_when_there_is_none() {
    let tmp = TempDir::new("freshfail");
    let sqlite3 = sqlite3_or_skip!();
    let systemctl = systemctl_shim(tmp.path());
    let data = tmp.path().join("data"); // never created: the fresh-host case

    let stage = tmp.path().join("stage");
    fs::create_dir_all(stage.join("uploads")).unwrap();
    fs::create_dir_all(stage.join("private-uploads")).unwrap();
    fs::write(stage.join("coterie.db"), vec![0x7f; 4096]).unwrap();
    let bad = tmp.path().join("bad.tar.gz");
    let tar = Command::new("tar")
        .arg("-czf")
        .arg(&bad)
        .arg("-C")
        .arg(&stage)
        .args(["coterie.db", "uploads", "private-uploads"])
        .output()
        .expect("tar");
    assert!(tar.status.success());

    let out = run_restore(&sqlite3, &systemctl, &data, &bad);
    assert!(
        !out.status.success(),
        "a bad database must fail the restore"
    );
    let said = stdout_of(&out);
    assert!(
        !said.contains("pre-restore"),
        "nothing was displaced, so the failure must not point at a pre-restore dir: {said}"
    );
    assert!(
        said.contains("Nothing was displaced"),
        "the failure should say so plainly instead: {said}"
    );
}
