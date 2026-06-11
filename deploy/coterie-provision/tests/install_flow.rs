//! Integration tests driving `install::run` against the FakeSystem +
//! FakeFs traits. These exercise the end-to-end orchestration without
//! actually spawning subprocesses or touching disk.

use coterie_provision::caddyfile;
use coterie_provision::checklist::TEST_MODE_CHECKLIST;
use coterie_provision::install::{self, detect_state, InstallArgs, StripeMode};
use coterie_provision::output::CaptureOutput;
use coterie_provision::system::CommandOutput;
use coterie_provision::test_support::{FakeFs, FakeSystem, MockPrompter};
use secrecy::SecretString;
use std::path::Path;
use std::time::{Duration, Instant};

fn base_args() -> InstallArgs {
    InstallArgs {
        org_name: Some("Acme".to_string()),
        portal_domain: Some("portal.acme.io".to_string()),
        contact_email: Some("ops@acme.io".to_string()),
        admin_email: Some("rab@acme.io".to_string()),
        admin_username: Some("rab".to_string()),
        admin_full_name: Some("R A Bee".to_string()),
        admin_password: Some(SecretString::new("hunter2hunter2".to_string())),
        enable_stripe: Some(false),
        enable_discord: Some(false),
        enable_unifi: Some(false),
        enable_caddy: Some(true),
        version: Some("v1.1.0".to_string()),
        no_prompt: true,
        dry_run: true,
        ..Default::default()
    }
}

#[test]
fn dry_run_orchestrates_full_flow_without_side_effects() {
    let args = base_args();
    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    let prompts = MockPrompter::new();
    install::run(args, &sys, &fs, &prompts, &CaptureOutput::new()).expect("dry run succeeds");
    // Dry-run never calls into the system.
    assert_eq!(
        sys.calls.borrow().len(),
        0,
        "dry-run must not invoke any commands"
    );
    // Likewise no fs mutations on the actual destination paths.
    let files = fs.files.borrow();
    assert!(!files.contains_key(Path::new("/opt/coterie/.env")));
    assert!(!files.contains_key(Path::new("/etc/caddy/Caddyfile")));
}

#[test]
fn idempotent_rerun_detects_existing_env_and_caddyfile() {
    let fs = FakeFs::new();

    // Pretend a previous run left a managed Caddyfile and a .env.
    fs.put(
        Path::new("/opt/coterie/.env"),
        b"COTERIE__SERVER__PORT=8080\n",
    );
    fs.put(
        Path::new("/etc/caddy/Caddyfile"),
        format!(
            "{}\nportal.acme.io {{ reverse_proxy 127.0.0.1:8080 }}\n",
            caddyfile::COTERIE_MARKER
        )
        .as_bytes(),
    );

    let state = detect_state(&fs);
    assert!(state.env_present);
    assert!(state.caddyfile_present);
    assert!(state.caddyfile_managed_by_us);
}

#[test]
fn no_prompt_with_existing_env_and_no_overwrite_fails_clearly() {
    let mut args = base_args();
    args.overwrite_env = false;
    args.no_prompt = true;
    // Critically NOT dry-run — we want gather_inputs to actually check
    // .env state. But that still requires non-dry-run pathing. We'll
    // route via gather_inputs by calling install::run with dry_run=true
    // and pre-populating .env to test the policy.
    args.dry_run = true;

    let fs = FakeFs::new();
    fs.put(Path::new("/opt/coterie/.env"), b"existing=value\n");

    let sys = FakeSystem::new();
    let prompts = MockPrompter::new();
    std::env::remove_var("COTERIE_PROVISION_OVERWRITE_ENV");

    let err = install::run(args, &sys, &fs, &prompts, &CaptureOutput::new()).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("overwrite") || msg.contains("OVERWRITE"),
        "expected overwrite guidance in error, got: {msg}"
    );
}

#[test]
fn overwrite_env_flag_or_envvar_allows_clobber() {
    let mut args = base_args();
    args.overwrite_env = true;

    let fs = FakeFs::new();
    fs.put(Path::new("/opt/coterie/.env"), b"existing=value\n");

    let sys = FakeSystem::new();
    let prompts = MockPrompter::new();
    install::run(args, &sys, &fs, &prompts, &CaptureOutput::new())
        .expect("overwrite-env should bypass the prompt");
}

#[test]
fn stripe_enabled_requires_correctly_prefixed_keys() {
    let mut args = base_args();
    args.enable_stripe = Some(true);
    args.stripe_publishable_key = Some("not_a_stripe_key".to_string());
    args.stripe_secret_key = Some(SecretString::new("sk_test_x".to_string()));
    args.stripe_webhook_secret = Some(SecretString::new("whsec_x".to_string()));

    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    let prompts = MockPrompter::new();
    let err = install::run(args, &sys, &fs, &prompts, &CaptureOutput::new()).unwrap_err();
    let m = err.to_string();
    assert!(m.contains("pk_test_") || m.contains("pk_live_"));
}

#[test]
fn discord_enabled_requires_token_and_role_ids() {
    let mut args = base_args();
    args.enable_discord = Some(true);
    // No bot token supplied — should fail.

    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    let prompts = MockPrompter::new();
    let err = install::run(args, &sys, &fs, &prompts, &CaptureOutput::new()).unwrap_err();
    assert!(err.to_string().contains("discord-bot-token") || err.to_string().contains("DISCORD"));
}

#[test]
fn missing_admin_password_in_no_prompt_mode_fails() {
    let mut args = base_args();
    args.admin_password = None;
    std::env::remove_var("COTERIE_PROVISION_ADMIN_PASSWORD");

    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    let prompts = MockPrompter::new();
    let err = install::run(args, &sys, &fs, &prompts, &CaptureOutput::new()).unwrap_err();
    assert!(
        err.to_string().contains("admin-password") || err.to_string().contains("ADMIN_PASSWORD"),
        "got: {err}"
    );
}

// ---------------------------------------------------------------------
// a25: test-mode wizard coverage
// ---------------------------------------------------------------------

/// Stages enough state on `FakeFs` so a non-dry-run install can pass
/// `assert_binaries_present` and so the .env.example template is
/// available to `render_and_write_env`.
fn stage_fake_install_state(fs: &FakeFs) {
    fs.put(Path::new("/opt/coterie/coterie"), b"binary");
    fs.put(Path::new("/opt/coterie/create_admin"), b"binary");
    fs.put(
        Path::new("/opt/coterie/.env.example"),
        include_str!("fixtures/env_example.txt").as_bytes(),
    );
}

fn test_mode_args() -> InstallArgs {
    let mut args = base_args();
    args.enable_stripe = Some(true);
    args.stripe_mode = Some(StripeMode::Test);
    args.stripe_publishable_key = Some("pk_test_abc".to_string());
    args.stripe_secret_key = Some(SecretString::new("sk_test_xyz".to_string()));
    args.stripe_webhook_secret = Some(SecretString::new("whsec_zzz".to_string()));
    args.enable_caddy = Some(false); // skip Caddyfile rendering for these tests
    args.dry_run = false;
    args.overwrite_env = true;
    args.skip_root_check = true;
    args
}

#[test]
fn test_mode_emits_checklist_to_output() {
    let args = test_mode_args();
    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    stage_fake_install_state(&fs);
    let prompts = MockPrompter::new();
    let out = CaptureOutput::new();
    install::run(args, &sys, &fs, &prompts, &out).expect("test-mode install succeeds");
    assert!(
        out.contains(TEST_MODE_CHECKLIST),
        "test mode must emit the verification checklist; lines: {:?}",
        out.lines.borrow()
    );
}

#[test]
fn live_mode_does_not_emit_checklist() {
    let mut args = base_args();
    args.enable_stripe = Some(true);
    args.stripe_mode = Some(StripeMode::Live);
    args.stripe_publishable_key = Some("pk_live_abc".to_string());
    args.stripe_secret_key = Some(SecretString::new("sk_live_xyz".to_string()));
    args.stripe_webhook_secret = Some(SecretString::new("whsec_zzz".to_string()));
    args.enable_caddy = Some(false);
    args.dry_run = false;
    args.overwrite_env = true;
    args.skip_root_check = true;

    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    stage_fake_install_state(&fs);
    let prompts = MockPrompter::new();
    let out = CaptureOutput::new();
    install::run(args, &sys, &fs, &prompts, &out).expect("live-mode install succeeds");
    assert!(
        !out.contains(TEST_MODE_CHECKLIST),
        "live mode must NOT emit the test-mode checklist; lines: {:?}",
        out.lines.borrow()
    );
}

#[test]
fn test_mode_renders_env_with_coterie_test_db_url() {
    let args = test_mode_args();
    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    stage_fake_install_state(&fs);
    let prompts = MockPrompter::new();
    let out = CaptureOutput::new();
    install::run(args, &sys, &fs, &prompts, &out).expect("test-mode install succeeds");

    let env_bytes = fs
        .get(Path::new("/opt/coterie/.env"))
        .expect(".env should be written");
    let env_str = String::from_utf8(env_bytes).unwrap();
    assert!(
        env_str
            .contains("COTERIE__DATABASE__URL=sqlite:///var/lib/coterie/coterie-test.db?mode=rwc"),
        "test-mode .env should reference coterie-test.db; got:\n{env_str}"
    );
    assert!(
        env_str.contains("COTERIE__STRIPE__PUBLISHABLE_KEY=pk_test_abc"),
        "test-mode .env should hold the test publishable key; got:\n{env_str}"
    );
    // No .env.live should have been created because operator did not
    // pre-load live creds.
    assert!(
        fs.get(Path::new("/opt/coterie/.env.live")).is_none(),
        ".env.live must not exist when live creds were not pre-loaded"
    );
}

#[test]
fn test_mode_with_live_preload_writes_env_live() {
    let mut args = test_mode_args();
    args.preload_live_creds = Some(true);
    args.stripe_live_publishable_key = Some("pk_live_AAA".to_string());
    args.stripe_live_secret_key = Some(SecretString::new("sk_live_BBB".to_string()));
    args.stripe_live_webhook_secret = Some(SecretString::new("whsec_CCC".to_string()));

    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    stage_fake_install_state(&fs);
    let prompts = MockPrompter::new();
    let out = CaptureOutput::new();
    install::run(args, &sys, &fs, &prompts, &out).expect("test-mode install succeeds");

    let live_bytes = fs
        .get(Path::new("/opt/coterie/.env.live"))
        .expect(".env.live should be written when live creds were pre-loaded");
    let live_str = String::from_utf8(live_bytes).unwrap();
    assert!(live_str.contains("COTERIE__STRIPE__PUBLISHABLE_KEY=pk_live_AAA"));
    assert!(live_str.contains("COTERIE__STRIPE__SECRET_KEY=sk_live_BBB"));
    assert!(live_str.contains("COTERIE__STRIPE__WEBHOOK_SECRET=whsec_CCC"));
}

#[test]
fn live_mode_does_not_write_env_live_and_keeps_default_db() {
    let mut args = base_args();
    args.enable_stripe = Some(true);
    args.stripe_mode = Some(StripeMode::Live);
    args.stripe_publishable_key = Some("pk_live_xxx".to_string());
    args.stripe_secret_key = Some(SecretString::new("sk_live_xxx".to_string()));
    args.stripe_webhook_secret = Some(SecretString::new("whsec_xxx".to_string()));
    args.enable_caddy = Some(false);
    args.dry_run = false;
    args.overwrite_env = true;
    args.skip_root_check = true;

    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    stage_fake_install_state(&fs);
    let prompts = MockPrompter::new();
    let out = CaptureOutput::new();
    install::run(args, &sys, &fs, &prompts, &out).expect("live install succeeds");

    let env_bytes = fs.get(Path::new("/opt/coterie/.env")).unwrap();
    let env_str = String::from_utf8(env_bytes).unwrap();
    assert!(
        env_str.contains("COTERIE__DATABASE__URL=sqlite://coterie.db"),
        "live mode should keep the default database URL"
    );
    assert!(
        fs.get(Path::new("/opt/coterie/.env.live")).is_none(),
        "live mode never creates .env.live"
    );
    assert!(
        fs.get(Path::new("/var/lib/coterie/coterie-test.db"))
            .is_none(),
        "live mode never creates coterie-test.db"
    );
}

#[test]
fn test_mode_rejects_wrong_prefix() {
    let mut args = test_mode_args();
    // Test mode requires `pk_test_`; pass a live one to trigger.
    args.stripe_publishable_key = Some("pk_live_oops".to_string());

    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    stage_fake_install_state(&fs);
    let prompts = MockPrompter::new();
    let err = install::run(args, &sys, &fs, &prompts, &CaptureOutput::new()).unwrap_err();
    assert!(
        err.to_string().contains("pk_test_"),
        "expected pk_test_ in error, got: {err}"
    );
}

// ---------------------------------------------------------------------
// a34: tightened error handling
// ---------------------------------------------------------------------

/// Base non-dry-run args without Stripe/Discord/UniFi/Caddy noise so
/// the install path executes end-to-end and surfaces a single failure
/// at a time. Smoke-test timing is overridden for fast runs.
fn live_run_args() -> InstallArgs {
    let mut args = base_args();
    args.enable_caddy = Some(false);
    args.dry_run = false;
    args.overwrite_env = true;
    args.skip_root_check = true;
    // Keep the smoke-test polling loop fast in tests. Production
    // defaults (1s interval / 30s budget) are far too slow.
    args.smoke_test_interval = Some(Duration::from_millis(1));
    args.smoke_test_budget = Some(Duration::from_millis(50));
    args
}

#[test]
fn chown_failure_aborts_wizard() {
    let args = live_run_args();
    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    stage_fake_install_state(&fs);
    // Configure FakeFs to fail the chown on `.env`.
    fs.fail_chown_on(Path::new("/opt/coterie/.env"));

    let prompts = MockPrompter::new();
    let err = install::run(args, &sys, &fs, &prompts, &CaptureOutput::new()).unwrap_err();

    // anyhow chains the inner FakeFs error under the with_context
    // wrapper. Both `chown` (from the wrapper) and `.env` (from both
    // the wrapper and the inner error) must appear somewhere in the
    // formatted chain.
    let chain = format!("{err:#}");
    assert!(
        chain.contains("chown"),
        "error chain must mention `chown`; got: {chain}"
    );
    assert!(
        chain.contains(".env"),
        "error chain must mention `.env`; got: {chain}"
    );
}

#[test]
fn unexpected_create_admin_code_aborts() {
    let args = live_run_args();
    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    stage_fake_install_state(&fs);
    // create_admin's args include a dynamic tempfile path, so match
    // by command name.
    sys.respond_to_cmd(
        "/opt/coterie/create_admin",
        CommandOutput {
            status: 3,
            stdout: String::new(),
            stderr: "validation failed (hypothetical)".to_string(),
        },
    );

    let prompts = MockPrompter::new();
    let err = install::run(args, &sys, &fs, &prompts, &CaptureOutput::new()).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("unexpectedly"),
        "error message must contain `unexpectedly`; got: {msg}"
    );
    assert!(
        msg.contains('3'),
        "error message must contain the exit code `3`; got: {msg}"
    );
}

#[test]
fn smoke_test_retries_through_startup() {
    let args = live_run_args();
    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    stage_fake_install_state(&fs);
    // First 2 curl /health calls fail (status 7 = connection refused),
    // 3rd returns 200.
    sys.respond_to_sequence(
        "curl",
        &["-fsSL", "http://127.0.0.1:8080/health"],
        vec![
            CommandOutput {
                status: 7,
                stdout: String::new(),
                stderr: "curl: (7) Failed to connect".to_string(),
            },
            CommandOutput {
                status: 7,
                stdout: String::new(),
                stderr: "curl: (7) Failed to connect".to_string(),
            },
            CommandOutput {
                status: 0,
                stdout: "{\"status\":\"ok\"}".to_string(),
                stderr: String::new(),
            },
        ],
    );

    let prompts = MockPrompter::new();
    install::run(args, &sys, &fs, &prompts, &CaptureOutput::new())
        .expect("wizard should succeed once /health comes up");

    let curl_calls = sys
        .calls
        .borrow()
        .iter()
        .filter(|c| c.cmd == "curl")
        .count();
    assert!(
        curl_calls >= 3,
        "expected at least 3 curl calls; got {curl_calls}"
    );
}

#[test]
fn smoke_test_fails_after_budget_with_last_error() {
    let args = live_run_args();
    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    stage_fake_install_state(&fs);
    // Every curl call returns a 500. With -f, curl exits 22 on >=400.
    sys.respond_to(
        "curl",
        &["-fsSL", "http://127.0.0.1:8080/health"],
        CommandOutput {
            status: 22,
            stdout: String::new(),
            stderr: "curl: (22) The requested URL returned error: 500".to_string(),
        },
    );

    let started = Instant::now();
    let prompts = MockPrompter::new();
    let err = install::run(args, &sys, &fs, &prompts, &CaptureOutput::new()).unwrap_err();
    let elapsed = started.elapsed();

    // Confirm the test-friendly budget (50ms) was honored; this would
    // be 30s in production. Allow generous slack for scheduling.
    assert!(
        elapsed < Duration::from_secs(5),
        "test budget must be honored; elapsed = {elapsed:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("500"),
        "error message must surface the 500 from the last attempt; got: {msg}"
    );
    assert!(
        msg.contains("smoke test failed"),
        "error message must say smoke test failed; got: {msg}"
    );
}
