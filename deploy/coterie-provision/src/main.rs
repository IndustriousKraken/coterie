use anyhow::Result;
use clap::{Parser, Subcommand};
use coterie_provision::fs_ops::RealFs;
use coterie_provision::install::{self, InstallArgs, StripeMode};
use coterie_provision::output::RealOutput;
use coterie_provision::prompts::InquirePrompter;
use coterie_provision::stripe_api::RealStripeApi;
use coterie_provision::switch_to_live::{self, RealRootCheck, SwitchArgs};
use coterie_provision::system::RealSystem;
use secrecy::SecretString;

#[derive(Debug, Parser)]
#[command(
    name = "coterie-provision",
    version,
    about = "Provision a fresh Debian host with Coterie + Caddy + first admin."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the full install wizard (the primary entry point).
    Install(Box<InstallCli>),
    /// (placeholder) Switch a live Stripe deploy from test keys.
    /// Implemented by openspec change `a25`.
    SwitchStripeToLive(Box<SwitchStripeCli>),
}

#[derive(Debug, Parser, Default)]
struct InstallCli {
    /// Human-readable org name (page titles, emails).
    #[arg(long, env = "COTERIE_PROVISION_ORG_NAME")]
    org_name: Option<String>,

    /// Portal hostname (e.g. coterie.example.com).
    #[arg(long, env = "COTERIE_PROVISION_PORTAL_DOMAIN")]
    portal_domain: Option<String>,

    /// Optional marketing hostname (second Caddy vhost).
    #[arg(long, env = "COTERIE_PROVISION_MARKETING_DOMAIN")]
    marketing_domain: Option<String>,

    /// Org contact email used for AdminAlert delivery.
    #[arg(long, env = "COTERIE_PROVISION_CONTACT_EMAIL")]
    contact_email: Option<String>,

    /// First admin email.
    #[arg(long, env = "COTERIE_PROVISION_ADMIN_EMAIL")]
    admin_email: Option<String>,

    /// First admin username.
    #[arg(long, env = "COTERIE_PROVISION_ADMIN_USERNAME")]
    admin_username: Option<String>,

    /// First admin full name.
    #[arg(long, env = "COTERIE_PROVISION_ADMIN_FULL_NAME")]
    admin_full_name: Option<String>,

    /// First admin password. Env var preferred — passing on the CLI
    /// makes it visible in process listings.
    #[arg(long, env = "COTERIE_PROVISION_ADMIN_PASSWORD", hide_env_values = true)]
    admin_password: Option<String>,

    #[arg(long, env = "COTERIE_PROVISION_ENABLE_STRIPE")]
    enable_stripe: Option<bool>,

    /// Stripe mode: `test` or `live`. Default: `live`.
    #[arg(long, env = "COTERIE_PROVISION_STRIPE_MODE")]
    stripe_mode: Option<String>,

    #[arg(long, env = "COTERIE_PROVISION_STRIPE_PK")]
    stripe_publishable_key: Option<String>,

    #[arg(long, env = "COTERIE_PROVISION_STRIPE_SK", hide_env_values = true)]
    stripe_secret_key: Option<String>,

    #[arg(long, env = "COTERIE_PROVISION_STRIPE_WHSEC", hide_env_values = true)]
    stripe_webhook_secret: Option<String>,

    /// In test mode: also stage live creds in /opt/coterie/.env.live.
    /// Inferred true when all three --live-* values are supplied.
    #[arg(long, env = "COTERIE_PROVISION_PRELOAD_LIVE_CREDS")]
    preload_live_creds: Option<bool>,

    #[arg(long, env = "COTERIE_PROVISION_STRIPE_LIVE_PK")]
    stripe_live_publishable_key: Option<String>,

    #[arg(long, env = "COTERIE_PROVISION_STRIPE_LIVE_SK", hide_env_values = true)]
    stripe_live_secret_key: Option<String>,

    #[arg(
        long,
        env = "COTERIE_PROVISION_STRIPE_LIVE_WHSEC",
        hide_env_values = true
    )]
    stripe_live_webhook_secret: Option<String>,

    #[arg(long, env = "COTERIE_PROVISION_ENABLE_DISCORD")]
    enable_discord: Option<bool>,

    #[arg(long, env = "COTERIE_PROVISION_DISCORD_TOKEN", hide_env_values = true)]
    discord_bot_token: Option<String>,

    #[arg(long, env = "COTERIE_PROVISION_DISCORD_GUILD")]
    discord_guild_id: Option<String>,

    #[arg(long, env = "COTERIE_PROVISION_DISCORD_MEMBER_ROLE")]
    discord_member_role_id: Option<String>,

    #[arg(long, env = "COTERIE_PROVISION_DISCORD_EXPIRED_ROLE")]
    discord_expired_role_id: Option<String>,

    #[arg(long, env = "COTERIE_PROVISION_ENABLE_UNIFI")]
    enable_unifi: Option<bool>,

    #[arg(long, env = "COTERIE_PROVISION_UNIFI_URL")]
    unifi_controller_url: Option<String>,

    #[arg(long, env = "COTERIE_PROVISION_UNIFI_USERNAME")]
    unifi_username: Option<String>,

    #[arg(long, env = "COTERIE_PROVISION_UNIFI_PASSWORD", hide_env_values = true)]
    unifi_password: Option<String>,

    #[arg(long, env = "COTERIE_PROVISION_UNIFI_SITE")]
    unifi_site_id: Option<String>,

    /// Install and configure Caddy (recommended).
    #[arg(long, env = "COTERIE_PROVISION_ENABLE_CADDY")]
    enable_caddy: Option<bool>,

    /// Coterie release tag (e.g. v1.1.0). If omitted, defaults to the
    /// latest stable release via the GitHub API.
    #[arg(long, env = "COTERIE_PROVISION_VERSION")]
    version: Option<String>,

    /// Disable interactive prompting — required inputs must be set via
    /// flags or env vars.
    #[arg(long)]
    no_prompt: bool,

    /// Print the planned actions without executing any of them.
    #[arg(long)]
    dry_run: bool,

    /// Overwrite an existing /opt/coterie/.env without prompting.
    #[arg(long)]
    overwrite_env: bool,
}

#[derive(Debug, Parser, Default)]
struct SwitchStripeCli {
    /// Delete coterie-test.db rather than archiving it.
    #[arg(long)]
    discard_test_db: bool,

    /// Skip the confirmation prompt and proceed.
    #[arg(long)]
    yes: bool,

    /// Disable interactive prompting — credentials must come from
    /// /opt/coterie/.env.live, env vars, or flags.
    #[arg(long)]
    no_prompt: bool,

    /// Live publishable key (pk_live_…).
    #[arg(long, env = "COTERIE_PROVISION_STRIPE_LIVE_PK")]
    live_pk: Option<String>,

    /// Live secret key (sk_live_…). Env-only is preferred.
    #[arg(long, env = "COTERIE_PROVISION_STRIPE_LIVE_SK", hide_env_values = true)]
    live_sk: Option<String>,

    /// Live webhook signing secret (whsec_…).
    #[arg(
        long,
        env = "COTERIE_PROVISION_STRIPE_LIVE_WHSEC",
        hide_env_values = true
    )]
    live_whsec: Option<String>,
}

impl From<SwitchStripeCli> for SwitchArgs {
    fn from(c: SwitchStripeCli) -> Self {
        Self {
            discard_test_db: c.discard_test_db,
            yes: c.yes,
            no_prompt: c.no_prompt,
            live_pk: c.live_pk,
            live_sk: c.live_sk.map(SecretString::new),
            live_whsec: c.live_whsec.map(SecretString::new),
        }
    }
}

impl TryFrom<InstallCli> for InstallArgs {
    type Error = anyhow::Error;
    fn try_from(c: InstallCli) -> Result<Self> {
        let stripe_mode = match c.stripe_mode.as_deref() {
            Some(s) => Some(s.parse::<StripeMode>()?),
            None => None,
        };
        Ok(Self {
            org_name: c.org_name,
            portal_domain: c.portal_domain,
            marketing_domain: c.marketing_domain,
            contact_email: c.contact_email,
            admin_email: c.admin_email,
            admin_username: c.admin_username,
            admin_full_name: c.admin_full_name,
            admin_password: c.admin_password.map(SecretString::new),
            enable_stripe: c.enable_stripe,
            stripe_mode,
            stripe_publishable_key: c.stripe_publishable_key,
            stripe_secret_key: c.stripe_secret_key.map(SecretString::new),
            stripe_webhook_secret: c.stripe_webhook_secret.map(SecretString::new),
            preload_live_creds: c.preload_live_creds,
            stripe_live_publishable_key: c.stripe_live_publishable_key,
            stripe_live_secret_key: c.stripe_live_secret_key.map(SecretString::new),
            stripe_live_webhook_secret: c.stripe_live_webhook_secret.map(SecretString::new),
            enable_discord: c.enable_discord,
            discord_bot_token: c.discord_bot_token.map(SecretString::new),
            discord_guild_id: c.discord_guild_id,
            discord_member_role_id: c.discord_member_role_id,
            discord_expired_role_id: c.discord_expired_role_id,
            enable_unifi: c.enable_unifi,
            unifi_controller_url: c.unifi_controller_url,
            unifi_username: c.unifi_username,
            unifi_password: c.unifi_password.map(SecretString::new),
            unifi_site_id: c.unifi_site_id,
            enable_caddy: c.enable_caddy,
            version: c.version,
            no_prompt: c.no_prompt,
            dry_run: c.dry_run,
            overwrite_env: c.overwrite_env,
            skip_root_check: false,
        })
    }
}

fn main() {
    if let Err(e) = real_main() {
        eprintln!("\nError: {e}");
        let mut src = e.source();
        let mut depth = 0;
        while let Some(s) = src {
            eprintln!("  {depth}: {s}");
            src = s.source();
            depth += 1;
        }
        std::process::exit(1);
    }
}

fn real_main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Install(args) => {
            let install_args: InstallArgs = (*args).try_into()?;
            let sys = RealSystem::new();
            let fs = RealFs::new();
            let prompts = InquirePrompter::new();
            let output = RealOutput::new();
            install::run(install_args, &sys, &fs, &prompts, &output)
        }
        Command::SwitchStripeToLive(args) => {
            let switch_args: SwitchArgs = (*args).into();
            let sys = RealSystem::new();
            let fs = RealFs::new();
            let api = RealStripeApi::new()?;
            let prompts = InquirePrompter::new();
            let output = RealOutput::new();
            let root = RealRootCheck;
            let exit_code =
                switch_to_live::run(switch_args, &sys, &fs, &api, &prompts, &output, &root)?;
            if exit_code != 0 {
                std::process::exit(exit_code);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_help_renders() {
        Cli::command().debug_assert();
    }

    #[test]
    fn install_parses_no_args() {
        let parsed = Cli::try_parse_from(["coterie-provision", "install"]);
        assert!(parsed.is_ok());
    }

    #[test]
    fn install_parses_all_flags() {
        // Don't let the test runner's env vars (with hyphen-prefixed
        // arg interpretation) confuse clap.
        let argv = [
            "coterie-provision",
            "install",
            "--org-name",
            "Acme",
            "--portal-domain",
            "portal.acme.io",
            "--marketing-domain",
            "acme.io",
            "--contact-email",
            "ops@acme.io",
            "--admin-email",
            "rab@acme.io",
            "--admin-username",
            "rab",
            "--admin-full-name",
            "R A Bee",
            "--enable-stripe",
            "true",
            "--enable-caddy",
            "true",
            "--enable-discord",
            "false",
            "--enable-unifi",
            "false",
            "--version",
            "v1.1.0",
            "--no-prompt",
            "--dry-run",
        ];
        let parsed = Cli::try_parse_from(argv).expect("parse");
        match parsed.command {
            Command::Install(c) => {
                assert_eq!(c.org_name.as_deref(), Some("Acme"));
                assert_eq!(c.portal_domain.as_deref(), Some("portal.acme.io"));
                assert_eq!(c.enable_stripe, Some(true));
                assert!(c.no_prompt);
                assert!(c.dry_run);
            }
            Command::SwitchStripeToLive(_) => panic!("expected install"),
        }
    }

    #[test]
    fn switch_stripe_parses() {
        let parsed = Cli::try_parse_from(["coterie-provision", "switch-stripe-to-live"]);
        assert!(parsed.is_ok());
    }
}
