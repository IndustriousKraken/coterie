use anyhow::{anyhow, Context, Result};
use rand::RngCore;
use secrecy::{ExposeSecret, SecretString};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::caddyfile;
use crate::env_template::{self, EnvConfig};
use crate::fs_ops::FileSystem;
use crate::prompts::{resolve, Prompter};
use crate::stripe_check;
use crate::system::SystemCommand;

const INSTALL_DIR: &str = "/opt/coterie";
const ENV_PATH: &str = "/opt/coterie/.env";
const ENV_EXAMPLE_PATH: &str = "/opt/coterie/.env.example";
const CADDYFILE_PATH: &str = "/etc/caddy/Caddyfile";
const CADDYFILE_EXAMPLE_PATH: &str = "/opt/coterie/deploy/Caddyfile.example";
const CADDY_LOG_DIR: &str = "/var/log/caddy";
const RELEASE_DEPLOY_PATH: &str = "/usr/local/bin/coterie-release-deploy";

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
    pub stripe_publishable_key: Option<String>,
    pub stripe_secret_key: Option<SecretString>,
    pub stripe_webhook_secret: Option<SecretString>,

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
}

/// Idempotency findings detected at the start of the install.
#[derive(Debug, Clone, Default)]
pub struct PreflightState {
    pub env_present: bool,
    pub caddyfile_present: bool,
    pub caddyfile_managed_by_us: bool,
}

/// The orchestrator. Parametric over `SystemCommand` + `FileSystem` so
/// the test suite can drive it end-to-end with fakes.
pub fn run<S: SystemCommand, F: FileSystem, P: Prompter>(
    args: InstallArgs,
    sys: &S,
    fs: &F,
    prompts: &P,
) -> Result<()> {
    // --- Preflight ----------------------------------------------------
    if !args.dry_run {
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
    };

    exec.apt_update()?;
    exec.apt_install(inputs.enable_caddy)?;
    exec.fetch_release_deploy()?;
    exec.run_release_deploy(&inputs.version)?;
    exec.assert_binaries_present()?;
    exec.render_and_write_env(&inputs)?;
    exec.bootstrap_admin(&inputs)?;
    if inputs.enable_caddy {
        exec.write_caddyfile(&inputs)?;
    }
    exec.enable_and_start_service()?;
    exec.smoke_test()?;

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

/// The collected, resolved configuration ready for execution.
pub struct ResolvedInputs {
    pub org_name: String,
    pub portal_domain: String,
    pub marketing_domain: Option<String>,
    pub contact_email: String,
    pub admin_email: String,
    pub admin_username: String,
    pub admin_full_name: String,
    pub admin_password: SecretString,
    pub enable_stripe: bool,
    pub stripe_publishable_key: Option<String>,
    pub stripe_secret_key: Option<SecretString>,
    pub stripe_webhook_secret: Option<SecretString>,
    pub enable_discord: bool,
    pub discord_bot_token: Option<SecretString>,
    pub discord_guild_id: Option<String>,
    pub discord_member_role_id: Option<String>,
    pub discord_expired_role_id: Option<String>,
    pub enable_unifi: bool,
    pub unifi_controller_url: Option<String>,
    pub unifi_username: Option<String>,
    pub unifi_password: Option<SecretString>,
    pub unifi_site_id: Option<String>,
    pub enable_caddy: bool,
    pub version: String,
    pub overwrite_env: bool,
    pub session_secret: SecretString,
}

fn gather_inputs<P: Prompter>(
    args: &InstallArgs,
    prompts: &P,
    preflight: &PreflightState,
) -> Result<ResolvedInputs> {
    let no_prompt = args.no_prompt;

    let org_name = resolve(
        "org-name",
        "COTERIE_PROVISION_ORG_NAME",
        args.org_name.clone(),
        None,
        no_prompt,
        || prompts.prompt_text("Org name (used in emails, page titles)", None),
    )?;

    let portal_domain = resolve(
        "portal-domain",
        "COTERIE_PROVISION_PORTAL_DOMAIN",
        args.portal_domain.clone(),
        None,
        no_prompt,
        || prompts.prompt_text("Portal domain (e.g. coterie.example.com)", None),
    )?;

    let marketing_domain_raw = resolve(
        "marketing-domain",
        "COTERIE_PROVISION_MARKETING_DOMAIN",
        args.marketing_domain.clone(),
        Some(String::new()),
        no_prompt,
        || prompts.prompt_text("Marketing domain (optional, blank to skip)", Some("")),
    )?;
    let marketing_domain = if marketing_domain_raw.trim().is_empty() {
        None
    } else {
        Some(marketing_domain_raw)
    };

    let contact_email = resolve(
        "contact-email",
        "COTERIE_PROVISION_CONTACT_EMAIL",
        args.contact_email.clone(),
        None,
        no_prompt,
        || prompts.prompt_text("Org contact email (for AdminAlerts)", None),
    )?;

    let admin_email = resolve(
        "admin-email",
        "COTERIE_PROVISION_ADMIN_EMAIL",
        args.admin_email.clone(),
        None,
        no_prompt,
        || prompts.prompt_text("First admin email", None),
    )?;

    let admin_username = resolve(
        "admin-username",
        "COTERIE_PROVISION_ADMIN_USERNAME",
        args.admin_username.clone(),
        None,
        no_prompt,
        || prompts.prompt_text("First admin username", None),
    )?;

    let admin_full_name = resolve(
        "admin-full-name",
        "COTERIE_PROVISION_ADMIN_FULL_NAME",
        args.admin_full_name.clone(),
        None,
        no_prompt,
        || prompts.prompt_text("First admin full name", None),
    )?;

    let admin_password = resolve_secret(
        "admin-password",
        "COTERIE_PROVISION_ADMIN_PASSWORD",
        args.admin_password.clone(),
        no_prompt,
        || prompts.prompt_secret("First admin password (input hidden)"),
    )?;

    let enable_stripe = resolve_bool(
        "enable-stripe",
        "COTERIE_PROVISION_ENABLE_STRIPE",
        args.enable_stripe,
        Some(false),
        no_prompt,
        || prompts.prompt_yn("Enable Stripe integration?", false),
    )?;

    let (stripe_pk, stripe_sk, stripe_whsec) = if enable_stripe {
        let pk = resolve(
            "stripe-publishable-key",
            "COTERIE_PROVISION_STRIPE_PK",
            args.stripe_publishable_key.clone(),
            None,
            no_prompt,
            || prompts.prompt_text("Stripe publishable key (pk_…)", None),
        )?;
        // Allow either test or live; we just enforce the family.
        if !pk.starts_with("pk_test_") && !pk.starts_with("pk_live_") {
            return Err(anyhow!(
                "Stripe publishable key must start with pk_test_ or pk_live_"
            ));
        }
        let sk = resolve_secret(
            "stripe-secret-key",
            "COTERIE_PROVISION_STRIPE_SK",
            args.stripe_secret_key.clone(),
            no_prompt,
            || prompts.prompt_secret("Stripe secret key (sk_…) — input hidden"),
        )?;
        let sk_str = sk.expose_secret().clone();
        if !sk_str.starts_with("sk_test_") && !sk_str.starts_with("sk_live_") {
            return Err(anyhow!(
                "Stripe secret key must start with sk_test_ or sk_live_"
            ));
        }
        let whsec = resolve_secret(
            "stripe-webhook-secret",
            "COTERIE_PROVISION_STRIPE_WHSEC",
            args.stripe_webhook_secret.clone(),
            no_prompt,
            || prompts.prompt_secret("Stripe webhook signing secret (whsec_…)"),
        )?;
        stripe_check::validate_prefix(whsec.expose_secret(), "whsec_")?;
        (Some(pk), Some(sk), Some(whsec))
    } else {
        (None, None, None)
    };

    let enable_discord = resolve_bool(
        "enable-discord",
        "COTERIE_PROVISION_ENABLE_DISCORD",
        args.enable_discord,
        Some(false),
        no_prompt,
        || prompts.prompt_yn("Enable Discord integration?", false),
    )?;

    let (discord_token, discord_guild, discord_member_role, discord_expired_role) =
        if enable_discord {
            let t = resolve_secret(
                "discord-bot-token",
                "COTERIE_PROVISION_DISCORD_TOKEN",
                args.discord_bot_token.clone(),
                no_prompt,
                || prompts.prompt_secret("Discord bot token (input hidden)"),
            )?;
            let g = resolve(
                "discord-guild-id",
                "COTERIE_PROVISION_DISCORD_GUILD",
                args.discord_guild_id.clone(),
                None,
                no_prompt,
                || prompts.prompt_text("Discord guild (server) ID", None),
            )?;
            let mr = resolve(
                "discord-member-role-id",
                "COTERIE_PROVISION_DISCORD_MEMBER_ROLE",
                args.discord_member_role_id.clone(),
                None,
                no_prompt,
                || prompts.prompt_text("Discord member role ID", None),
            )?;
            let er = resolve(
                "discord-expired-role-id",
                "COTERIE_PROVISION_DISCORD_EXPIRED_ROLE",
                args.discord_expired_role_id.clone(),
                Some(String::new()),
                no_prompt,
                || prompts.prompt_text("Discord expired role ID (blank to skip)", Some("")),
            )?;
            let er = if er.trim().is_empty() { None } else { Some(er) };
            (Some(t), Some(g), Some(mr), er)
        } else {
            (None, None, None, None)
        };

    let enable_unifi = resolve_bool(
        "enable-unifi",
        "COTERIE_PROVISION_ENABLE_UNIFI",
        args.enable_unifi,
        Some(false),
        no_prompt,
        || prompts.prompt_yn("Enable UniFi integration?", false),
    )?;

    let (unifi_url, unifi_user, unifi_pw, unifi_site) = if enable_unifi {
        let url = resolve(
            "unifi-controller-url",
            "COTERIE_PROVISION_UNIFI_URL",
            args.unifi_controller_url.clone(),
            None,
            no_prompt,
            || prompts.prompt_text("UniFi controller URL", None),
        )?;
        let u = resolve(
            "unifi-username",
            "COTERIE_PROVISION_UNIFI_USERNAME",
            args.unifi_username.clone(),
            None,
            no_prompt,
            || prompts.prompt_text("UniFi username", None),
        )?;
        let pw = resolve_secret(
            "unifi-password",
            "COTERIE_PROVISION_UNIFI_PASSWORD",
            args.unifi_password.clone(),
            no_prompt,
            || prompts.prompt_secret("UniFi password (input hidden)"),
        )?;
        let s = resolve(
            "unifi-site-id",
            "COTERIE_PROVISION_UNIFI_SITE",
            args.unifi_site_id.clone(),
            Some("default".to_string()),
            no_prompt,
            || prompts.prompt_text("UniFi site ID", Some("default")),
        )?;
        (Some(url), Some(u), Some(pw), Some(s))
    } else {
        (None, None, None, None)
    };

    let enable_caddy = resolve_bool(
        "enable-caddy",
        "COTERIE_PROVISION_ENABLE_CADDY",
        args.enable_caddy,
        Some(true),
        no_prompt,
        || prompts.prompt_yn("Install and configure Caddy?", true),
    )?;

    // Version selection. The version selector module fetches the
    // release list separately in the production path; here we accept
    // a flag/env override or fall back to a static prompt that just
    // asks for a tag string. (The version-list UI lives in main.rs.)
    let version = resolve(
        "version",
        "COTERIE_PROVISION_VERSION",
        args.version.clone(),
        None,
        no_prompt,
        || prompts.prompt_text("Coterie version tag to install (e.g. v1.1.0)", None),
    )?;

    // Idempotency check: if .env already exists, ask before overwriting.
    let overwrite_env = if preflight.env_present {
        let from_env = std::env::var("COTERIE_PROVISION_OVERWRITE_ENV")
            .ok()
            .map(|s| matches!(s.as_str(), "true" | "1" | "yes"))
            .unwrap_or(false);
        if from_env || args.overwrite_env {
            true
        } else if no_prompt {
            return Err(anyhow!(
                "/opt/coterie/.env already exists; pass --overwrite-env or set COTERIE_PROVISION_OVERWRITE_ENV=true to clobber"
            ));
        } else {
            prompts.prompt_yn(
                "/opt/coterie/.env exists. Overwrite with new values?",
                false,
            )?
        }
    } else {
        true
    };

    // Generate a session secret (64 hex chars).
    let session_secret = generate_session_secret();

    Ok(ResolvedInputs {
        org_name,
        portal_domain,
        marketing_domain,
        contact_email,
        admin_email,
        admin_username,
        admin_full_name,
        admin_password,
        enable_stripe,
        stripe_publishable_key: stripe_pk,
        stripe_secret_key: stripe_sk,
        stripe_webhook_secret: stripe_whsec,
        enable_discord,
        discord_bot_token: discord_token,
        discord_guild_id: discord_guild,
        discord_member_role_id: discord_member_role,
        discord_expired_role_id: discord_expired_role,
        enable_unifi,
        unifi_controller_url: unifi_url,
        unifi_username: unifi_user,
        unifi_password: unifi_pw,
        unifi_site_id: unifi_site,
        enable_caddy,
        version,
        overwrite_env,
        session_secret,
    })
}

fn resolve_bool<Fp: FnOnce() -> Result<bool>>(
    name: &str,
    env_var: &str,
    cli_value: Option<bool>,
    default: Option<bool>,
    no_prompt: bool,
    prompt_fn: Fp,
) -> Result<bool> {
    if let Some(v) = cli_value {
        return Ok(v);
    }
    if let Ok(raw) = std::env::var(env_var) {
        if !raw.is_empty() {
            return match raw.to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "y" | "on" => Ok(true),
                "false" | "0" | "no" | "n" | "off" => Ok(false),
                other => Err(anyhow!("env var {env_var}={other} is not a boolean")),
            };
        }
    }
    if no_prompt {
        if let Some(d) = default {
            return Ok(d);
        }
        return Err(anyhow!(
            "missing required boolean input `{name}` — set {env_var} or pass --{name}"
        ));
    }
    prompt_fn()
}

fn resolve_secret<Fp: FnOnce() -> Result<SecretString>>(
    name: &str,
    env_var: &str,
    cli_value: Option<SecretString>,
    no_prompt: bool,
    prompt_fn: Fp,
) -> Result<SecretString> {
    if let Some(v) = cli_value {
        return Ok(v);
    }
    if let Ok(raw) = std::env::var(env_var) {
        if !raw.is_empty() {
            return Ok(SecretString::new(raw));
        }
    }
    if no_prompt {
        return Err(anyhow!(
            "missing required secret input `{name}` — set {env_var} or pass --{name}"
        ));
    }
    prompt_fn()
}

fn generate_session_secret() -> SecretString {
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    SecretString::new(hex::encode(buf))
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
        println!("     See /opt/coterie/deploy/STRIPE-SETUP.md for events to subscribe to.");
    }
    println!("  3. Log in at {portal_url}/login");
    println!("============================================================");
}

struct Executor<'a, S: SystemCommand, F: FileSystem> {
    sys: &'a S,
    fs: &'a F,
    dry_run: bool,
}

impl<'a, S: SystemCommand, F: FileSystem> Executor<'a, S, F> {
    fn announce(&self, what: &str) {
        let tag = if self.dry_run { "DRY-RUN" } else { "STEP" };
        println!("[{tag}] {what}");
    }

    fn run(&self, cmd: &str, args: &[&str], description: &str) -> Result<()> {
        self.announce(&format!("{description}: {cmd} {}", args.join(" ")));
        if self.dry_run {
            return Ok(());
        }
        let out = self.sys.run(cmd, args)?;
        if !out.success() {
            return Err(anyhow!(
                "{description} failed (exit {}): {}\n{}",
                out.status,
                out.stdout,
                out.stderr
            ));
        }
        Ok(())
    }

    fn run_allow_codes(
        &self,
        cmd: &str,
        args: &[&str],
        description: &str,
        allowed_codes: &[i32],
    ) -> Result<i32> {
        self.announce(&format!("{description}: {cmd} {}", args.join(" ")));
        if self.dry_run {
            return Ok(0);
        }
        let out = self.sys.run(cmd, args)?;
        if !allowed_codes.contains(&out.status) {
            return Err(anyhow!(
                "{description} failed (exit {}): {}\n{}",
                out.status,
                out.stdout,
                out.stderr
            ));
        }
        Ok(out.status)
    }

    fn apt_update(&self) -> Result<()> {
        self.run("apt-get", &["update"], "apt-get update")
    }

    fn apt_install(&self, with_caddy: bool) -> Result<()> {
        let mut args = vec![
            "install",
            "-y",
            "--no-install-recommends",
            "curl",
            "python3",
            "tar",
            "sqlite3",
            "ca-certificates",
            "openssl",
        ];
        if with_caddy {
            args.push("caddy");
        }
        self.run("apt-get", &args, "apt-get install")
    }

    fn fetch_release_deploy(&self) -> Result<()> {
        // Provided by the release tarball alongside the binary, but
        // we may also need it before the first release-deploy.sh run.
        // Use the install dir's copy if it exists.
        let from = Path::new("/opt/coterie/deploy/release-deploy.sh");
        let to = Path::new(RELEASE_DEPLOY_PATH);
        if self.fs.is_file(from) {
            self.announce(&format!("copying {} -> {}", from.display(), to.display()));
            if !self.dry_run {
                let body = self.fs.read_to_string(from)?;
                self.fs.write(to, body.as_bytes())?;
                self.fs.chmod(to, 0o755)?;
            }
        } else {
            self.announce(&format!(
                "release-deploy.sh not yet present at {} — release-deploy.sh will run from the extracted tarball.",
                from.display()
            ));
        }
        Ok(())
    }

    fn run_release_deploy(&self, tag: &str) -> Result<()> {
        // Use whichever exists: the just-staged /usr/local/bin path or
        // the in-place path.
        let to = Path::new(RELEASE_DEPLOY_PATH);
        let from = Path::new("/opt/coterie/deploy/release-deploy.sh");
        let cmd: PathBuf = if self.fs.is_file(to) {
            to.to_path_buf()
        } else {
            from.to_path_buf()
        };
        let cmd_str = cmd.to_string_lossy();
        self.run(
            "bash",
            &[cmd_str.as_ref(), tag],
            "release-deploy.sh (fetch + place binaries)",
        )
    }

    fn assert_binaries_present(&self) -> Result<()> {
        if self.dry_run {
            self.announce("would assert /opt/coterie/coterie + create_admin exist");
            return Ok(());
        }
        for bin in ["coterie", "create_admin"] {
            let p = PathBuf::from(format!("{INSTALL_DIR}/{bin}"));
            if !self.fs.is_file(&p) {
                return Err(anyhow!(
                    "expected `{}` after release-deploy.sh but it was not present",
                    p.display()
                ));
            }
        }
        Ok(())
    }

    fn render_and_write_env(&self, inputs: &ResolvedInputs) -> Result<()> {
        if !inputs.overwrite_env {
            self.announce("skipping .env render (existing file preserved)");
            return Ok(());
        }
        let template = if self.dry_run && !self.fs.is_file(Path::new(ENV_EXAMPLE_PATH)) {
            // For dry-run on a fresh box where release-deploy.sh
            // hasn't actually placed .env.example, fall back to the
            // embedded fixture so the rendered .env preview is still
            // accurate.
            include_str!("../tests/fixtures/env_example.txt").to_string()
        } else {
            self.fs.read_to_string(Path::new(ENV_EXAMPLE_PATH))?
        };

        let base_url = format!("https://{}", inputs.portal_domain);
        let mut env_config = EnvConfig::defaults_for(&base_url, inputs.session_secret.clone());
        if inputs.enable_stripe {
            env_config.stripe = Some(env_template::StripeConfig {
                publishable_key: inputs
                    .stripe_publishable_key
                    .clone()
                    .ok_or_else(|| anyhow!("stripe enabled but publishable key missing"))?,
                secret_key: inputs
                    .stripe_secret_key
                    .clone()
                    .ok_or_else(|| anyhow!("stripe enabled but secret key missing"))?,
                webhook_secret: inputs
                    .stripe_webhook_secret
                    .clone()
                    .ok_or_else(|| anyhow!("stripe enabled but webhook secret missing"))?,
            });
        }
        if inputs.enable_discord {
            env_config.discord = Some(env_template::DiscordConfig {
                bot_token: inputs
                    .discord_bot_token
                    .clone()
                    .ok_or_else(|| anyhow!("discord enabled but bot token missing"))?,
                guild_id: inputs
                    .discord_guild_id
                    .clone()
                    .ok_or_else(|| anyhow!("discord enabled but guild ID missing"))?,
                member_role_id: inputs
                    .discord_member_role_id
                    .clone()
                    .ok_or_else(|| anyhow!("discord enabled but member role ID missing"))?,
                expired_role_id: inputs.discord_expired_role_id.clone(),
            });
        }
        if inputs.enable_unifi {
            env_config.unifi = Some(env_template::UnifiConfig {
                controller_url: inputs
                    .unifi_controller_url
                    .clone()
                    .ok_or_else(|| anyhow!("unifi enabled but controller URL missing"))?,
                username: inputs
                    .unifi_username
                    .clone()
                    .ok_or_else(|| anyhow!("unifi enabled but username missing"))?,
                password: inputs
                    .unifi_password
                    .clone()
                    .ok_or_else(|| anyhow!("unifi enabled but password missing"))?,
                site_id: inputs
                    .unifi_site_id
                    .clone()
                    .unwrap_or_else(|| "default".to_string()),
            });
        }

        let rendered = env_template::render_env(&template, &env_config);

        self.announce(&format!("writing {}", ENV_PATH));
        if self.dry_run {
            println!("--- {ENV_PATH} preview (dry-run, secrets visible — review carefully) ---");
            for line in rendered.lines() {
                println!("    {line}");
            }
            return Ok(());
        }
        self.fs.write(Path::new(ENV_PATH), rendered.as_bytes())?;
        self.fs.chmod(Path::new(ENV_PATH), 0o640)?;
        self.fs
            .chown(Path::new(ENV_PATH), "coterie", "coterie")
            .ok();
        Ok(())
    }

    fn bootstrap_admin(&self, inputs: &ResolvedInputs) -> Result<()> {
        self.announce("bootstrapping first admin via create_admin");
        if self.dry_run {
            return Ok(());
        }

        // Write the password to a 0600 NamedTempFile, hand the path
        // to create_admin, then overwrite-with-zeros and unlink.
        let mut tmp = tempfile::Builder::new()
            .prefix("coterie-pw-")
            .tempfile()
            .context("failed to create password tempfile")?;
        {
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(tmp.path(), perms).context("chmod 0600 on tempfile")?;
        }
        tmp.write_all(inputs.admin_password.expose_secret().as_bytes())
            .context("writing password to tempfile")?;
        tmp.flush().ok();

        let tmp_path_owned = tmp.path().to_path_buf();
        let tmp_path = tmp_path_owned.to_string_lossy();

        let exit_code = self.run_allow_codes(
            "/opt/coterie/create_admin",
            &[
                "--password-file",
                tmp_path.as_ref(),
                "--email",
                inputs.admin_email.as_str(),
                "--username",
                inputs.admin_username.as_str(),
                "--full-name",
                inputs.admin_full_name.as_str(),
            ],
            "create_admin",
            &[0, 2],
        );

        // Always shred-then-drop, regardless of create_admin's exit code.
        let _ = overwrite_with_zeros(tmp.path());
        drop(tmp);

        match exit_code {
            Ok(0) => {
                self.announce("first admin created");
                Ok(())
            }
            Ok(2) => {
                self.announce("admin already exists — skipping create_admin (idempotent)");
                Ok(())
            }
            Ok(other) => Err(anyhow!(
                "create_admin returned unexpected exit code {other}"
            )),
            Err(e) => Err(e),
        }
    }

    fn write_caddyfile(&self, inputs: &ResolvedInputs) -> Result<()> {
        let template = if self.dry_run && !self.fs.is_file(Path::new(CADDYFILE_EXAMPLE_PATH)) {
            include_str!("../tests/fixtures/caddyfile_example.txt").to_string()
        } else {
            self.fs.read_to_string(Path::new(CADDYFILE_EXAMPLE_PATH))?
        };

        let rendered = caddyfile::render_caddyfile(
            &template,
            &inputs.portal_domain,
            inputs.marketing_domain.as_deref(),
        );

        self.announce("creating /var/log/caddy + chown caddy:caddy (log-dir fix)");
        if !self.dry_run {
            self.fs.create_dir_all(Path::new(CADDY_LOG_DIR))?;
            self.fs
                .chown(Path::new(CADDY_LOG_DIR), "caddy", "caddy")
                .ok();
        }

        self.announce(&format!("writing {}", CADDYFILE_PATH));
        if self.dry_run {
            println!("--- {CADDYFILE_PATH} preview (dry-run) ---");
            for line in rendered.lines() {
                println!("    {line}");
            }
        } else {
            self.fs
                .write(Path::new(CADDYFILE_PATH), rendered.as_bytes())?;
        }

        self.run(
            "caddy",
            &["validate", "--config", CADDYFILE_PATH],
            "caddy validate",
        )?;

        self.run("systemctl", &["reload", "caddy"], "systemctl reload caddy")
    }

    fn enable_and_start_service(&self) -> Result<()> {
        self.run(
            "systemctl",
            &["enable", "--now", "coterie"],
            "systemctl enable --now coterie",
        )
    }

    fn smoke_test(&self) -> Result<()> {
        if self.dry_run {
            self.announce("would GET http://127.0.0.1:8080/health");
            return Ok(());
        }
        // Use curl for the smoke test so it routes through the
        // SystemCommand trait and is mockable in tests. -fsSL gives us
        // a non-zero exit on HTTP error.
        self.run(
            "curl",
            &["-fsSL", "http://127.0.0.1:8080/health"],
            "smoke test GET /health",
        )
    }
}

fn overwrite_with_zeros(path: &Path) -> Result<()> {
    if let Ok(meta) = std::fs::metadata(path) {
        let len = meta.len() as usize;
        let zeros = vec![0u8; len.max(64)];
        std::fs::write(path, zeros)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{FakeFs, FakeSystem, MockPrompter};
    use std::path::Path;

    fn make_args() -> InstallArgs {
        InstallArgs {
            org_name: Some("Acme Coterie".to_string()),
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
    fn dry_run_install_no_caddy_no_stripe() {
        let args = make_args();
        let sys = FakeSystem::new();
        let fs = FakeFs::new();
        let prompts = MockPrompter::new();
        run(args, &sys, &fs, &prompts).unwrap();
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
        run(args, &sys, &fs, &prompts).unwrap();
    }

    #[test]
    fn missing_required_input_fails_under_no_prompt() {
        let mut args = make_args();
        args.org_name = None;
        std::env::remove_var("COTERIE_PROVISION_ORG_NAME");

        let sys = FakeSystem::new();
        let fs = FakeFs::new();
        let prompts = MockPrompter::new();
        let err = run(args, &sys, &fs, &prompts).unwrap_err();
        assert!(err.to_string().contains("org-name") || err.to_string().contains("ORG_NAME"));
    }

    #[test]
    fn stripe_bad_prefix_rejected() {
        let mut args = make_args();
        args.enable_stripe = Some(true);
        args.stripe_publishable_key = Some("pk_invalid_abc".to_string());
        args.stripe_secret_key = Some(SecretString::new("sk_test_xyz".to_string()));
        args.stripe_webhook_secret = Some(SecretString::new("whsec_zzz".to_string()));

        let sys = FakeSystem::new();
        let fs = FakeFs::new();
        let prompts = MockPrompter::new();
        let err = run(args, &sys, &fs, &prompts).unwrap_err();
        assert!(err.to_string().contains("pk_test_") || err.to_string().contains("pk_live_"));
    }
}
