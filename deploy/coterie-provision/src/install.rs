use crate::caddyfile::{render_caddyfile, COTERIE_MARKER};
use crate::env_template::{generate_session_secret, render_env, EnvConfig};
use crate::fs_ops::FileSystem;
use crate::prompts::{resolve, resolve_secret, Prompter};
use crate::stripe_check::validate_prefix;
use crate::system::{CommandOutput, SystemCommand};
use anyhow::{anyhow, Context, Result};
use clap::Args;
use secrecy::{ExposeSecret, Secret};
use std::path::Path;

pub const INSTALL_DIR: &str = "/opt/coterie";
pub const CADDYFILE_PATH: &str = "/etc/caddy/Caddyfile";
pub const ENV_PATH: &str = "/opt/coterie/.env";
pub const ENV_EXAMPLE_PATH: &str = "/opt/coterie/.env.example";
pub const CADDYFILE_EXAMPLE_PATH: &str = "/opt/coterie/deploy/Caddyfile.example";
pub const COTERIE_BIN: &str = "/opt/coterie/coterie";
pub const CREATE_ADMIN_BIN: &str = "/opt/coterie/create_admin";
pub const RELEASE_DEPLOY_BIN: &str = "/usr/local/bin/coterie-release-deploy";
pub const RELEASE_DEPLOY_URL: &str =
    "https://raw.githubusercontent.com/IndustriousKraken/coterie/master/deploy/release-deploy.sh";
pub const VAR_LOG_CADDY: &str = "/var/log/caddy";
pub const HEALTH_URL: &str = "http://127.0.0.1:8080/health";

#[derive(Debug, Clone, Args, Default)]
pub struct InstallArgs {
    /// Org name shown in emails, page titles, receipts.
    #[arg(long, env = "COTERIE_PROVISION_ORG_NAME")]
    pub org_name: Option<String>,

    /// Portal domain (e.g. coterie.example.org).
    #[arg(long, env = "COTERIE_PROVISION_PORTAL_DOMAIN")]
    pub portal_domain: Option<String>,

    /// Marketing domain (optional). If absent, only the portal site is configured.
    #[arg(long, env = "COTERIE_PROVISION_MARKETING_DOMAIN")]
    pub marketing_domain: Option<String>,

    /// Org-wide contact email used for AdminAlert delivery.
    #[arg(long, env = "COTERIE_PROVISION_CONTACT_EMAIL")]
    pub contact_email: Option<String>,

    #[arg(long, env = "COTERIE_PROVISION_ADMIN_EMAIL")]
    pub admin_email: Option<String>,
    #[arg(long, env = "COTERIE_PROVISION_ADMIN_USERNAME")]
    pub admin_username: Option<String>,
    #[arg(long, env = "COTERIE_PROVISION_ADMIN_FULL_NAME")]
    pub admin_full_name: Option<String>,
    /// Admin password. Prefer the env var to keep it off argv.
    #[arg(long, env = "COTERIE_PROVISION_ADMIN_PASSWORD")]
    pub admin_password: Option<String>,

    #[arg(long, env = "COTERIE_PROVISION_ENABLE_STRIPE")]
    pub enable_stripe: Option<bool>,
    #[arg(long, env = "COTERIE_PROVISION_STRIPE_PK")]
    pub stripe_publishable_key: Option<String>,
    #[arg(long, env = "COTERIE_PROVISION_STRIPE_SK")]
    pub stripe_secret_key: Option<String>,
    #[arg(long, env = "COTERIE_PROVISION_STRIPE_WHSEC")]
    pub stripe_webhook_secret: Option<String>,

    #[arg(long, env = "COTERIE_PROVISION_ENABLE_CADDY")]
    pub enable_caddy: Option<bool>,

    #[arg(long, env = "COTERIE_PROVISION_ENABLE_DISCORD")]
    pub enable_discord: Option<bool>,
    #[arg(long, env = "COTERIE_PROVISION_DISCORD_BOT_TOKEN")]
    pub discord_bot_token: Option<String>,
    #[arg(long, env = "COTERIE_PROVISION_DISCORD_GUILD_ID")]
    pub discord_guild_id: Option<String>,
    #[arg(long, env = "COTERIE_PROVISION_DISCORD_ANNOUNCE_CHANNEL")]
    pub discord_announce_channel: Option<String>,

    #[arg(long, env = "COTERIE_PROVISION_ENABLE_UNIFI")]
    pub enable_unifi: Option<bool>,
    #[arg(long, env = "COTERIE_PROVISION_UNIFI_URL")]
    pub unifi_url: Option<String>,
    #[arg(long, env = "COTERIE_PROVISION_UNIFI_USERNAME")]
    pub unifi_username: Option<String>,
    #[arg(long, env = "COTERIE_PROVISION_UNIFI_PASSWORD")]
    pub unifi_password: Option<String>,

    /// Coterie version to install (tag, e.g. v1.0.0). Default: latest stable.
    #[arg(long, env = "COTERIE_PROVISION_VERSION")]
    pub version: Option<String>,

    /// Skip every interactive prompt; require inputs via env/flags or defaults.
    #[arg(long, default_value_t = false)]
    pub no_prompt: bool,

    /// Plan-only: print the steps but do not execute side effects.
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,

    /// Overwrite an existing /opt/coterie/.env without prompting.
    #[arg(long, env = "COTERIE_PROVISION_OVERWRITE_ENV", default_value_t = false)]
    pub overwrite_env: bool,

    /// Override the GitHub-API release lookup with a literal releases JSON (testing).
    #[arg(long, hide = true)]
    pub release_json_override: Option<String>,
}

/// Top-level resolved inputs. Computed up front so the install loop
/// can write golden-stable .env / Caddyfile content.
#[derive(Debug, Clone)]
pub struct ResolvedInputs {
    pub org_name: String,
    pub portal_domain: String,
    pub marketing_domain: Option<String>,
    pub contact_email: String,

    pub admin_email: String,
    pub admin_username: String,
    pub admin_full_name: String,
    pub admin_password: Secret<String>,

    pub enable_caddy: bool,
    pub enable_stripe: bool,
    pub stripe_pk: Option<String>,
    pub stripe_sk: Option<Secret<String>>,
    pub stripe_whsec: Option<Secret<String>>,

    pub enable_discord: bool,
    pub discord_bot_token: Option<Secret<String>>,
    pub discord_guild_id: Option<String>,
    pub discord_announce_channel: Option<String>,

    pub enable_unifi: bool,
    pub unifi_url: Option<String>,
    pub unifi_username: Option<String>,
    pub unifi_password: Option<Secret<String>>,

    pub version: String,
}

pub fn run<S, F, P>(args: InstallArgs, sys: &S, fs: &F, prompts: &P) -> Result<()>
where
    S: SystemCommand,
    F: FileSystem,
    P: Prompter,
{
    let dry_run = args.dry_run;
    say(dry_run, "coterie-provision install");
    say(dry_run, "============================");
    if dry_run {
        println!("(dry-run — no side effects will occur)");
    }

    preflight(sys, fs, &args)?;

    let inputs = gather_inputs(&args, prompts)?;
    print_summary(&inputs);
    if !args.no_prompt && !dry_run {
        let proceed = prompts.yes_no("Proceed with these settings?", true)?;
        if !proceed {
            anyhow::bail!("operator declined to proceed");
        }
    }

    step_apt(sys, &inputs, dry_run)?;
    step_release_deploy(sys, fs, &inputs, dry_run)?;
    step_write_env(fs, prompts, &inputs, &args, dry_run)?;
    let admin_skipped = step_create_admin(sys, &inputs, dry_run)?;
    if inputs.enable_caddy {
        step_caddy(sys, fs, prompts, &inputs, &args, dry_run)?;
    } else {
        say(
            dry_run,
            "skipping caddy step (operator chose not to install)",
        );
    }
    step_service(sys, dry_run)?;
    step_smoke_test(sys, dry_run)?;
    print_summary_final(&inputs, admin_skipped);
    Ok(())
}

fn say(_dry_run: bool, msg: &str) {
    println!("{msg}");
}

fn announce(dry_run: bool, msg: &str) {
    let tag = if dry_run { "[plan] " } else { "[step] " };
    println!("{tag}{msg}");
}

fn run_step<S: SystemCommand>(
    sys: &S,
    dry_run: bool,
    cmd: &str,
    args: &[&str],
) -> Result<CommandOutput> {
    announce(dry_run, &format!("$ {cmd} {}", args.join(" ")));
    if dry_run {
        return Ok(CommandOutput::ok(""));
    }
    let out = sys.run_interactive(cmd, args)?;
    if !out.succeeded() {
        anyhow::bail!(
            "command {cmd} {} failed: status={} stderr={}",
            args.join(" "),
            out.status,
            out.stderr
        );
    }
    Ok(out)
}

fn preflight<S: SystemCommand, F: FileSystem>(sys: &S, fs: &F, args: &InstallArgs) -> Result<()> {
    if !args.dry_run {
        let uid_out = sys.run("id", &["-u"])?;
        if uid_out.succeeded() {
            let uid = uid_out.stdout.trim();
            if uid != "0" {
                anyhow::bail!("coterie-provision install must run as root (uid 0); got uid {uid}");
            }
        }
        if fs.exists(Path::new("/etc/os-release")) {
            let os = fs.read_to_string(Path::new("/etc/os-release"))?;
            if !os.contains("ID=debian") && !os.contains("ID=ubuntu") {
                println!(
                    "warning: /etc/os-release does not identify Debian or Ubuntu; \
                     wizard will proceed but assumes apt + systemd"
                );
            }
        }
        if !fs.is_dir(Path::new("/var/lib/coterie")) {
            println!(
                "warning: /var/lib/coterie does not exist yet. install.sh will create it, \
                 but if you intended to mount a separate volume there, mount it BEFORE \
                 continuing."
            );
        }
    }
    Ok(())
}

fn gather_inputs<P: Prompter>(args: &InstallArgs, prompts: &P) -> Result<ResolvedInputs> {
    let org_name: String = resolve(
        "COTERIE_PROVISION_ORG_NAME",
        args.org_name.clone(),
        None,
        args.no_prompt,
        || prompts.text("Org name", None),
    )?;
    let portal_domain: String = resolve(
        "COTERIE_PROVISION_PORTAL_DOMAIN",
        args.portal_domain.clone(),
        None,
        args.no_prompt,
        || prompts.text("Portal domain (e.g. coterie.example.org)", None),
    )?;
    let marketing_domain: Option<String> = if args.marketing_domain.is_some() {
        args.marketing_domain.clone()
    } else if let Ok(env) = std::env::var("COTERIE_PROVISION_MARKETING_DOMAIN") {
        if env.is_empty() {
            None
        } else {
            Some(env)
        }
    } else if args.no_prompt {
        None
    } else if prompts.yes_no(
        "Configure a marketing domain too? (e.g. example.org)",
        false,
    )? {
        Some(prompts.text("Marketing domain", None)?)
    } else {
        None
    };
    let contact_email: String = resolve(
        "COTERIE_PROVISION_CONTACT_EMAIL",
        args.contact_email.clone(),
        None,
        args.no_prompt,
        || prompts.text("Org contact email", None),
    )?;
    let admin_email: String = resolve(
        "COTERIE_PROVISION_ADMIN_EMAIL",
        args.admin_email.clone(),
        None,
        args.no_prompt,
        || prompts.text("First admin email", None),
    )?;
    let admin_username: String = resolve(
        "COTERIE_PROVISION_ADMIN_USERNAME",
        args.admin_username.clone(),
        None,
        args.no_prompt,
        || prompts.text("First admin username", None),
    )?;
    let admin_full_name: String = resolve(
        "COTERIE_PROVISION_ADMIN_FULL_NAME",
        args.admin_full_name.clone(),
        None,
        args.no_prompt,
        || prompts.text("First admin full name", None),
    )?;
    let admin_password: Secret<String> = resolve_secret(
        "COTERIE_PROVISION_ADMIN_PASSWORD",
        args.admin_password.clone(),
        args.no_prompt,
        || prompts.secret("First admin password", true),
    )?;

    let enable_caddy: bool = resolve(
        "COTERIE_PROVISION_ENABLE_CADDY",
        args.enable_caddy,
        Some(true),
        args.no_prompt,
        || prompts.yes_no("Install + configure Caddy?", true),
    )?;
    let enable_stripe: bool = resolve(
        "COTERIE_PROVISION_ENABLE_STRIPE",
        args.enable_stripe,
        Some(false),
        args.no_prompt,
        || prompts.yes_no("Enable Stripe?", false),
    )?;

    let (stripe_pk, stripe_sk, stripe_whsec) = if enable_stripe {
        let pk = resolve(
            "COTERIE_PROVISION_STRIPE_PK",
            args.stripe_publishable_key.clone(),
            None,
            args.no_prompt,
            || prompts.text("Stripe publishable key (pk_…)", None),
        )?;
        let sk_secret = resolve_secret(
            "COTERIE_PROVISION_STRIPE_SK",
            args.stripe_secret_key.clone(),
            args.no_prompt,
            || prompts.secret("Stripe secret key (sk_…)", false),
        )?;
        let whsec = resolve_secret(
            "COTERIE_PROVISION_STRIPE_WHSEC",
            args.stripe_webhook_secret.clone(),
            args.no_prompt,
            || prompts.secret("Stripe webhook secret (whsec_…)", false),
        )?;
        if pk.starts_with("pk_test_") {
            validate_prefix(&pk, "pk_test_")?;
        } else {
            validate_prefix(&pk, "pk_live_")?;
        }
        if sk_secret.expose_secret().starts_with("sk_test_") {
            validate_prefix(sk_secret.expose_secret(), "sk_test_")?;
        } else {
            validate_prefix(sk_secret.expose_secret(), "sk_live_")?;
        }
        validate_prefix(whsec.expose_secret(), "whsec_")?;
        (Some(pk), Some(sk_secret), Some(whsec))
    } else {
        (None, None, None)
    };

    let enable_discord: bool = resolve(
        "COTERIE_PROVISION_ENABLE_DISCORD",
        args.enable_discord,
        Some(false),
        args.no_prompt,
        || prompts.yes_no("Enable Discord integration?", false),
    )?;
    let (discord_bot_token, discord_guild_id, discord_announce_channel) = if enable_discord {
        let bot = resolve_secret(
            "COTERIE_PROVISION_DISCORD_BOT_TOKEN",
            args.discord_bot_token.clone(),
            args.no_prompt,
            || prompts.secret("Discord bot token", false),
        )?;
        let guild = resolve(
            "COTERIE_PROVISION_DISCORD_GUILD_ID",
            args.discord_guild_id.clone(),
            None,
            args.no_prompt,
            || prompts.text("Discord guild ID", None),
        )?;
        let ch = resolve(
            "COTERIE_PROVISION_DISCORD_ANNOUNCE_CHANNEL",
            args.discord_announce_channel.clone(),
            None,
            args.no_prompt,
            || prompts.text("Discord announcements channel ID", None),
        )?;
        (Some(bot), Some(guild), Some(ch))
    } else {
        (None, None, None)
    };

    let enable_unifi: bool = resolve(
        "COTERIE_PROVISION_ENABLE_UNIFI",
        args.enable_unifi,
        Some(false),
        args.no_prompt,
        || prompts.yes_no("Enable UniFi integration?", false),
    )?;
    let (unifi_url, unifi_username, unifi_password) = if enable_unifi {
        let url = resolve(
            "COTERIE_PROVISION_UNIFI_URL",
            args.unifi_url.clone(),
            None,
            args.no_prompt,
            || prompts.text("UniFi controller URL", None),
        )?;
        let user = resolve(
            "COTERIE_PROVISION_UNIFI_USERNAME",
            args.unifi_username.clone(),
            None,
            args.no_prompt,
            || prompts.text("UniFi username", None),
        )?;
        let pw = resolve_secret(
            "COTERIE_PROVISION_UNIFI_PASSWORD",
            args.unifi_password.clone(),
            args.no_prompt,
            || prompts.secret("UniFi password", false),
        )?;
        (Some(url), Some(user), Some(pw))
    } else {
        (None, None, None)
    };

    let version = pick_version(args, prompts)?;

    Ok(ResolvedInputs {
        org_name,
        portal_domain,
        marketing_domain,
        contact_email,
        admin_email,
        admin_username,
        admin_full_name,
        admin_password,
        enable_caddy,
        enable_stripe,
        stripe_pk,
        stripe_sk,
        stripe_whsec,
        enable_discord,
        discord_bot_token,
        discord_guild_id,
        discord_announce_channel,
        enable_unifi,
        unifi_url,
        unifi_username,
        unifi_password,
        version,
    })
}

fn pick_version<P: Prompter>(args: &InstallArgs, prompts: &P) -> Result<String> {
    use crate::version_selector::{parse_releases, select_default_stable, top_stable};

    if let Some(v) = &args.version {
        return Ok(v.clone());
    }
    let raw = if let Some(payload) = &args.release_json_override {
        payload.clone()
    } else {
        match crate::github_api::fetch_recent_releases() {
            Ok(json) => json,
            Err(e) => {
                println!("warning: failed to fetch releases from GitHub ({e}); defaulting tag to 'latest'");
                return Ok("latest".to_string());
            }
        }
    };
    let releases = parse_releases(&raw).context("parsing releases JSON")?;
    let default = select_default_stable(&releases).map(|r| r.tag_name.clone());
    if args.no_prompt {
        return default.ok_or_else(|| anyhow!("no stable release available and --no-prompt is on"));
    }
    let stable = top_stable(&releases, 5);
    if stable.is_empty() {
        return default
            .ok_or_else(|| anyhow!("no stable release available; pass --version explicitly"));
    }
    let mut items: Vec<String> = stable
        .iter()
        .map(|r| format!("{} ({})", r.tag_name, r.published_at))
        .collect();
    items.push("(pick a specific tag)".into());
    let idx = prompts.select("Which Coterie version?", &items)?;
    if idx < stable.len() {
        return Ok(stable[idx].tag_name.clone());
    }
    prompts.text("Tag to install (e.g. v1.0.0)", default.as_deref())
}

fn print_summary(inputs: &ResolvedInputs) {
    println!();
    println!("Summary of inputs:");
    println!("  Org name:           {}", inputs.org_name);
    println!("  Portal domain:      {}", inputs.portal_domain);
    if let Some(m) = &inputs.marketing_domain {
        println!("  Marketing domain:   {m}");
    }
    println!("  Contact email:      {}", inputs.contact_email);
    println!(
        "  First admin:        {} <{}>",
        inputs.admin_full_name, inputs.admin_email
    );
    println!("  Admin username:     {}", inputs.admin_username);
    println!(
        "  Stripe:             {}",
        if inputs.enable_stripe {
            "enabled"
        } else {
            "disabled"
        }
    );
    println!(
        "  Discord:            {}",
        if inputs.enable_discord {
            "enabled"
        } else {
            "disabled"
        }
    );
    println!(
        "  UniFi:              {}",
        if inputs.enable_unifi {
            "enabled"
        } else {
            "disabled"
        }
    );
    println!(
        "  Caddy:              {}",
        if inputs.enable_caddy {
            "install + configure"
        } else {
            "skip"
        }
    );
    println!("  Version:            {}", inputs.version);
    println!();
}

fn step_apt<S: SystemCommand>(sys: &S, inputs: &ResolvedInputs, dry_run: bool) -> Result<()> {
    announce(dry_run, "apt-get update + install base packages");
    if dry_run {
        return Ok(());
    }
    run_step(sys, dry_run, "apt-get", &["update"])?;
    let mut pkgs: Vec<&str> = vec![
        "curl",
        "python3",
        "tar",
        "sqlite3",
        "ca-certificates",
        "openssl",
    ];
    if inputs.enable_caddy {
        pkgs.push("caddy");
    }
    let mut args: Vec<&str> = vec!["install", "-y", "--no-install-recommends"];
    args.extend(pkgs);
    run_step(sys, dry_run, "apt-get", &args)?;
    Ok(())
}

fn step_release_deploy<S: SystemCommand, F: FileSystem>(
    sys: &S,
    fs: &F,
    inputs: &ResolvedInputs,
    dry_run: bool,
) -> Result<()> {
    announce(
        dry_run,
        &format!("fetch and run release-deploy.sh for tag {}", inputs.version),
    );
    if dry_run {
        return Ok(());
    }
    if !fs.exists(Path::new(RELEASE_DEPLOY_BIN)) {
        run_step(
            sys,
            dry_run,
            "curl",
            &["-sfL", "-o", RELEASE_DEPLOY_BIN, RELEASE_DEPLOY_URL],
        )?;
        fs.chmod(Path::new(RELEASE_DEPLOY_BIN), 0o755)?;
    }
    let tag_arg = if inputs.version == "latest" {
        vec![]
    } else {
        vec![inputs.version.as_str()]
    };
    run_step(sys, dry_run, RELEASE_DEPLOY_BIN, &tag_arg)?;
    if !fs.is_file(Path::new(COTERIE_BIN)) {
        anyhow::bail!(
            "expected {} after release-deploy.sh but it is missing",
            COTERIE_BIN
        );
    }
    if !fs.is_file(Path::new(CREATE_ADMIN_BIN)) {
        anyhow::bail!(
            "expected {} after release-deploy.sh but it is missing — \
             requires a release that includes the a23 create_admin binary",
            CREATE_ADMIN_BIN
        );
    }
    Ok(())
}

fn step_write_env<F: FileSystem, P: Prompter>(
    fs: &F,
    prompts: &P,
    inputs: &ResolvedInputs,
    args: &InstallArgs,
    dry_run: bool,
) -> Result<()> {
    announce(dry_run, "render and write /opt/coterie/.env");

    let env_path = Path::new(ENV_PATH);
    let example_path = Path::new(ENV_EXAMPLE_PATH);

    if fs.exists(env_path) && !dry_run {
        let overwrite = if args.overwrite_env || args.no_prompt {
            args.overwrite_env
        } else {
            prompts.yes_no(
                "/opt/coterie/.env already exists. Overwrite with new values?",
                false,
            )?
        };
        if !overwrite {
            println!("(keeping existing /opt/coterie/.env; skipping render)");
            return Ok(());
        }
    }

    let template = if dry_run && !fs.exists(example_path) {
        include_str!("../tests/fixtures/env_example.txt").to_string()
    } else {
        fs.read_to_string(example_path)
            .with_context(|| format!("reading {}", example_path.display()))?
    };

    let base_url = format!("https://{}", inputs.portal_domain);
    let cfg = EnvConfig {
        server_host: "127.0.0.1".into(),
        server_port: 8080,
        base_url,
        data_dir: Some("/var/lib/coterie".into()),
        database_url: "sqlite://coterie.db?mode=rwc".into(),
        database_max_connections: 10,
        session_secret: generate_session_secret(),
        session_duration_hours: 24,
        totp_issuer: inputs.org_name.clone(),
        stripe_enabled: inputs.enable_stripe,
        stripe_publishable_key: inputs.stripe_pk.clone(),
        stripe_secret_key: inputs.stripe_sk.clone(),
        stripe_webhook_secret: inputs.stripe_whsec.clone(),
        discord_enabled: inputs.enable_discord,
        discord_bot_token: inputs.discord_bot_token.clone(),
        discord_guild_id: inputs.discord_guild_id.clone(),
        discord_member_role_id: None,
        discord_expired_role_id: None,
        discord_announce_channel_id: inputs.discord_announce_channel.clone(),
        unifi_enabled: inputs.enable_unifi,
        unifi_controller_url: inputs.unifi_url.clone(),
        unifi_username: inputs.unifi_username.clone(),
        unifi_password: inputs.unifi_password.clone(),
        unifi_site_id: Some("default".into()),
    };
    let rendered = render_env(&template, &cfg);
    if dry_run {
        println!("------ would write {ENV_PATH} ------");
        println!("{}", redact_for_display(&rendered));
        println!("------ end {ENV_PATH} ------");
    } else {
        fs.write(env_path, rendered.as_bytes())?;
        fs.chmod(env_path, 0o640)?;
        // Best-effort chown to coterie:coterie — install.sh created the
        // user already in step_release_deploy → release-deploy.sh.
        let _ = fs.chown(env_path, "coterie", "coterie");
    }
    Ok(())
}

fn redact_for_display(rendered: &str) -> String {
    let mut out = String::with_capacity(rendered.len());
    for line in rendered.lines() {
        if let Some((k, _)) = line.split_once('=') {
            let key = k.trim();
            if is_sensitive_key(key) {
                out.push_str(&format!("{key}=<redacted>\n"));
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn is_sensitive_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    upper.contains("SECRET")
        || upper.contains("TOKEN")
        || upper.contains("PASSWORD")
        || upper.ends_with("KEY")
}

fn step_create_admin<S: SystemCommand>(
    sys: &S,
    inputs: &ResolvedInputs,
    dry_run: bool,
) -> Result<bool> {
    announce(dry_run, "bootstrap first admin via create_admin");
    if dry_run {
        println!(
            "[plan] would write password to a mode-0600 tempfile and invoke {}",
            CREATE_ADMIN_BIN
        );
        return Ok(false);
    }

    // RAII tempfile that wipes itself on drop.
    let mut tf = tempfile::NamedTempFile::new().context("creating password tempfile")?;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(tf.path(), perms).context("setting tempfile mode 0600")?;
    tf.as_file_mut()
        .write_all(inputs.admin_password.expose_secret().as_bytes())
        .context("writing password to tempfile")?;
    tf.as_file_mut().flush().ok();
    let path = tf.path().to_path_buf();
    let result = sys.run(
        CREATE_ADMIN_BIN,
        &[
            "--password-file",
            path.to_str().unwrap(),
            "--email",
            &inputs.admin_email,
            "--username",
            &inputs.admin_username,
            "--full-name",
            &inputs.admin_full_name,
        ],
    );

    // Shred-then-drop: overwrite the file with zeros before letting
    // the NamedTempFile drop unlink it.
    {
        let metadata = std::fs::metadata(&path).ok();
        let len = metadata.map(|m| m.len()).unwrap_or(0);
        if len > 0 {
            if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(&path) {
                let zeros = vec![0u8; len as usize];
                let _ = f.write_all(&zeros);
                let _ = f.sync_all();
            }
        }
    }
    drop(tf);

    let out = result?;
    // create_admin contract from a23:
    //   exit 0  — admin created
    //   exit 2  — admin already exists (idempotent skip)
    //   other   — error
    if out.status == 0 {
        println!("first admin created.");
        Ok(false)
    } else if out.status == 2 {
        println!(
            "admin already exists in DB; skipping create_admin (output: {})",
            out.stdout.trim()
        );
        Ok(true)
    } else {
        anyhow::bail!(
            "create_admin failed (status {}): {}",
            out.status,
            out.stderr.trim()
        )
    }
}

fn step_caddy<S: SystemCommand, F: FileSystem, P: Prompter>(
    sys: &S,
    fs: &F,
    prompts: &P,
    inputs: &ResolvedInputs,
    args: &InstallArgs,
    dry_run: bool,
) -> Result<()> {
    announce(dry_run, "render and install /etc/caddy/Caddyfile");

    let target = Path::new(CADDYFILE_PATH);
    if fs.exists(target) && !dry_run {
        let existing = fs.read_to_string(target).unwrap_or_default();
        let wizard_managed = existing.contains(COTERIE_MARKER);
        let overwrite = if args.no_prompt || wizard_managed {
            true
        } else {
            prompts.yes_no(
                "/etc/caddy/Caddyfile already exists and was not written by the wizard. Overwrite?",
                false,
            )?
        };
        if !overwrite {
            println!("(keeping existing Caddyfile; skipping caddy step)");
            return Ok(());
        }
    }

    let template = if dry_run && !fs.exists(Path::new(CADDYFILE_EXAMPLE_PATH)) {
        include_str!("../../Caddyfile.example").to_string()
    } else {
        fs.read_to_string(Path::new(CADDYFILE_EXAMPLE_PATH))?
    };
    let rendered = render_caddyfile(
        &template,
        &inputs.portal_domain,
        inputs.marketing_domain.as_deref(),
    );

    if dry_run {
        println!("------ would write {CADDYFILE_PATH} ------");
        println!("{rendered}");
        println!("------ end {CADDYFILE_PATH} ------");
        println!("[plan] mkdir -p {VAR_LOG_CADDY} && chown -R caddy:caddy {VAR_LOG_CADDY}");
        println!("[plan] caddy validate --config {CADDYFILE_PATH}");
        println!("[plan] systemctl reload caddy");
        return Ok(());
    }

    fs.write(target, rendered.as_bytes())?;
    fs.create_dir_all(Path::new(VAR_LOG_CADDY))?;
    fs.chown(Path::new(VAR_LOG_CADDY), "caddy", "caddy")?;
    let validate = sys.run("caddy", &["validate", "--config", CADDYFILE_PATH])?;
    if !validate.succeeded() {
        anyhow::bail!(
            "caddy validate rejected the generated Caddyfile: {}\n{}",
            validate.stdout,
            validate.stderr
        );
    }
    run_step(sys, dry_run, "systemctl", &["reload", "caddy"])?;
    Ok(())
}

fn step_service<S: SystemCommand>(sys: &S, dry_run: bool) -> Result<()> {
    announce(dry_run, "enable + start coterie service");
    run_step(sys, dry_run, "systemctl", &["enable", "--now", "coterie"])?;
    Ok(())
}

fn step_smoke_test<S: SystemCommand>(sys: &S, dry_run: bool) -> Result<()> {
    announce(dry_run, &format!("smoke test: GET {HEALTH_URL}"));
    if dry_run {
        return Ok(());
    }
    let out = sys.run(
        "curl",
        &["-sf", "-o", "/dev/null", "-w", "%{http_code}", HEALTH_URL],
    )?;
    if !out.succeeded() || !out.stdout.trim().starts_with("200") {
        println!(
            "health smoke test failed (status {}, body: {})",
            out.status, out.stdout
        );
        let _ = sys.run("systemctl", &["status", "coterie", "--no-pager"]);
        let _ = sys.run("journalctl", &["-u", "coterie", "--no-pager", "-n", "50"]);
        anyhow::bail!("coterie is not responding at {HEALTH_URL}; see service logs above");
    }
    println!("health endpoint returned 200.");
    Ok(())
}

fn print_summary_final(inputs: &ResolvedInputs, admin_skipped: bool) {
    println!();
    println!("============================================================");
    println!("Coterie installation complete.");
    println!();
    println!("  Org name:         {}", inputs.org_name);
    println!("  Portal URL:       https://{}", inputs.portal_domain);
    println!("  Admin email:      {}", inputs.admin_email);
    if admin_skipped {
        println!("  (admin already existed; skipped create_admin)");
    }
    println!();
    println!("Next steps:");
    println!(
        "  1. Point DNS for {} at this box's public IP. Caddy will",
        inputs.portal_domain
    );
    println!("     auto-provision a TLS cert on the first inbound HTTPS request.");
    if inputs.enable_stripe {
        println!("  2. Register a Stripe webhook (see deploy/STRIPE-SETUP.md):");
        println!(
            "     URL: https://{}/api/payments/webhook/stripe",
            inputs.portal_domain
        );
    }
    println!("  3. Log in: visit https://{}/login", inputs.portal_domain);
    println!("============================================================");
}
