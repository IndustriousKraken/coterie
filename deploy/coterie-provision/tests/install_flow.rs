//! Integration tests driving `install::run` against the FakeSystem +
//! FakeFs traits. These exercise the end-to-end orchestration without
//! actually spawning subprocesses or touching disk.

use coterie_provision::caddyfile;
use coterie_provision::install::{self, detect_state, InstallArgs};
use coterie_provision::test_support::{FakeFs, FakeSystem, MockPrompter};
use secrecy::SecretString;
use std::path::Path;

fn base_args() -> InstallArgs {
    InstallArgs {
        org_name: Some("Acme".to_string()),
        portal_domain: Some("portal.acme.io".to_string()),
        marketing_domain: None,
        contact_email: Some("ops@acme.io".to_string()),
        admin_email: Some("rab@acme.io".to_string()),
        admin_username: Some("rab".to_string()),
        admin_full_name: Some("R A Bee".to_string()),
        admin_password: Some(SecretString::new("hunter2hunter2".to_string())),
        enable_stripe: Some(false),
        stripe_publishable_key: None,
        stripe_secret_key: None,
        stripe_webhook_secret: None,
        enable_discord: Some(false),
        discord_bot_token: None,
        discord_guild_id: None,
        discord_member_role_id: None,
        discord_expired_role_id: None,
        enable_unifi: Some(false),
        unifi_controller_url: None,
        unifi_username: None,
        unifi_password: None,
        unifi_site_id: None,
        enable_caddy: Some(true),
        version: Some("v1.1.0".to_string()),
        no_prompt: true,
        dry_run: true,
        overwrite_env: false,
    }
}

#[test]
fn dry_run_orchestrates_full_flow_without_side_effects() {
    let args = base_args();
    let sys = FakeSystem::new();
    let fs = FakeFs::new();
    let prompts = MockPrompter::new();
    install::run(args, &sys, &fs, &prompts).expect("dry run succeeds");
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

    let err = install::run(args, &sys, &fs, &prompts).unwrap_err();
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
    install::run(args, &sys, &fs, &prompts).expect("overwrite-env should bypass the prompt");
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
    let err = install::run(args, &sys, &fs, &prompts).unwrap_err();
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
    let err = install::run(args, &sys, &fs, &prompts).unwrap_err();
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
    let err = install::run(args, &sys, &fs, &prompts).unwrap_err();
    assert!(
        err.to_string().contains("admin-password") || err.to_string().contains("ADMIN_PASSWORD"),
        "got: {err}"
    );
}
