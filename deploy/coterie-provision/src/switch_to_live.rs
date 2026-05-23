//! `coterie-provision switch-stripe-to-live` — one-shot transition from
//! test-mode Coterie to live-mode Coterie.
//!
//! See openspec change `a25` for the full spec. Quick summary of the
//! steps this module performs (in order):
//!
//! 1. Preflight: refuse if `.env` already has `pk_live_`, or if no
//!    `coterie-test.db` exists.
//! 2. Load live credentials from `/opt/coterie/.env.live` (if present)
//!    or prompt for them. Validate prefixes.
//! 3. Stripe `/v1/balance` smoke test via `StripeApi`. Abort BEFORE any
//!    destructive operation if the key is rejected.
//! 4. Confirmation prompt (skipped if `--yes` or `--no-prompt`).
//! 5. `systemctl stop coterie`.
//! 6. Create fresh `coterie.db` with the same schema as
//!    `coterie-test.db` (via `rusqlite`'s `sqlite_master`).
//! 7. Copy admin row(s) from `coterie-test.db` → `coterie.db` via
//!    `ATTACH DATABASE ... INSERT ... SELECT`.
//! 8. Archive `coterie-test.db` (default) or discard with
//!    `--discard-test-db`.
//! 9. Atomic `.env` rewrite (write `.env.new`, rename to `.env`).
//! 10. Remove `/opt/coterie/.env.live` if it existed.
//! 11. `systemctl start coterie`, poll `is-active`.
//! 12. HTTP smoke test against `/health`.
//! 13. Print success summary + webhook reminder.

use anyhow::{anyhow, Context, Result};
use secrecy::{ExposeSecret, Secret};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::env_template;
use crate::fs_ops::FileSystem;
use crate::output::Output;
use crate::prompts::{resolve, resolve_secret, Prompter};
use crate::stripe_api::StripeApi;
use crate::stripe_check;
use crate::system::SystemCommand;

pub const ENV_PATH: &str = "/opt/coterie/.env";
pub const ENV_LIVE_PATH: &str = "/opt/coterie/.env.live";
pub const ENV_NEW_PATH: &str = "/opt/coterie/.env.new";
pub const COTERIE_DB_PATH: &str = "/var/lib/coterie/coterie.db";
pub const COTERIE_TEST_DB_PATH: &str = "/var/lib/coterie/coterie-test.db";
pub const ARCHIVE_DIR: &str = "/var/lib/coterie";
const HEALTH_URL: &str = "http://127.0.0.1:8080/health";

/// Filesystem paths the switchover touches. Production uses the
/// well-known `/opt/coterie/...` + `/var/lib/coterie/...` paths;
/// integration tests inject tempfile-backed paths so the sqlite ops
/// can run end-to-end against real files.
#[derive(Debug, Clone)]
pub struct Paths {
    pub env: PathBuf,
    pub env_live: PathBuf,
    pub env_new: PathBuf,
    pub coterie_db: PathBuf,
    pub coterie_test_db: PathBuf,
    pub archive_dir: PathBuf,
}

impl Paths {
    pub fn defaults() -> Self {
        Self {
            env: PathBuf::from(ENV_PATH),
            env_live: PathBuf::from(ENV_LIVE_PATH),
            env_new: PathBuf::from(ENV_NEW_PATH),
            coterie_db: PathBuf::from(COTERIE_DB_PATH),
            coterie_test_db: PathBuf::from(COTERIE_TEST_DB_PATH),
            archive_dir: PathBuf::from(ARCHIVE_DIR),
        }
    }
}

impl Default for Paths {
    fn default() -> Self {
        Self::defaults()
    }
}

/// CLI/env-resolved inputs for the subcommand. Mirrors `InstallArgs`'s
/// shape so `main.rs` converts from clap once and hands this in.
#[derive(Debug, Clone, Default)]
pub struct SwitchArgs {
    pub discard_test_db: bool,
    pub yes: bool,
    pub no_prompt: bool,
    pub live_pk: Option<String>,
    pub live_sk: Option<secrecy::SecretString>,
    pub live_whsec: Option<secrecy::SecretString>,
}

/// Configurable predicate so tests can skip the `EUID == 0` enforcement
/// without globally trampling `is_root()`.
pub trait RootCheck {
    fn must_be_root(&self) -> bool;
}

pub struct RealRootCheck;

impl RootCheck for RealRootCheck {
    fn must_be_root(&self) -> bool {
        true
    }
}

/// Test helper: pretends not-root never matters.
pub struct SkipRootCheck;

impl RootCheck for SkipRootCheck {
    fn must_be_root(&self) -> bool {
        false
    }
}

extern "C" {
    fn geteuid() -> u32;
}

fn is_root() -> bool {
    // SAFETY: geteuid is a thread-safe POSIX call with no preconditions.
    unsafe { geteuid() == 0 }
}

/// Top-level entry point. Returns an exit-code-friendly `Result`: Ok(0)
/// for success-or-already-done, Ok(N>0) for "this run did nothing
/// destructive but the operator should look at it", Err for failures.
pub fn run<S, F, A, P, O, R>(
    args: SwitchArgs,
    sys: &S,
    fs: &F,
    api: &A,
    prompts: &P,
    output: &O,
    root: &R,
) -> Result<i32>
where
    S: SystemCommand,
    F: FileSystem,
    A: StripeApi,
    P: Prompter,
    O: Output,
    R: RootCheck,
{
    run_with_paths(
        args,
        sys,
        fs,
        api,
        prompts,
        output,
        root,
        &Paths::defaults(),
    )
}

/// Like [`run`] but with injectable filesystem paths. Production calls
/// `run` (which delegates here with `Paths::defaults()`); integration
/// tests inject `Paths` pointing at tempdirs so the rusqlite ops can
/// hit real files.
#[allow(clippy::too_many_arguments)]
pub fn run_with_paths<S, F, A, P, O, R>(
    args: SwitchArgs,
    sys: &S,
    fs: &F,
    api: &A,
    prompts: &P,
    output: &O,
    root: &R,
    paths: &Paths,
) -> Result<i32>
where
    S: SystemCommand,
    F: FileSystem,
    A: StripeApi,
    P: Prompter,
    O: Output,
    R: RootCheck,
{
    // --- 1. Preflight: root + idempotency -----------------------------
    if root.must_be_root() && !is_root() {
        return Err(anyhow!(
            "coterie-provision switch-stripe-to-live must run as root (try sudo)"
        ));
    }

    if !fs.exists(&paths.env) {
        return Err(anyhow!(
            "{} does not exist — this command is for upgrading an existing test-mode install, not bootstrapping",
            paths.env.display()
        ));
    }
    let current_env = fs.read_to_string(&paths.env)?;
    if has_live_pk(&current_env) {
        output.println("Already in live mode; nothing to do.");
        return Ok(0);
    }

    if !fs.exists(&paths.coterie_test_db) {
        return Err(anyhow!(
            "Not in test mode; no test DB to migrate from. Expected: {}",
            paths.coterie_test_db.display()
        ));
    }

    // --- 2. Load live creds (file → flags/env/prompt) -----------------
    let creds = load_live_creds(&args, fs, prompts, &paths.env_live)?;

    // --- 3. Stripe smoke test (abort BEFORE any mutation) -------------
    api.check_balance(&creds.sk).map_err(|e| {
        anyhow!("Stripe rejected the live secret key — aborting before any modifications.\n  {e}")
    })?;

    // --- 4. Confirmation prompt ---------------------------------------
    print_plan(output, args.discard_test_db);
    if !args.yes && !args.no_prompt {
        let proceed = prompts.prompt_yn("Proceed with the switchover?", false)?;
        if !proceed {
            return Err(anyhow!("switchover aborted by operator"));
        }
    }

    // --- 5. systemctl stop coterie ------------------------------------
    let out = sys.run("systemctl", &["stop", "coterie"])?;
    if !out.success() {
        return Err(anyhow!(
            "systemctl stop coterie failed (exit {}): {}\n{}",
            out.status,
            out.stdout,
            out.stderr
        ));
    }

    // --- 6. Create fresh coterie.db with the same schema --------------
    create_live_db_with_test_schema(&paths.coterie_test_db, &paths.coterie_db)
        .context("creating fresh coterie.db with migrated schema")?;

    // --- 7. Copy admin row(s) via ATTACH DATABASE ---------------------
    copy_admin_rows(&paths.coterie_test_db, &paths.coterie_db)
        .context("copying admin rows from coterie-test.db to coterie.db")?;

    // --- 8. Archive or discard the test DB ----------------------------
    if args.discard_test_db {
        fs.remove_file(&paths.coterie_test_db)?;
        output.println(&format!("discarded {}", paths.coterie_test_db.display()));
    } else {
        let archive = archive_name();
        let archive_path = paths.archive_dir.join(&archive);
        fs.rename(&paths.coterie_test_db, &archive_path)?;
        output.println(&format!("archived test DB → {}", archive_path.display()));
    }

    // --- 9. Atomic .env rewrite ---------------------------------------
    let new_env = rewrite_env(&current_env, &creds.pk, &creds.sk, &creds.whsec);
    fs.write(&paths.env_new, new_env.as_bytes())?;
    fs.chmod(&paths.env_new, 0o640)?;
    fs.chown(&paths.env_new, "coterie", "coterie").ok();
    fs.rename(&paths.env_new, &paths.env)?;

    // --- 10. Remove .env.live if it existed ---------------------------
    if fs.exists(&paths.env_live) {
        fs.remove_file(&paths.env_live)?;
        output.println(&format!(
            "removed {} (live creds now in .env)",
            paths.env_live.display()
        ));
    }

    // --- 11. systemctl start + poll is-active -------------------------
    let out = sys.run("systemctl", &["start", "coterie"])?;
    if !out.success() {
        dump_diagnostics(sys, output);
        return Err(anyhow!(
            "systemctl start coterie failed (exit {}): {}",
            out.status,
            out.stderr
        ));
    }

    if !poll_is_active(sys)? {
        dump_diagnostics(sys, output);
        return Err(anyhow!(
            "coterie service did not reach active state within 30s. .env is live-mode but the service is down — debug with `journalctl -u coterie`"
        ));
    }

    // --- 12. HTTP smoke test ------------------------------------------
    smoke_test_health(sys, output)?;

    // --- 13. Success summary + webhook reminder -----------------------
    print_success(output, &new_env);

    Ok(0)
}

/// Live credentials, all wrapped/owned. We use `Secret` for sk/whsec
/// so accidental `Debug` printing doesn't leak them.
struct LiveCreds {
    pk: String,
    sk: Secret<String>,
    whsec: Secret<String>,
}

fn load_live_creds<F: FileSystem, P: Prompter>(
    args: &SwitchArgs,
    fs: &F,
    prompts: &P,
    live_path: &Path,
) -> Result<LiveCreds> {
    // If the file exists, parse it and pull the three keys from it.
    if fs.exists(live_path) {
        let body = fs.read_to_string(live_path)?;
        let parsed = parse_env_pairs(&body);
        let pk = parsed
            .get("COTERIE__STRIPE__PUBLISHABLE_KEY")
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "{} missing COTERIE__STRIPE__PUBLISHABLE_KEY",
                    live_path.display()
                )
            })?;
        let sk_str = parsed
            .get("COTERIE__STRIPE__SECRET_KEY")
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "{} missing COTERIE__STRIPE__SECRET_KEY",
                    live_path.display()
                )
            })?;
        let whsec_str = parsed
            .get("COTERIE__STRIPE__WEBHOOK_SECRET")
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "{} missing COTERIE__STRIPE__WEBHOOK_SECRET",
                    live_path.display()
                )
            })?;
        stripe_check::validate_prefix(&pk, "pk_live_")?;
        stripe_check::validate_prefix(&sk_str, "sk_live_")?;
        stripe_check::validate_prefix(&whsec_str, "whsec_")?;
        return Ok(LiveCreds {
            pk,
            sk: Secret::new(sk_str),
            whsec: Secret::new(whsec_str),
        });
    }

    // No .env.live; resolve via flag/env/prompt.
    let pk = resolve(
        "live-pk",
        "COTERIE_PROVISION_STRIPE_LIVE_PK",
        args.live_pk.clone(),
        None,
        args.no_prompt,
        || prompts.prompt_text("Stripe LIVE publishable key (pk_live_…)", None),
    )?;
    stripe_check::validate_prefix(&pk, "pk_live_")?;

    let sk = resolve_secret(
        "live-sk",
        "COTERIE_PROVISION_STRIPE_LIVE_SK",
        args.live_sk.clone(),
        args.no_prompt,
        || prompts.prompt_secret("Stripe LIVE secret key (sk_live_…) — input hidden"),
    )?;
    stripe_check::validate_prefix(sk.expose_secret(), "sk_live_")?;

    let whsec = resolve_secret(
        "live-whsec",
        "COTERIE_PROVISION_STRIPE_LIVE_WHSEC",
        args.live_whsec.clone(),
        args.no_prompt,
        || prompts.prompt_secret("Stripe LIVE webhook signing secret (whsec_…)"),
    )?;
    stripe_check::validate_prefix(whsec.expose_secret(), "whsec_")?;

    Ok(LiveCreds {
        pk,
        sk: Secret::new(sk.expose_secret().clone()),
        whsec: Secret::new(whsec.expose_secret().clone()),
    })
}

/// Returns true iff any non-comment line in `env` has a value starting
/// with `pk_live_` for `COTERIE__STRIPE__PUBLISHABLE_KEY`.
pub fn has_live_pk(env: &str) -> bool {
    for line in env.lines() {
        let l = line.trim_start();
        if l.starts_with('#') {
            continue;
        }
        if let Some(rest) = l.strip_prefix("COTERIE__STRIPE__PUBLISHABLE_KEY=") {
            if rest.trim().starts_with("pk_live_") {
                return true;
            }
        }
    }
    false
}

/// Minimal dotenv-style parser. Supports `KEY=VALUE` lines, ignores
/// comments and blank lines, strips surrounding single or double quotes
/// from the value. Adequate for the constrained shape of `.env.live`.
pub fn parse_env_pairs(input: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for raw in input.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(eq) = line.find('=') else { continue };
        let key = line[..eq].trim().to_string();
        let mut value = line[eq + 1..].trim().to_string();
        let quoted_double = value.starts_with('"') && value.ends_with('"');
        let quoted_single = value.starts_with('\'') && value.ends_with('\'');
        if (quoted_double || quoted_single) && value.len() >= 2 {
            value = value[1..value.len() - 1].to_string();
        }
        out.insert(key, value);
    }
    out
}

/// Pure function — substitutes the three Stripe lines and the
/// DATABASE_URL line. Lines outside that set pass through verbatim.
pub fn rewrite_env(
    current: &str,
    live_pk: &str,
    live_sk: &Secret<String>,
    live_whsec: &Secret<String>,
) -> String {
    let mut out = String::with_capacity(current.len());
    for line in current.split_inclusive('\n') {
        let body = line.trim_end_matches('\n').trim_end_matches('\r');
        let replaced = rewrite_line(body, live_pk, live_sk, live_whsec);
        match replaced {
            Some(new_body) => {
                out.push_str(&new_body);
                out.push('\n');
            }
            None => out.push_str(line),
        }
    }
    // If the source had no trailing newline, neither should our output.
    if !current.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    out
}

fn rewrite_line(
    body: &str,
    live_pk: &str,
    live_sk: &Secret<String>,
    live_whsec: &Secret<String>,
) -> Option<String> {
    if body.starts_with('#') {
        return None;
    }
    let eq = body.find('=')?;
    let key = &body[..eq];
    match key {
        "COTERIE__STRIPE__PUBLISHABLE_KEY" => Some(format!("{key}={live_pk}")),
        "COTERIE__STRIPE__SECRET_KEY" => Some(format!("{key}={}", live_sk.expose_secret())),
        "COTERIE__STRIPE__WEBHOOK_SECRET" => Some(format!("{key}={}", live_whsec.expose_secret())),
        "COTERIE__DATABASE__URL" => Some(format!(
            "{key}={}",
            env_template::DatabaseUrl::Live.as_env_str()
        )),
        _ => None,
    }
}

/// Returns `coterie-test-archive-YYYYMMDD-HHMMSS.db`. Local time per
/// design D5 — operators reading filesystem listings want their wall
/// clock, not UTC.
pub fn archive_name() -> String {
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    format!("coterie-test-archive-{ts}.db")
}

/// Approach (c) from design.md D9.2: copy the schema from coterie-test.db
/// into the new coterie.db by reading the `sqlite_master` CREATE
/// statements. The two DBs share migration history (same coterie
/// binary), so the schemas match. Faster + simpler than re-running
/// migrations or invoking the coterie binary.
pub fn create_live_db_with_test_schema(test_db: &Path, new_db: &Path) -> Result<()> {
    // If the new DB already exists (rerun of a partial switchover), wipe
    // and start fresh — we're past the idempotency gate at this point.
    if new_db.exists() {
        std::fs::remove_file(new_db)
            .with_context(|| format!("removing stale {}", new_db.display()))?;
    }

    let test_conn = rusqlite::Connection::open(test_db)
        .with_context(|| format!("opening {}", test_db.display()))?;
    let new_conn = rusqlite::Connection::open(new_db)
        .with_context(|| format!("opening (creating) {}", new_db.display()))?;

    // Collect schema statements first to avoid holding the read-lock
    // while we write.
    let mut stmt = test_conn.prepare(
        "SELECT sql FROM sqlite_master \
         WHERE sql IS NOT NULL \
           AND type IN ('table','index','view','trigger') \
           AND name NOT LIKE 'sqlite_%' \
         ORDER BY CASE type WHEN 'table' THEN 1 WHEN 'index' THEN 2 WHEN 'view' THEN 3 WHEN 'trigger' THEN 4 ELSE 5 END",
    )?;
    let rows: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    for ddl in rows {
        new_conn
            .execute_batch(&format!("{ddl};"))
            .with_context(|| format!("applying schema DDL: {ddl}"))?;
    }
    Ok(())
}

/// SQL from design D3 — verbatim. ATTACH the test DB, INSERT the admin
/// row(s), DETACH. rusqlite executes this as a single batch.
pub fn copy_admin_rows(test_db: &Path, new_db: &Path) -> Result<()> {
    let conn = rusqlite::Connection::open(new_db)
        .with_context(|| format!("opening {}", new_db.display()))?;
    let test_path = test_db.to_string_lossy();
    let sql = format!(
        "ATTACH DATABASE '{path}' AS test;\n\
         INSERT INTO members SELECT * FROM test.members WHERE is_admin = 1;\n\
         DETACH DATABASE test;",
        path = test_path.replace('\'', "''"),
    );
    conn.execute_batch(&sql)
        .with_context(|| format!("ATTACH + admin INSERT failed (sql: {sql})"))?;
    Ok(())
}

fn print_plan<O: Output>(output: &O, discard: bool) {
    let archive_or_discard = if discard {
        "discard coterie-test.db"
    } else {
        "archive coterie-test.db to coterie-test-archive-YYYYMMDD-HHMMSS.db"
    };
    output.println("\n===== switch-stripe-to-live: planned actions =====");
    output.println("  1. systemctl stop coterie");
    output.println("  2. create fresh coterie.db with test DB's schema");
    output.println("  3. migrate admin row(s) via ATTACH DATABASE + INSERT...SELECT");
    output.println(&format!("  4. {archive_or_discard}"));
    output.println("  5. rewrite .env (atomic): swap Stripe creds + DATABASE_URL");
    output.println("  6. remove /opt/coterie/.env.live if present");
    output.println("  7. systemctl start coterie + poll is-active (up to 30s)");
    output.println("  8. HTTP GET http://127.0.0.1:8080/health (smoke test)");
    output.println("");
}

/// Polls `systemctl is-active coterie` once per second for up to 30s.
/// Returns true if/when the unit becomes active.
fn poll_is_active<S: SystemCommand>(sys: &S) -> Result<bool> {
    for _ in 0..30 {
        let out = sys.run("systemctl", &["is-active", "coterie"])?;
        if out.stdout.trim() == "active" {
            return Ok(true);
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    Ok(false)
}

fn dump_diagnostics<S: SystemCommand, O: Output>(sys: &S, output: &O) {
    output.println("---- journalctl -u coterie -n 50 --no-pager ----");
    if let Ok(out) = sys.run("journalctl", &["-u", "coterie", "-n", "50", "--no-pager"]) {
        output.println(&out.stdout);
        if !out.stderr.is_empty() {
            output.println(&out.stderr);
        }
    } else {
        output.println("(failed to invoke journalctl)");
    }
}

fn smoke_test_health<S: SystemCommand, O: Output>(sys: &S, output: &O) -> Result<()> {
    // -fsSL flags: fail on HTTP error, silent, show error, follow
    // redirects. With -f we also get non-zero exit on 3xx-with-Location
    // (303 → /setup means admin migration failed).
    let out = sys.run("curl", &["-fsSL", HEALTH_URL])?;
    if out.success() {
        return Ok(());
    }
    dump_diagnostics(sys, output);
    Err(anyhow!(
        "smoke test GET {HEALTH_URL} failed (exit {}): {} — likely admin migration didn't land",
        out.status,
        out.stderr
    ))
}

fn print_success<O: Output>(output: &O, new_env: &str) {
    // Extract base_url from the rewritten .env so the reminder URL is
    // accurate.
    let portal_url = parse_env_pairs(new_env)
        .get("COTERIE__SERVER__BASE_URL")
        .cloned()
        .unwrap_or_else(|| "https://your-coterie-host".to_string());
    let webhook_url = format!("{portal_url}/api/payments/webhook/stripe");
    output.println("\n============================================================");
    output.println("Switched to Stripe LIVE mode.");
    output.println("");
    output.println("IMPORTANT: verify the LIVE-mode webhook endpoint is registered in");
    output.println("your Stripe dashboard:");
    output.println("");
    output.println("  Stripe dashboard → toggle to LIVE mode → Developers → Webhooks");
    output.println("  → confirm an endpoint exists for:");
    output.println(&format!("       {webhook_url}"));
    output.println("  → confirm the signing secret matches the whsec_ value you just");
    output.println("       supplied to this script");
    output.println("");
    output.println("Without a live-mode webhook registered, real charges will go through");
    output.println("Stripe but Coterie will never hear about them — dues will never");
    output.println("extend, payments will never advance from Pending.");
    output.println("============================================================");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_live_pk_detects_live_value() {
        let env = "COTERIE__SERVER__PORT=8080\nCOTERIE__STRIPE__PUBLISHABLE_KEY=pk_live_AAA\n";
        assert!(has_live_pk(env));
    }

    #[test]
    fn has_live_pk_ignores_test_value() {
        let env = "COTERIE__STRIPE__PUBLISHABLE_KEY=pk_test_AAA\n";
        assert!(!has_live_pk(env));
    }

    #[test]
    fn has_live_pk_ignores_commented_line() {
        let env = "# COTERIE__STRIPE__PUBLISHABLE_KEY=pk_live_AAA\n";
        assert!(!has_live_pk(env));
    }

    #[test]
    fn parse_env_pairs_basic() {
        let input = "# comment\n\nFOO=bar\nBAZ=qux\n";
        let m = parse_env_pairs(input);
        assert_eq!(m.get("FOO").map(String::as_str), Some("bar"));
        assert_eq!(m.get("BAZ").map(String::as_str), Some("qux"));
    }

    #[test]
    fn parse_env_pairs_strips_quotes() {
        let input = "FOO=\"bar\"\nBAZ='qux'\n";
        let m = parse_env_pairs(input);
        assert_eq!(m.get("FOO").map(String::as_str), Some("bar"));
        assert_eq!(m.get("BAZ").map(String::as_str), Some("qux"));
    }

    #[test]
    fn rewrite_env_swaps_stripe_and_db_url() {
        let input = "\
COTERIE__SERVER__PORT=8080
COTERIE__STRIPE__ENABLED=true
COTERIE__STRIPE__PUBLISHABLE_KEY=pk_test_old
COTERIE__STRIPE__SECRET_KEY=sk_test_old
COTERIE__STRIPE__WEBHOOK_SECRET=whsec_old
COTERIE__DATABASE__URL=sqlite:///var/lib/coterie/coterie-test.db?mode=rwc
COTERIE__AUTH__SESSION_SECRET=deadbeef
";
        let pk = "pk_live_NEW";
        let sk = Secret::new("sk_live_NEW".to_string());
        let wh = Secret::new("whsec_NEW".to_string());
        let out = rewrite_env(input, pk, &sk, &wh);
        let expected = "\
COTERIE__SERVER__PORT=8080
COTERIE__STRIPE__ENABLED=true
COTERIE__STRIPE__PUBLISHABLE_KEY=pk_live_NEW
COTERIE__STRIPE__SECRET_KEY=sk_live_NEW
COTERIE__STRIPE__WEBHOOK_SECRET=whsec_NEW
COTERIE__DATABASE__URL=sqlite://coterie.db
COTERIE__AUTH__SESSION_SECRET=deadbeef
";
        assert_eq!(out, expected);
    }

    #[test]
    fn rewrite_env_passes_through_unrelated_lines() {
        let input = "FOO=bar\nBAZ=qux\n";
        let out = rewrite_env(
            input,
            "pk_live_x",
            &Secret::new("sk_live_x".to_string()),
            &Secret::new("whsec_x".to_string()),
        );
        assert_eq!(out, input);
    }

    #[test]
    fn rewrite_env_preserves_lack_of_trailing_newline() {
        let input = "COTERIE__STRIPE__PUBLISHABLE_KEY=pk_test_old";
        let out = rewrite_env(
            input,
            "pk_live_new",
            &Secret::new("sk_live_new".to_string()),
            &Secret::new("whsec_new".to_string()),
        );
        assert_eq!(out, "COTERIE__STRIPE__PUBLISHABLE_KEY=pk_live_new");
    }

    #[test]
    fn rewrite_env_skips_commented_lines() {
        let input = "# COTERIE__STRIPE__PUBLISHABLE_KEY=pk_test_old\n";
        let out = rewrite_env(
            input,
            "pk_live_x",
            &Secret::new("sk_live_x".to_string()),
            &Secret::new("whsec_x".to_string()),
        );
        assert_eq!(out, input);
    }

    #[test]
    fn archive_name_has_expected_shape() {
        let n = archive_name();
        assert!(n.starts_with("coterie-test-archive-"));
        assert!(n.ends_with(".db"));
        // YYYYMMDD-HHMMSS = 15 chars, prefix len + suffix len:
        let body = n
            .strip_prefix("coterie-test-archive-")
            .unwrap()
            .strip_suffix(".db")
            .unwrap();
        assert_eq!(body.len(), 15);
        assert!(body.chars().nth(8) == Some('-'));
    }
}
