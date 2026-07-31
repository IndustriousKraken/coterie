//! The install wizard, split by collaborator: [`inputs`] is the pure
//! operator interview (it talks to `Prompter` and nothing else), and
//! [`executor`] is the privileged box mutation (it talks to
//! `SystemCommand`/`FileSystem`/`Output` and nothing else). What stays
//! here is the CLI-facing shape and the orchestrator [`run`] that reads
//! state, gathers, prints the summary, and executes.

use anyhow::{anyhow, Result};
use secrecy::SecretString;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use crate::caddyfile;
use crate::checklist::TEST_MODE_CHECKLIST;
use crate::fs_ops::FileSystem;
use crate::output::Output;
use crate::prompts::Prompter;
use crate::system::SystemCommand;

mod executor;
mod inputs;

use executor::Executor;
use inputs::gather_inputs;

// The crate's public surface is unchanged by the split.
pub use executor::smoke_test;
pub use inputs::ResolvedInputs;

pub(super) const INSTALL_DIR: &str = "/opt/coterie";
/// Auto-detect default data dir (matches install.sh + config's
/// default_data_dir). The SQLite DB and uploads live here.
pub(super) const DATA_DIR: &str = "/var/lib/coterie";
pub(super) const ENV_PATH: &str = "/opt/coterie/.env";
pub(crate) const ENV_LIVE_PATH: &str = "/opt/coterie/.env.live";
pub(super) const ENV_EXAMPLE_PATH: &str = "/opt/coterie/.env.example";
pub(super) const CADDYFILE_PATH: &str = "/etc/caddy/Caddyfile";
pub(super) const CADDYFILE_EXAMPLE_PATH: &str = "/opt/coterie/deploy/Caddyfile.example";
pub(super) const CADDY_LOG_DIR: &str = "/var/log/caddy";
pub(super) const RELEASE_DEPLOY_PATH: &str = "/usr/local/bin/coterie-release-deploy";

/// Production per-iteration sleep between `/health` polls.
pub(crate) const SMOKE_TEST_INTERVAL: Duration = Duration::from_secs(1);
/// Production total budget for the `/health` poll loop.
pub(crate) const SMOKE_TEST_BUDGET: Duration = Duration::from_secs(30);

/// Which Stripe mode the wizard is configuring for. Default: `Live`
/// (matches a24 baseline behavior).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripeMode {
    Test,
    Live,
}

impl StripeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            StripeMode::Test => "test",
            StripeMode::Live => "live",
        }
    }
}

impl FromStr for StripeMode {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "test" => Ok(StripeMode::Test),
            "live" => Ok(StripeMode::Live),
            other => Err(anyhow!(
                "invalid stripe mode `{other}` — expected `test` or `live`"
            )),
        }
    }
}

/// Parsed CLI inputs (independent of clap so the install flow is
/// straightforward to unit test). `main.rs` converts the clap struct
/// into this.
#[derive(Debug, Clone, Default)]
pub struct InstallArgs {
    pub org_name: Option<String>,
    pub portal_domain: Option<String>,
    pub marketing_domain: Option<String>,
    pub contact_email: Option<String>,
    pub admin_email: Option<String>,
    pub admin_username: Option<String>,
    pub admin_full_name: Option<String>,
    pub admin_password: Option<SecretString>,

    pub enable_stripe: Option<bool>,
    /// Test or live mode. Defaults to `Live` when unset and `no_prompt`
    /// is on (matches a24 baseline). In interactive mode the operator is
    /// prompted.
    pub stripe_mode: Option<StripeMode>,
    pub stripe_publishable_key: Option<String>,
    pub stripe_secret_key: Option<SecretString>,
    pub stripe_webhook_secret: Option<SecretString>,
    /// If test mode is selected, this flag (or programmatic supply of
    /// the three `stripe_live_*` fields below) opts in to staging live
    /// credentials in `/opt/coterie/.env.live` for the switchover.
    pub preload_live_creds: Option<bool>,
    pub stripe_live_publishable_key: Option<String>,
    pub stripe_live_secret_key: Option<SecretString>,
    pub stripe_live_webhook_secret: Option<SecretString>,

    pub enable_discord: Option<bool>,
    pub discord_bot_token: Option<SecretString>,
    pub discord_guild_id: Option<String>,
    pub discord_member_role_id: Option<String>,
    pub discord_expired_role_id: Option<String>,

    pub enable_unifi: Option<bool>,
    pub unifi_controller_url: Option<String>,
    pub unifi_username: Option<String>,
    pub unifi_password: Option<SecretString>,
    pub unifi_site_id: Option<String>,

    pub enable_caddy: Option<bool>,
    pub version: Option<String>,
    pub no_prompt: bool,
    pub dry_run: bool,
    /// If true, fail fast when state would be overwritten rather than
    /// prompting. Honors `COTERIE_PROVISION_OVERWRITE_ENV=true` to flip
    /// the behavior to silent-overwrite.
    pub overwrite_env: bool,
    /// Test-only escape hatch. The CLI never sets this. Production code
    /// always enforces the root check; integration tests against
    /// `FakeFs`/`FakeSystem` set it true so they can exercise the full
    /// write path without being root.
    #[doc(hidden)]
    pub skip_root_check: bool,
    /// Test-only override for `smoke_test`'s per-iteration sleep.
    /// Production uses `SMOKE_TEST_INTERVAL` (1s) when this is None.
    #[doc(hidden)]
    pub smoke_test_interval: Option<Duration>,
    /// Test-only override for `smoke_test`'s total budget.
    /// Production uses `SMOKE_TEST_BUDGET` (30s) when this is None.
    #[doc(hidden)]
    pub smoke_test_budget: Option<Duration>,
}

/// Idempotency findings detected at the start of the install.
#[derive(Debug, Clone, Default)]
pub struct PreflightState {
    pub env_present: bool,
    pub caddyfile_present: bool,
    pub caddyfile_managed_by_us: bool,
}

/// The orchestrator. Parametric over `SystemCommand` + `FileSystem` +
/// `Output` so the test suite can drive it end-to-end with fakes.
///
/// `Output` carries the test-mode verification checklist (and any other
/// "must be assertable" lines) so integration tests can confirm the
/// text was emitted without scraping the real process stdout.
pub fn run<S: SystemCommand, F: FileSystem, P: Prompter, O: Output>(
    args: InstallArgs,
    sys: &S,
    fs: &F,
    prompts: &P,
    output: &O,
) -> Result<()> {
    // --- Preflight ----------------------------------------------------
    if !args.dry_run && !args.skip_root_check {
        // Root check. The test path passes dry_run = true so this is
        // skipped; the prod path enforces it.
        if !is_root() {
            return Err(anyhow!(
                "coterie-provision install must run as root (try sudo)"
            ));
        }
    }

    let preflight = detect_state(fs);

    // --- Gather inputs ------------------------------------------------
    let inputs = gather_inputs(&args, prompts, &preflight)?;

    // Print plan summary.
    print_summary(&inputs, args.dry_run);

    if !args.no_prompt && !args.dry_run {
        let proceed = prompts.prompt_yn("Proceed with the install with the values above?", true)?;
        if !proceed {
            return Err(anyhow!("install aborted by operator"));
        }
    }

    // --- Execute -----------------------------------------------------
    let exec = Executor {
        sys,
        fs,
        dry_run: args.dry_run,
        smoke_test_interval: args.smoke_test_interval.unwrap_or(SMOKE_TEST_INTERVAL),
        smoke_test_budget: args.smoke_test_budget.unwrap_or(SMOKE_TEST_BUDGET),
    };

    exec.apt_update()?;
    exec.apt_install(inputs.enable_caddy)?;
    exec.fetch_release_deploy(&inputs.version)?;
    exec.run_release_deploy(&inputs.version)?;
    exec.assert_binaries_present()?;
    exec.render_and_write_env(&inputs)?;
    exec.write_live_overlay_if_needed(&inputs)?;
    exec.bootstrap_admin(&inputs)?;
    exec.chown_data_dir()?;
    if inputs.enable_caddy {
        exec.write_caddyfile(&inputs)?;
    }
    exec.enable_and_start_service()?;
    exec.smoke_test()?;

    // Test-mode wizard prints the verification checklist before the
    // closing summary. Live mode skips it (matches a24 baseline).
    if inputs.enable_stripe && inputs.stripe_mode == StripeMode::Test {
        output.println(TEST_MODE_CHECKLIST);
    }

    print_exit_summary(&inputs);
    Ok(())
}

extern "C" {
    fn geteuid() -> u32;
}

fn is_root() -> bool {
    // SAFETY: `geteuid` is a thread-safe POSIX call with no preconditions.
    unsafe { geteuid() == 0 }
}

/// Detect existing state so we can prompt before clobbering.
pub fn detect_state<F: FileSystem>(fs: &F) -> PreflightState {
    let env_present = fs.exists(Path::new(ENV_PATH));
    let caddyfile_present = fs.exists(Path::new(CADDYFILE_PATH));
    let caddyfile_managed_by_us = if caddyfile_present {
        fs.read_to_string(Path::new(CADDYFILE_PATH))
            .map(|s| caddyfile::has_coterie_marker(&s))
            .unwrap_or(false)
    } else {
        false
    };
    PreflightState {
        env_present,
        caddyfile_present,
        caddyfile_managed_by_us,
    }
}

fn print_summary(inputs: &ResolvedInputs, dry_run: bool) {
    let banner = if dry_run {
        "===== DRY RUN: planned install ====="
    } else {
        "===== Install plan ====="
    };
    println!("\n{banner}");
    println!("Org:             {}", inputs.org_name);
    println!("Portal domain:   {}", inputs.portal_domain);
    if let Some(m) = inputs.marketing_domain.as_ref() {
        println!("Marketing:       {m}");
    }
    println!("Contact email:   {}", inputs.contact_email);
    println!("Admin email:     {}", inputs.admin_email);
    println!("Admin username:  {}", inputs.admin_username);
    println!("Version:         {}", inputs.version);
    println!(
        "Integrations:    stripe={} discord={} unifi={} caddy={}",
        inputs.enable_stripe, inputs.enable_discord, inputs.enable_unifi, inputs.enable_caddy
    );
    if inputs.enable_stripe {
        println!("Stripe mode:     {}", inputs.stripe_mode.as_str());
    }
    if !inputs.overwrite_env {
        println!("(.env already present; will be preserved)");
    }
    println!();
}

fn print_exit_summary(inputs: &ResolvedInputs) {
    let portal_url = if inputs.portal_domain.starts_with("http") {
        inputs.portal_domain.clone()
    } else {
        format!("https://{}", inputs.portal_domain)
    };
    println!("\n============================================================");
    println!("Coterie installation complete.");
    println!();
    println!("  Org name:         {}", inputs.org_name);
    println!("  Portal URL:       {portal_url}");
    println!("  Admin email:      {}", inputs.admin_email);
    println!();
    println!("Next steps:");
    println!(
        "  1. Point DNS for {} at this box's public IP.",
        inputs.portal_domain
    );
    if inputs.enable_stripe {
        println!("  2. Register a Stripe webhook at {portal_url}/api/payments/webhook/stripe");
        println!("     See https://github.com/IndustriousKraken/coterie/blob/master/docs/deploy/STRIPE-SETUP.md for events to subscribe to.");
    }
    println!("  3. Log in at {portal_url}/login");
    println!("============================================================");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::CaptureOutput;
    use crate::test_support::{FakeFs, FakeSystem, MockPrompter};
    use std::path::Path;

    pub(super) fn make_args() -> InstallArgs {
        InstallArgs {
            org_name: Some("Acme Coterie".to_string()),
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
    fn dry_run_install_no_caddy_no_stripe() {
        let args = make_args();
        let sys = FakeSystem::new();
        let fs = FakeFs::new();
        let prompts = MockPrompter::new();
        let out = CaptureOutput::new();
        run(args, &sys, &fs, &prompts, &out).unwrap();
        // In dry-run mode, no apt-get calls are made (announce-only).
        assert_eq!(sys.calls.borrow().len(), 0);
    }

    #[test]
    fn detect_state_empty_box() {
        let fs = FakeFs::new();
        let s = detect_state(&fs);
        assert!(!s.env_present);
        assert!(!s.caddyfile_present);
        assert!(!s.caddyfile_managed_by_us);
    }

    #[test]
    fn detect_state_existing_env_and_managed_caddyfile() {
        let fs = FakeFs::new();
        fs.put(Path::new(ENV_PATH), b"COTERIE__SERVER__PORT=8080\n");
        fs.put(
            Path::new(CADDYFILE_PATH),
            format!(
                "{}\nportal.example.com {{ ... }}\n",
                caddyfile::COTERIE_MARKER
            )
            .as_bytes(),
        );
        let s = detect_state(&fs);
        assert!(s.env_present);
        assert!(s.caddyfile_present);
        assert!(s.caddyfile_managed_by_us);
    }

    #[test]
    fn detect_state_unmanaged_caddyfile() {
        let fs = FakeFs::new();
        fs.put(
            Path::new(CADDYFILE_PATH),
            b"# operator-edited\nportal.example.com { ... }\n",
        );
        let s = detect_state(&fs);
        assert!(s.caddyfile_present);
        assert!(!s.caddyfile_managed_by_us);
    }

    #[test]
    fn dry_run_install_with_all_integrations() {
        let mut args = make_args();
        args.enable_stripe = Some(true);
        // Default mode is live; supply live-or-test-prefixed keys.
        args.stripe_mode = Some(StripeMode::Live);
        args.stripe_publishable_key = Some("pk_test_abc".to_string());
        args.stripe_secret_key = Some(SecretString::new("sk_test_xyz".to_string()));
        args.stripe_webhook_secret = Some(SecretString::new("whsec_zzz".to_string()));
        args.enable_discord = Some(true);
        args.discord_bot_token = Some(SecretString::new("dtok".to_string()));
        args.discord_guild_id = Some("111".to_string());
        args.discord_member_role_id = Some("222".to_string());
        args.enable_unifi = Some(true);
        args.unifi_controller_url = Some("https://unifi.example.com:8443".to_string());
        args.unifi_username = Some("admin".to_string());
        args.unifi_password = Some(SecretString::new("pw".to_string()));
        args.unifi_site_id = Some("default".to_string());

        let sys = FakeSystem::new();
        let fs = FakeFs::new();
        let prompts = MockPrompter::new();
        let out = CaptureOutput::new();
        run(args, &sys, &fs, &prompts, &out).unwrap();
    }
}
