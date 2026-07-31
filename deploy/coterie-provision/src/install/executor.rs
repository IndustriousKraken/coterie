//! The privileged box mutation: apt -> fetch release-deploy -> write
//! `/opt/coterie/.env` -> bootstrap admin -> chown -> Caddy -> systemd
//! -> smoke test, in that order. Depends on `SystemCommand` /
//! `FileSystem` / `Output` and takes a `&ResolvedInputs` — it never
//! prompts.

use anyhow::{anyhow, Context, Result};
use secrecy::ExposeSecret;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::caddyfile;
use crate::env_template::{self, DatabaseUrl, EnvConfig};
use crate::fs_ops::FileSystem;
use crate::output::Output;
use crate::system::SystemCommand;

use super::{
    ResolvedInputs, StripeMode, BACKUP_SERVICE_DST, BACKUP_SERVICE_SRC, BACKUP_TIMER_DST,
    BACKUP_TIMER_SRC, CADDYFILE_EXAMPLE_PATH, CADDYFILE_PATH, CADDY_LOG_DIR, DATA_DIR,
    ENV_EXAMPLE_PATH, ENV_LIVE_PATH, ENV_PATH, INSTALL_DIR, RELEASE_DEPLOY_PATH,
};

pub(super) struct Executor<'a, S: SystemCommand, F: FileSystem> {
    pub(super) sys: &'a S,
    pub(super) fs: &'a F,
    pub(super) dry_run: bool,
    pub(super) smoke_test_interval: Duration,
    pub(super) smoke_test_budget: Duration,
}

impl<'a, S: SystemCommand, F: FileSystem> Executor<'a, S, F> {
    fn step_line(&self, what: &str) -> String {
        let tag = if self.dry_run { "DRY-RUN" } else { "STEP" };
        format!("[{tag}] {what}")
    }

    pub(super) fn announce(&self, what: &str) {
        println!("{}", self.step_line(what));
    }

    pub(super) fn run(&self, cmd: &str, args: &[&str], description: &str) -> Result<()> {
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

    pub(super) fn apt_update(&self) -> Result<()> {
        self.run("apt-get", &["update"], "apt-get update")
    }

    pub(super) fn apt_install(&self, with_caddy: bool) -> Result<()> {
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

    pub(super) fn fetch_release_deploy(&self, tag: &str) -> Result<()> {
        // Always fetch the release-deploy.sh pinned to the tag we're
        // installing, rather than trusting whatever is already at
        // /opt/coterie/deploy/. A leftover copy from a failed or older
        // install (e.g. a buggy v1.0.2 bootstrap script) would otherwise
        // shadow the version we mean to install. This is the install path
        // only; updates go through `coterie-provision update`.
        let to = Path::new(RELEASE_DEPLOY_PATH);
        let url = format!(
            "https://raw.githubusercontent.com/IndustriousKraken/coterie/{tag}/deploy/release-deploy.sh"
        );
        self.run(
            "curl",
            &["-sfL", "-o", RELEASE_DEPLOY_PATH, url.as_str()],
            "fetch release-deploy.sh (pinned to install tag)",
        )?;
        if !self.dry_run {
            self.fs.chmod(to, 0o755)?;
        }
        Ok(())
    }

    pub(super) fn run_release_deploy(&self, tag: &str) -> Result<()> {
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

    pub(super) fn assert_binaries_present(&self) -> Result<()> {
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

    pub(super) fn render_and_write_env(&self, inputs: &ResolvedInputs) -> Result<()> {
        if !inputs.overwrite_env {
            self.announce("skipping .env render (existing file preserved)");
            return Ok(());
        }
        let template = if self.dry_run && !self.fs.is_file(Path::new(ENV_EXAMPLE_PATH)) {
            // For dry-run on a fresh box where release-deploy.sh
            // hasn't actually placed .env.example, fall back to the
            // embedded fixture so the rendered .env preview is still
            // accurate.
            include_str!("../../tests/fixtures/env_example.txt").to_string()
        } else {
            self.fs.read_to_string(Path::new(ENV_EXAMPLE_PATH))?
        };

        let base_url = format!("https://{}", inputs.portal_domain);
        let mut env_config = EnvConfig::defaults_for(&base_url, inputs.session_secret.clone());
        // A marketing domain means a separate marketing site's browser
        // will call /public/*; allow its origin(s) through CORS. No
        // marketing domain leaves cors_origins None (same-origin default).
        env_config.cors_origins = inputs
            .marketing_domain
            .as_deref()
            .map(env_template::cors_origins_for);
        // When test mode is selected, route to a separate sqlite file so
        // test charges/members don't land in what will become the
        // production DB. The switchover subcommand rewrites this back to
        // `coterie.db` when the operator transitions to live mode.
        if inputs.enable_stripe && inputs.stripe_mode == StripeMode::Test {
            env_config.database_url = DatabaseUrl::Test.as_env_str().to_string();
        }
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
            .with_context(|| format!("chown coterie:coterie {ENV_PATH}"))?;
        Ok(())
    }

    /// Stage `/opt/coterie/.env.live` if the operator pre-loaded a live
    /// Stripe triple during a test-mode wizard run. The switchover
    /// subcommand consumes (and removes) this file later.
    pub(super) fn write_live_overlay_if_needed(&self, inputs: &ResolvedInputs) -> Result<()> {
        let (Some(pk), Some(sk), Some(whsec)) = (
            inputs.stripe_live_publishable_key.as_ref(),
            inputs.stripe_live_secret_key.as_ref(),
            inputs.stripe_live_webhook_secret.as_ref(),
        ) else {
            return Ok(());
        };
        let rendered = env_template::render_live_overlay(
            pk,
            &secrecy::Secret::new(sk.expose_secret().clone()),
            &secrecy::Secret::new(whsec.expose_secret().clone()),
        );
        self.announce(&format!(
            "writing {} (live creds staged for switchover)",
            ENV_LIVE_PATH
        ));
        if self.dry_run {
            println!("--- {ENV_LIVE_PATH} preview (dry-run) ---");
            for line in rendered.lines() {
                println!("    {line}");
            }
            return Ok(());
        }
        self.fs
            .write(Path::new(ENV_LIVE_PATH), rendered.as_bytes())?;
        self.fs.chmod(Path::new(ENV_LIVE_PATH), 0o640)?;
        self.fs
            .chown(Path::new(ENV_LIVE_PATH), "coterie", "coterie")
            .with_context(|| format!("chown coterie:coterie {ENV_LIVE_PATH}"))?;
        Ok(())
    }

    pub(super) fn bootstrap_admin(&self, inputs: &ResolvedInputs) -> Result<()> {
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

        // Call sys.run directly (rather than run_allow_codes) so we can
        // produce the spec-required "exited unexpectedly with code N"
        // message for any non-{0,2} exit code, instead of bouncing off
        // the run_allow_codes whitelist with a generic "failed" string.
        let result = self.sys.run(
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
        );

        // Always shred-then-drop, regardless of create_admin's exit code.
        let _ = overwrite_with_zeros(tmp.path());
        drop(tmp);

        let out = result.context("running create_admin")?;
        match out.status {
            0 => {
                self.announce("first admin created");
                Ok(())
            }
            2 => {
                self.announce("admin already exists — skipping create_admin (idempotent)");
                Ok(())
            }
            // RealSystem maps signal-terminated children to -1.
            -1 => Err(anyhow!(
                "create_admin terminated by signal; stdout: {}\nstderr: {}",
                out.stdout,
                out.stderr
            )),
            other => Err(anyhow!(
                "create_admin exited unexpectedly with code {other}; stdout: {}\nstderr: {}",
                out.stdout,
                out.stderr
            )),
        }
    }

    /// create_admin runs as root and creates the SQLite DB as its very
    /// first operation, so the db (+ -wal/-shm) land root-owned. The
    /// service runs as `coterie` and must be able to write them, so chown
    /// the data dir after bootstrap. Idempotent on re-runs.
    pub(super) fn chown_data_dir(&self) -> Result<()> {
        self.run(
            "chown",
            &["-R", "coterie:coterie", DATA_DIR],
            "chown data dir (coterie owns the DB create_admin made as root)",
        )
    }

    pub(super) fn write_caddyfile(&self, inputs: &ResolvedInputs) -> Result<()> {
        let template = if self.dry_run && !self.fs.is_file(Path::new(CADDYFILE_EXAMPLE_PATH)) {
            include_str!("../../tests/fixtures/caddyfile_example.txt").to_string()
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
                .with_context(|| format!("chown caddy:caddy {CADDY_LOG_DIR}"))?;
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

    pub(super) fn enable_and_start_service(&self) -> Result<()> {
        self.run(
            "systemctl",
            &["enable", "--now", "coterie"],
            "systemctl enable --now coterie",
        )
    }

    /// Install `coterie-backup.service` + `coterie-backup.timer` and
    /// enable the timer, so a provisioned host has scheduled backups
    /// without the operator having to know to do it by hand. Shipping a
    /// backup script that nothing schedules is not delivering backups:
    /// the gap is invisible until a restore is needed, which is how a
    /// production instance ran three weeks with none.
    ///
    /// Idempotent — a re-run that finds BOTH units already in place
    /// leaves them alone rather than installing a second copy. A host
    /// that has only one of them is repaired rather than skipped: a
    /// timer whose service unit is missing is enabled and scheduled but
    /// fails every time it fires, which looks exactly like working
    /// backups until someone reads the journal.
    ///
    /// Announces through `Output` (rather than the `println!`-based
    /// `announce`) so `--dry-run` can be asserted on in tests without
    /// scraping process stdout.
    pub(super) fn install_backup_timer<O: Output>(&self, output: &O) -> Result<()> {
        let missing: Vec<(&str, &str)> = [
            (BACKUP_SERVICE_SRC, BACKUP_SERVICE_DST),
            (BACKUP_TIMER_SRC, BACKUP_TIMER_DST),
        ]
        .into_iter()
        .filter(|(_, dst)| !self.fs.is_file(Path::new(dst)))
        .collect();

        if missing.is_empty() {
            output.println(&self.step_line(
                "scheduled backups: units already installed — leaving the existing schedule alone",
            ));
            return Ok(());
        }
        let installing: Vec<&str> = missing.iter().map(|(_, dst)| *dst).collect();
        output.println(&self.step_line(&format!(
            "scheduled backups: install {}, then enable the timer",
            installing.join(" + ")
        )));
        if !self.dry_run {
            for (src, dst) in &missing {
                self.fs
                    .copy_file(Path::new(src), Path::new(dst))
                    .with_context(|| format!("installing {dst}"))?;
            }
        }
        self.run("systemctl", &["daemon-reload"], "systemctl daemon-reload")?;
        self.run(
            "systemctl",
            &["enable", "--now", "coterie-backup.timer"],
            "systemctl enable --now coterie-backup.timer",
        )
    }

    pub(super) fn smoke_test(&self) -> Result<()> {
        if self.dry_run {
            self.announce("would GET http://127.0.0.1:8080/health");
            return Ok(());
        }
        self.announce(&format!(
            "smoke test GET /health (polling up to {}s)",
            self.smoke_test_budget.as_secs()
        ));
        smoke_test(self.sys, self.smoke_test_interval, self.smoke_test_budget)
    }
}

/// Poll `/health` via `curl` until a 2xx response or the budget
/// exhausts. Shared by the install flow and the update flow so both use
/// identical smoke-test semantics.
///
/// `curl` is used (rather than an in-process HTTP client) so the check
/// routes through the `SystemCommand` trait and is mockable in tests.
/// `-fsSL` yields a non-zero exit on any HTTP error. Polling covers the
/// documented race between systemd reporting `active` and the HTTP
/// listener binding (sqlx pool init, first-connection migrations,
/// address binding).
pub fn smoke_test<S: SystemCommand>(sys: &S, interval: Duration, budget: Duration) -> Result<()> {
    let deadline = Instant::now() + budget;
    loop {
        let last_error = match sys.run("curl", &["-fsSL", "http://127.0.0.1:8080/health"]) {
            Ok(out) if out.success() => return Ok(()),
            Ok(out) => format!(
                "status={}, stdout={}, stderr={}",
                out.status, out.stdout, out.stderr
            ),
            Err(e) => format!("{e}"),
        };
        if Instant::now() >= deadline {
            return Err(anyhow!(
                "smoke test failed after {}s: {}",
                budget.as_secs(),
                last_error
            ));
        }
        sleep(interval);
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
