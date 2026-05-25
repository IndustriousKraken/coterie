//! End-to-end integration tests for `coterie-provision
//! switch-stripe-to-live`. Drives the orchestrator against
//! `FakeSystem` + `FakeFs` + `FakeStripeApi` + tempfile-backed sqlite
//! DBs.

use coterie_provision::output::CaptureOutput;
use coterie_provision::stripe_api::fake::FakeStripeApi;
use coterie_provision::switch_to_live::{
    self, copy_admin_rows, create_live_db_with_test_schema, has_live_pk, parse_env_pairs,
    rewrite_env, Paths, SkipRootCheck, SwitchArgs, COTERIE_TEST_DB_PATH, ENV_LIVE_PATH, ENV_PATH,
};
use coterie_provision::test_support::{FakeFs, FakeSystem, MockPrompter};
use rusqlite::Connection;
use secrecy::{Secret, SecretString};
use std::path::Path;

// --- Helpers ---------------------------------------------------------

/// Test-only filesystem that delegates everything to `RealFs` except
/// `chown`, which it no-ops. CI test boxes have no `coterie:coterie`
/// user, so a real chown call always fails there — the previous code
/// masked this with `.ok()`, but a34 now correctly propagates chown
/// errors. These end-to-end tests still want RealFs for rusqlite and
/// real rename ops; the chown is the only call that needs stubbing.
struct NoopChownFs(coterie_provision::fs_ops::RealFs);

impl NoopChownFs {
    fn new() -> Self {
        Self(coterie_provision::fs_ops::RealFs::new())
    }
}

impl coterie_provision::fs_ops::FileSystem for NoopChownFs {
    fn read_to_string(&self, path: &std::path::Path) -> anyhow::Result<String> {
        self.0.read_to_string(path)
    }
    fn write(&self, path: &std::path::Path, contents: &[u8]) -> anyhow::Result<()> {
        self.0.write(path, contents)
    }
    fn append(&self, path: &std::path::Path, contents: &[u8]) -> anyhow::Result<()> {
        self.0.append(path, contents)
    }
    fn create_dir_all(&self, path: &std::path::Path) -> anyhow::Result<()> {
        self.0.create_dir_all(path)
    }
    fn chmod(&self, path: &std::path::Path, mode: u32) -> anyhow::Result<()> {
        self.0.chmod(path, mode)
    }
    fn chown(&self, _path: &std::path::Path, _user: &str, _group: &str) -> anyhow::Result<()> {
        Ok(())
    }
    fn exists(&self, path: &std::path::Path) -> bool {
        self.0.exists(path)
    }
    fn is_file(&self, path: &std::path::Path) -> bool {
        self.0.is_file(path)
    }
    fn is_dir(&self, path: &std::path::Path) -> bool {
        self.0.is_dir(path)
    }
    fn rename(&self, from: &std::path::Path, to: &std::path::Path) -> anyhow::Result<()> {
        self.0.rename(from, to)
    }
    fn remove_file(&self, path: &std::path::Path) -> anyhow::Result<()> {
        self.0.remove_file(path)
    }
    fn remove_dir_all(&self, path: &std::path::Path) -> anyhow::Result<()> {
        self.0.remove_dir_all(path)
    }
}

const SAMPLE_TEST_ENV: &str = "\
COTERIE__SERVER__PORT=8080
COTERIE__SERVER__BASE_URL=https://coterie.example.com
COTERIE__STRIPE__ENABLED=true
COTERIE__STRIPE__PUBLISHABLE_KEY=pk_test_OLD
COTERIE__STRIPE__SECRET_KEY=sk_test_OLD
COTERIE__STRIPE__WEBHOOK_SECRET=whsec_OLD
COTERIE__DATABASE__URL=sqlite:///var/lib/coterie/coterie-test.db?mode=rwc
COTERIE__AUTH__SESSION_SECRET=deadbeef
";

fn stage_test_mode_install(fs: &FakeFs) {
    fs.put(Path::new(ENV_PATH), SAMPLE_TEST_ENV.as_bytes());
    fs.put(Path::new(COTERIE_TEST_DB_PATH), b"sqlite-placeholder");
}

fn base_args() -> SwitchArgs {
    SwitchArgs {
        yes: true,
        no_prompt: true,
        live_pk: Some("pk_live_NEW".to_string()),
        live_sk: Some(SecretString::new("sk_live_NEW".to_string())),
        live_whsec: Some(SecretString::new("whsec_NEW".to_string())),
        ..Default::default()
    }
}

// --- Idempotency / preflight refusal paths --------------------------

#[test]
fn refuses_when_env_already_has_pk_live() {
    let fs = FakeFs::new();
    fs.put(
        Path::new(ENV_PATH),
        b"COTERIE__STRIPE__PUBLISHABLE_KEY=pk_live_ALREADY\n",
    );
    fs.put(Path::new(COTERIE_TEST_DB_PATH), b"sqlite-placeholder");

    let sys = FakeSystem::new();
    let api = FakeStripeApi::accept_all();
    let prompts = MockPrompter::new();
    let out = CaptureOutput::new();
    let code = switch_to_live::run(base_args(), &sys, &fs, &api, &prompts, &out, &SkipRootCheck)
        .expect("should exit 0 in already-live case");
    assert_eq!(code, 0, "already-live is an exit-0 (idempotent) case");

    // No commands run, no fs mutations beyond the initial read.
    assert_eq!(sys.calls.borrow().len(), 0);
    assert!(out.contains("Already in live mode"));
}

#[test]
fn refuses_when_no_test_db_exists() {
    let fs = FakeFs::new();
    fs.put(Path::new(ENV_PATH), SAMPLE_TEST_ENV.as_bytes());
    // No coterie-test.db.

    let sys = FakeSystem::new();
    let api = FakeStripeApi::accept_all();
    let prompts = MockPrompter::new();
    let out = CaptureOutput::new();
    let err = switch_to_live::run(base_args(), &sys, &fs, &api, &prompts, &out, &SkipRootCheck)
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Not in test mode"), "got: {msg}");
    assert_eq!(sys.calls.borrow().len(), 0);
}

#[test]
fn refuses_when_stripe_api_rejects_live_key() {
    let fs = FakeFs::new();
    stage_test_mode_install(&fs);

    let sys = FakeSystem::new();
    let api = FakeStripeApi::reject_all();
    let prompts = MockPrompter::new();
    let out = CaptureOutput::new();
    let err = switch_to_live::run(base_args(), &sys, &fs, &api, &prompts, &out, &SkipRootCheck)
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("Stripe rejected the live secret key"),
        "got: {msg}"
    );
    // CRITICAL: no destructive ops happened.
    assert_eq!(
        sys.calls.borrow().len(),
        0,
        "no commands should run when Stripe rejects"
    );
    // .env still has test creds.
    let env = String::from_utf8(fs.get(Path::new(ENV_PATH)).unwrap()).unwrap();
    assert!(env.contains("pk_test_OLD"));
}

// --- Pure function: rewrite_env (additional coverage) ---------------

#[test]
fn rewrite_env_golden_full_test_env() {
    let pk = "pk_live_AAA";
    let sk = Secret::new("sk_live_BBB".to_string());
    let wh = Secret::new("whsec_CCC".to_string());
    let out = rewrite_env(SAMPLE_TEST_ENV, pk, &sk, &wh);
    let expected = "\
COTERIE__SERVER__PORT=8080
COTERIE__SERVER__BASE_URL=https://coterie.example.com
COTERIE__STRIPE__ENABLED=true
COTERIE__STRIPE__PUBLISHABLE_KEY=pk_live_AAA
COTERIE__STRIPE__SECRET_KEY=sk_live_BBB
COTERIE__STRIPE__WEBHOOK_SECRET=whsec_CCC
COTERIE__DATABASE__URL=sqlite://coterie.db
COTERIE__AUTH__SESSION_SECRET=deadbeef
";
    assert_eq!(out, expected);
}

#[test]
fn has_live_pk_finds_value_in_env_blob() {
    let with_live = SAMPLE_TEST_ENV.replace("pk_test_OLD", "pk_live_NEW");
    assert!(has_live_pk(&with_live));
    assert!(!has_live_pk(SAMPLE_TEST_ENV));
}

// --- Pure function: parse_env_pairs ---------------------------------

#[test]
fn parse_env_live_extracts_three_keys() {
    let body = "\
# comment
COTERIE__STRIPE__PUBLISHABLE_KEY=pk_live_AAA
COTERIE__STRIPE__SECRET_KEY=sk_live_BBB
COTERIE__STRIPE__WEBHOOK_SECRET=whsec_CCC
";
    let map = parse_env_pairs(body);
    assert_eq!(
        map.get("COTERIE__STRIPE__PUBLISHABLE_KEY")
            .map(String::as_str),
        Some("pk_live_AAA")
    );
    assert_eq!(
        map.get("COTERIE__STRIPE__SECRET_KEY").map(String::as_str),
        Some("sk_live_BBB")
    );
    assert_eq!(
        map.get("COTERIE__STRIPE__WEBHOOK_SECRET")
            .map(String::as_str),
        Some("whsec_CCC")
    );
}

// --- Real-sqlite tests for the DB migration ------------------------

const SCHEMA_SQL: &str = "CREATE TABLE members (
    id INTEGER PRIMARY KEY,
    email TEXT NOT NULL,
    username TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    is_admin INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_members_email ON members(email);";

fn pre_seed_test_db(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(SCHEMA_SQL).unwrap();
    conn.execute(
        "INSERT INTO members (email, username, password_hash, is_admin) \
         VALUES ('rab@acme.io', 'rab', 'argon2$abc', 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO members (email, username, password_hash, is_admin) \
         VALUES ('member@acme.io', 'member', 'argon2$xyz', 0)",
        [],
    )
    .unwrap();
}

#[test]
fn schema_clones_from_test_db_into_fresh_live_db() {
    let dir = tempfile::tempdir().unwrap();
    let test_db = dir.path().join("coterie-test.db");
    let new_db = dir.path().join("coterie.db");
    pre_seed_test_db(&test_db);

    create_live_db_with_test_schema(&test_db, &new_db).unwrap();

    let new_conn = Connection::open(&new_db).unwrap();
    let count: i64 = new_conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='members'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "members table should exist on the fresh DB");
    let member_count: i64 = new_conn
        .query_row("SELECT COUNT(*) FROM members", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        member_count, 0,
        "fresh live DB starts empty (no rows copied)"
    );
}

#[test]
fn admin_row_migrates_across_dbs_via_attach() {
    let dir = tempfile::tempdir().unwrap();
    let test_db = dir.path().join("coterie-test.db");
    let new_db = dir.path().join("coterie.db");
    pre_seed_test_db(&test_db);

    create_live_db_with_test_schema(&test_db, &new_db).unwrap();
    copy_admin_rows(&test_db, &new_db).unwrap();

    let new_conn = Connection::open(&new_db).unwrap();
    let admin: (String, String, i64) = new_conn
        .query_row(
            "SELECT email, username, is_admin FROM members WHERE is_admin = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(admin.0, "rab@acme.io");
    assert_eq!(admin.1, "rab");
    assert_eq!(admin.2, 1);

    // Non-admin row NOT copied.
    let non_admin_count: i64 = new_conn
        .query_row("SELECT COUNT(*) FROM members WHERE is_admin = 0", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        non_admin_count, 0,
        "non-admin rows should not migrate across the switchover"
    );
}

#[test]
fn multiple_admin_rows_all_migrate() {
    let dir = tempfile::tempdir().unwrap();
    let test_db = dir.path().join("coterie-test.db");
    let new_db = dir.path().join("coterie.db");
    let conn = Connection::open(&test_db).unwrap();
    conn.execute_batch(SCHEMA_SQL).unwrap();
    conn.execute(
        "INSERT INTO members (email, username, password_hash, is_admin) \
         VALUES ('rab@acme.io', 'rab', 'p1', 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO members (email, username, password_hash, is_admin) \
         VALUES ('co@acme.io', 'co', 'p2', 1)",
        [],
    )
    .unwrap();
    drop(conn);

    create_live_db_with_test_schema(&test_db, &new_db).unwrap();
    copy_admin_rows(&test_db, &new_db).unwrap();

    let new_conn = Connection::open(&new_db).unwrap();
    let admin_count: i64 = new_conn
        .query_row("SELECT COUNT(*) FROM members WHERE is_admin = 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(admin_count, 2, "all admins should migrate");
}

// --- Live-cred loading path (.env.live) ----------------------------

#[test]
fn loads_live_creds_from_env_live_when_present() {
    let fs = FakeFs::new();
    stage_test_mode_install(&fs);
    fs.put(
        Path::new(ENV_LIVE_PATH),
        b"COTERIE__STRIPE__PUBLISHABLE_KEY=pk_live_FROMFILE\n\
          COTERIE__STRIPE__SECRET_KEY=sk_live_FROMFILE\n\
          COTERIE__STRIPE__WEBHOOK_SECRET=whsec_FROMFILE\n",
    );

    // Args don't supply flags — should not need to prompt or read CLI.
    let args = SwitchArgs {
        yes: true,
        no_prompt: true,
        ..Default::default()
    };
    let sys = FakeSystem::new();
    // Configure the FakeStripeApi to reject so we abort cleanly after
    // the cred-loading step — this proves the .env.live values were
    // loaded and handed to the StripeApi without any destructive ops
    // having happened.
    let api = FakeStripeApi::reject_all();
    let prompts = MockPrompter::new();
    let out = CaptureOutput::new();
    let err =
        switch_to_live::run(args, &sys, &fs, &api, &prompts, &out, &SkipRootCheck).unwrap_err();
    assert!(err.to_string().contains("Stripe"));
    assert_eq!(api.attempted.borrow().len(), 1);
    assert_eq!(api.attempted.borrow()[0], "sk_live_FROMFILE");
}

#[test]
fn refuses_when_env_live_has_bad_prefix() {
    let fs = FakeFs::new();
    stage_test_mode_install(&fs);
    fs.put(
        Path::new(ENV_LIVE_PATH),
        b"COTERIE__STRIPE__PUBLISHABLE_KEY=pk_test_BAD\n\
          COTERIE__STRIPE__SECRET_KEY=sk_live_AAA\n\
          COTERIE__STRIPE__WEBHOOK_SECRET=whsec_AAA\n",
    );

    let args = SwitchArgs {
        yes: true,
        no_prompt: true,
        ..Default::default()
    };
    let sys = FakeSystem::new();
    let api = FakeStripeApi::accept_all();
    let prompts = MockPrompter::new();
    let out = CaptureOutput::new();
    let err =
        switch_to_live::run(args, &sys, &fs, &api, &prompts, &out, &SkipRootCheck).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("pk_live_"),
        "should reject pk_test_ prefix in .env.live; got: {msg}"
    );
}

// --- DB-archive vs discard branches ---------------------------------

// --- End-to-end happy path via injectable Paths -----------------------

/// Production paths are `/opt/coterie/...` and `/var/lib/coterie/...`;
/// the `Paths` struct lets tests redirect to a tempdir so rusqlite can
/// work with real files. We also use real fs ops (RealFs) for the env
/// rewrites so the rename + read paths are exercised end-to-end.
fn paths_in(dir: &Path) -> Paths {
    Paths {
        env: dir.join("opt-coterie-env"),
        env_live: dir.join("opt-coterie-env-live"),
        env_new: dir.join("opt-coterie-env-new"),
        coterie_db: dir.join("coterie.db"),
        coterie_test_db: dir.join("coterie-test.db"),
        archive_dir: dir.to_path_buf(),
    }
}

#[test]
fn happy_path_archive_branch_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths_in(dir.path());
    let fs = NoopChownFs::new();

    // Seed test-mode state.
    std::fs::write(&paths.env, SAMPLE_TEST_ENV).unwrap();
    pre_seed_test_db(&paths.coterie_test_db);

    let args = SwitchArgs {
        yes: true,
        no_prompt: true,
        live_pk: Some("pk_live_HAPPY".to_string()),
        live_sk: Some(SecretString::new("sk_live_HAPPY".to_string())),
        live_whsec: Some(SecretString::new("whsec_HAPPY".to_string())),
        ..Default::default()
    };

    let sys = FakeSystem::new();
    sys.respond_to(
        "systemctl",
        &["is-active", "coterie"],
        coterie_provision::system::CommandOutput {
            status: 0,
            stdout: "active\n".to_string(),
            stderr: String::new(),
        },
    );
    let api = FakeStripeApi::accept_all();
    let prompts = MockPrompter::new();
    let out = CaptureOutput::new();

    let code = switch_to_live::run_with_paths(
        args,
        &sys,
        &fs,
        &api,
        &prompts,
        &out,
        &SkipRootCheck,
        &paths,
    )
    .expect("happy path should complete");
    assert_eq!(code, 0);

    // --- Assertions: command order ---
    let cmds: Vec<(String, Vec<String>)> = sys
        .calls
        .borrow()
        .iter()
        .map(|c| (c.cmd.clone(), c.args.clone()))
        .collect();
    let names: Vec<(&str, Vec<&str>)> = cmds
        .iter()
        .map(|(c, a)| (c.as_str(), a.iter().map(String::as_str).collect()))
        .collect();
    // Expected sequence: stop, then is-active poll(s), then start, then is-active, then curl.
    assert_eq!(names[0], ("systemctl", vec!["stop", "coterie"]));
    assert_eq!(
        names[names.len() - 1],
        ("curl", vec!["-fsSL", "http://127.0.0.1:8080/health"])
    );
    // Contains the start command somewhere in the middle.
    assert!(names
        .iter()
        .any(|(c, a)| *c == "systemctl" && a == &["start", "coterie"]));

    // --- Assertions: .env rewritten ---
    let new_env = std::fs::read_to_string(&paths.env).unwrap();
    assert!(new_env.contains("COTERIE__STRIPE__PUBLISHABLE_KEY=pk_live_HAPPY"));
    assert!(new_env.contains("COTERIE__STRIPE__SECRET_KEY=sk_live_HAPPY"));
    assert!(new_env.contains("COTERIE__STRIPE__WEBHOOK_SECRET=whsec_HAPPY"));
    assert!(new_env.contains("COTERIE__DATABASE__URL=sqlite://coterie.db"));

    // --- Assertions: coterie.db exists and has the admin row ---
    assert!(paths.coterie_db.exists());
    let conn = Connection::open(&paths.coterie_db).unwrap();
    let admin_email: String = conn
        .query_row("SELECT email FROM members WHERE is_admin = 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(admin_email, "rab@acme.io");

    // --- Assertions: coterie-test.db has been archived ---
    assert!(!paths.coterie_test_db.exists());
    let archived: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("coterie-test-archive-")
        })
        .collect();
    assert_eq!(archived.len(), 1, "archive file should exist");

    // --- Output contains webhook reminder ---
    assert!(out.contains("Switched to Stripe LIVE mode"));
    assert!(out.contains("/api/payments/webhook/stripe"));
}

#[test]
fn happy_path_discard_branch_removes_test_db() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths_in(dir.path());
    let fs = NoopChownFs::new();

    std::fs::write(&paths.env, SAMPLE_TEST_ENV).unwrap();
    pre_seed_test_db(&paths.coterie_test_db);

    let args = SwitchArgs {
        yes: true,
        no_prompt: true,
        discard_test_db: true,
        live_pk: Some("pk_live_X".to_string()),
        live_sk: Some(SecretString::new("sk_live_X".to_string())),
        live_whsec: Some(SecretString::new("whsec_X".to_string())),
    };

    let sys = FakeSystem::new();
    sys.respond_to(
        "systemctl",
        &["is-active", "coterie"],
        coterie_provision::system::CommandOutput {
            status: 0,
            stdout: "active\n".to_string(),
            stderr: String::new(),
        },
    );
    let api = FakeStripeApi::accept_all();
    let prompts = MockPrompter::new();
    let out = CaptureOutput::new();

    switch_to_live::run_with_paths(
        args,
        &sys,
        &fs,
        &api,
        &prompts,
        &out,
        &SkipRootCheck,
        &paths,
    )
    .unwrap();

    assert!(!paths.coterie_test_db.exists(), "discard removes test DB");
    let archives: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("coterie-test-archive-")
        })
        .collect();
    assert!(
        archives.is_empty(),
        "discard branch should not create an archive"
    );
}

#[test]
fn happy_path_consumes_env_live_and_removes_it() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths_in(dir.path());
    let fs = NoopChownFs::new();

    std::fs::write(&paths.env, SAMPLE_TEST_ENV).unwrap();
    std::fs::write(
        &paths.env_live,
        "COTERIE__STRIPE__PUBLISHABLE_KEY=pk_live_FROMFILE\n\
         COTERIE__STRIPE__SECRET_KEY=sk_live_FROMFILE\n\
         COTERIE__STRIPE__WEBHOOK_SECRET=whsec_FROMFILE\n",
    )
    .unwrap();
    pre_seed_test_db(&paths.coterie_test_db);

    // No live_* in args — should pull from .env.live.
    let args = SwitchArgs {
        yes: true,
        no_prompt: true,
        ..Default::default()
    };

    let sys = FakeSystem::new();
    sys.respond_to(
        "systemctl",
        &["is-active", "coterie"],
        coterie_provision::system::CommandOutput {
            status: 0,
            stdout: "active\n".to_string(),
            stderr: String::new(),
        },
    );
    let api = FakeStripeApi::accept_all();
    let prompts = MockPrompter::new();
    let out = CaptureOutput::new();

    switch_to_live::run_with_paths(
        args,
        &sys,
        &fs,
        &api,
        &prompts,
        &out,
        &SkipRootCheck,
        &paths,
    )
    .unwrap();

    // .env.live should be gone after consumption.
    assert!(
        !paths.env_live.exists(),
        ".env.live should be removed after the switchover"
    );

    let new_env = std::fs::read_to_string(&paths.env).unwrap();
    assert!(new_env.contains("pk_live_FROMFILE"));
    assert!(new_env.contains("sk_live_FROMFILE"));
}
