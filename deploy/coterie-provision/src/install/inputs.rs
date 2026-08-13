//! The operator interview: flag -> `COTERIE_PROVISION_*` env ->
//! interactive prompt, plus every prefix/format check on what comes
//! back. Depends on `Prompter` only — it never touches the box.

use anyhow::{anyhow, Result};
use rand::RngCore;
use secrecy::{ExposeSecret, SecretString};

use crate::prompts::{resolve, resolve_bool, resolve_secret, Prompter};
use crate::stripe_check;

use super::{InstallArgs, PreflightState, StripeMode};

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
    pub stripe_mode: StripeMode,
    pub stripe_publishable_key: Option<String>,
    pub stripe_secret_key: Option<SecretString>,
    pub stripe_webhook_secret: Option<SecretString>,
    /// Live triple staged into `/opt/coterie/.env.live` when the operator
    /// pre-loaded them during a test-mode wizard run. None outside that
    /// path.
    pub stripe_live_publishable_key: Option<String>,
    pub stripe_live_secret_key: Option<SecretString>,
    pub stripe_live_webhook_secret: Option<SecretString>,
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

/// Normalize the operator-supplied marketing domain to the trimmed apex:
/// strip surrounding whitespace and a single leading `www.`. Returns
/// `None` for blank (or bare `www.`) input. Both downstream consumers —
/// the Caddy vhost (`{apex}, www.{apex}`) and the CORS allowlist
/// (`https://{apex},https://www.{apex}`) — then derive the same apex+www
/// pair, instead of diverging into a `www.www.` vhost for a `www.`-prefixed
/// input.
fn normalize_marketing_domain(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let apex = trimmed.strip_prefix("www.").unwrap_or(trimmed);
    if apex.is_empty() {
        None
    } else {
        Some(apex.to_string())
    }
}

pub(super) fn gather_inputs<P: Prompter>(
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
    let marketing_domain = normalize_marketing_domain(&marketing_domain_raw);

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

    let stripe = gather_stripe_inputs(args, prompts, no_prompt)?;

    let discord = gather_discord_inputs(args, prompts, no_prompt)?;

    let unifi = gather_unifi_inputs(args, prompts, no_prompt)?;

    let enable_caddy = gather_caddy_inputs(args, prompts, no_prompt)?;

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
        enable_stripe: stripe.enable_stripe,
        stripe_mode: stripe.stripe_mode,
        stripe_publishable_key: stripe.stripe_publishable_key,
        stripe_secret_key: stripe.stripe_secret_key,
        stripe_webhook_secret: stripe.stripe_webhook_secret,
        stripe_live_publishable_key: stripe.stripe_live_publishable_key,
        stripe_live_secret_key: stripe.stripe_live_secret_key,
        stripe_live_webhook_secret: stripe.stripe_live_webhook_secret,
        enable_discord: discord.enable_discord,
        discord_bot_token: discord.discord_bot_token,
        discord_guild_id: discord.discord_guild_id,
        discord_member_role_id: discord.discord_member_role_id,
        discord_expired_role_id: discord.discord_expired_role_id,
        enable_unifi: unifi.enable_unifi,
        unifi_controller_url: unifi.unifi_controller_url,
        unifi_username: unifi.unifi_username,
        unifi_password: unifi.unifi_password,
        unifi_site_id: unifi.unifi_site_id,
        enable_caddy,
        version,
        overwrite_env,
        session_secret,
    })
}

/// Stripe configuration collected by the wizard, mirroring the Stripe
/// fields of `ResolvedInputs`. Assembled into `ResolvedInputs` by
/// `gather_inputs`.
struct StripeInputs {
    enable_stripe: bool,
    stripe_mode: StripeMode,
    stripe_publishable_key: Option<String>,
    stripe_secret_key: Option<SecretString>,
    stripe_webhook_secret: Option<SecretString>,
    stripe_live_publishable_key: Option<String>,
    stripe_live_secret_key: Option<SecretString>,
    stripe_live_webhook_secret: Option<SecretString>,
}

/// Discord configuration collected by the wizard.
struct DiscordInputs {
    enable_discord: bool,
    discord_bot_token: Option<SecretString>,
    discord_guild_id: Option<String>,
    discord_member_role_id: Option<String>,
    discord_expired_role_id: Option<String>,
}

/// UniFi configuration collected by the wizard.
struct UnifiInputs {
    enable_unifi: bool,
    unifi_controller_url: Option<String>,
    unifi_username: Option<String>,
    unifi_password: Option<SecretString>,
    unifi_site_id: Option<String>,
}

/// Collect Stripe inputs: the enable gate, then (when enabled) the
/// test/live mode branch and the optional live-credential pre-load.
/// Flag -> env -> prompt resolution order and every prefix check are
/// unchanged from the inline version.
fn gather_stripe_inputs<P: Prompter>(
    args: &InstallArgs,
    prompts: &P,
    no_prompt: bool,
) -> Result<StripeInputs> {
    let enable_stripe = resolve_bool(
        "enable-stripe",
        "COTERIE_PROVISION_ENABLE_STRIPE",
        args.enable_stripe,
        Some(false),
        no_prompt,
        || prompts.prompt_yn("Enable Stripe integration?", false),
    )?;

    let (
        stripe_mode,
        stripe_pk,
        stripe_sk,
        stripe_whsec,
        stripe_live_pk,
        stripe_live_sk,
        stripe_live_whsec,
    ) = if enable_stripe {
        // Mode prompt: defaults to `live` to match a24's baseline. The
        // `resolve` helper handles flag/env/prompt uniformly.
        let mode = resolve::<StripeMode, _>(
            "stripe-mode",
            "COTERIE_PROVISION_STRIPE_MODE",
            args.stripe_mode,
            Some(StripeMode::Live),
            no_prompt,
            || {
                let items = vec!["live".to_string(), "test".to_string()];
                let idx = prompts.prompt_select("Stripe mode (test or live)?", &items)?;
                items[idx]
                    .parse::<StripeMode>()
                    .map_err(|e| anyhow!("invalid stripe mode: {e}"))
            },
        )?;

        match mode {
            StripeMode::Live => {
                let pk = resolve(
                    "stripe-publishable-key",
                    "COTERIE_PROVISION_STRIPE_PK",
                    args.stripe_publishable_key.clone(),
                    None,
                    no_prompt,
                    || prompts.prompt_text("Stripe publishable key (pk_…)", None),
                )?;
                // Live mode wizard matches a24 baseline: accept either
                // test or live family. (Operators occasionally configure
                // a "live mode" instance with test keys for testing.)
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
                (mode, Some(pk), Some(sk), Some(whsec), None, None, None)
            }
            StripeMode::Test => {
                let pk = resolve(
                    "stripe-publishable-key",
                    "COTERIE_PROVISION_STRIPE_PK",
                    args.stripe_publishable_key.clone(),
                    None,
                    no_prompt,
                    || prompts.prompt_text("Stripe TEST publishable key (pk_test_…)", None),
                )?;
                stripe_check::validate_prefix(&pk, "pk_test_")?;
                let sk = resolve_secret(
                    "stripe-secret-key",
                    "COTERIE_PROVISION_STRIPE_SK",
                    args.stripe_secret_key.clone(),
                    no_prompt,
                    || prompts.prompt_secret("Stripe TEST secret key (sk_test_…) — input hidden"),
                )?;
                stripe_check::validate_prefix(sk.expose_secret(), "sk_test_")?;
                let whsec = resolve_secret(
                    "stripe-webhook-secret",
                    "COTERIE_PROVISION_STRIPE_WHSEC",
                    args.stripe_webhook_secret.clone(),
                    no_prompt,
                    || prompts.prompt_secret("Stripe TEST webhook signing secret (whsec_…)"),
                )?;
                stripe_check::validate_prefix(whsec.expose_secret(), "whsec_")?;

                // Optional pre-load of live creds. Programmatic users
                // can either set preload_live_creds=true, or simply
                // supply the three live values (we infer "yes" from
                // that). Interactive users get a y/N prompt.
                let already_supplied = args.stripe_live_publishable_key.is_some()
                    && args.stripe_live_secret_key.is_some()
                    && args.stripe_live_webhook_secret.is_some();
                let preload = if let Some(b) = args.preload_live_creds {
                    b
                } else if already_supplied {
                    true
                } else if no_prompt {
                    false
                } else {
                    prompts.prompt_yn(
                        "Do you also have live credentials to pre-load for later switchover?",
                        false,
                    )?
                };

                let (live_pk, live_sk, live_whsec) = if preload {
                    let lpk = resolve(
                        "stripe-live-pk",
                        "COTERIE_PROVISION_STRIPE_LIVE_PK",
                        args.stripe_live_publishable_key.clone(),
                        None,
                        no_prompt,
                        || prompts.prompt_text("Stripe LIVE publishable key (pk_live_…)", None),
                    )?;
                    stripe_check::validate_prefix(&lpk, "pk_live_")?;
                    let lsk = resolve_secret(
                        "stripe-live-sk",
                        "COTERIE_PROVISION_STRIPE_LIVE_SK",
                        args.stripe_live_secret_key.clone(),
                        no_prompt,
                        || {
                            prompts
                                .prompt_secret("Stripe LIVE secret key (sk_live_…) — input hidden")
                        },
                    )?;
                    stripe_check::validate_prefix(lsk.expose_secret(), "sk_live_")?;
                    let lwhsec = resolve_secret(
                        "stripe-live-whsec",
                        "COTERIE_PROVISION_STRIPE_LIVE_WHSEC",
                        args.stripe_live_webhook_secret.clone(),
                        no_prompt,
                        || prompts.prompt_secret("Stripe LIVE webhook signing secret (whsec_…)"),
                    )?;
                    stripe_check::validate_prefix(lwhsec.expose_secret(), "whsec_")?;
                    (Some(lpk), Some(lsk), Some(lwhsec))
                } else {
                    (None, None, None)
                };

                (
                    mode,
                    Some(pk),
                    Some(sk),
                    Some(whsec),
                    live_pk,
                    live_sk,
                    live_whsec,
                )
            }
        }
    } else {
        (StripeMode::Live, None, None, None, None, None, None)
    };

    Ok(StripeInputs {
        enable_stripe,
        stripe_mode,
        stripe_publishable_key: stripe_pk,
        stripe_secret_key: stripe_sk,
        stripe_webhook_secret: stripe_whsec,
        stripe_live_publishable_key: stripe_live_pk,
        stripe_live_secret_key: stripe_live_sk,
        stripe_live_webhook_secret: stripe_live_whsec,
    })
}

/// Collect Discord inputs: the enable gate, then (when enabled) the bot
/// token, guild ID, and role IDs.
fn gather_discord_inputs<P: Prompter>(
    args: &InstallArgs,
    prompts: &P,
    no_prompt: bool,
) -> Result<DiscordInputs> {
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

    Ok(DiscordInputs {
        enable_discord,
        discord_bot_token: discord_token,
        discord_guild_id: discord_guild,
        discord_member_role_id: discord_member_role,
        discord_expired_role_id: discord_expired_role,
    })
}

/// Collect UniFi inputs: the enable gate, then (when enabled) the
/// controller URL, credentials, and site ID.
fn gather_unifi_inputs<P: Prompter>(
    args: &InstallArgs,
    prompts: &P,
    no_prompt: bool,
) -> Result<UnifiInputs> {
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

    Ok(UnifiInputs {
        enable_unifi,
        unifi_controller_url: unifi_url,
        unifi_username: unifi_user,
        unifi_password: unifi_pw,
        unifi_site_id: unifi_site,
    })
}

/// Collect the Caddy enable gate.
fn gather_caddy_inputs<P: Prompter>(
    args: &InstallArgs,
    prompts: &P,
    no_prompt: bool,
) -> Result<bool> {
    resolve_bool(
        "enable-caddy",
        "COTERIE_PROVISION_ENABLE_CADDY",
        args.enable_caddy,
        Some(true),
        no_prompt,
        || prompts.prompt_yn("Install and configure Caddy?", true),
    )
}

fn generate_session_secret() -> SecretString {
    let mut buf = [0u8; 32];
    rand::rng().fill_bytes(&mut buf);
    SecretString::new(hex::encode(buf))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::run;
    use crate::install::tests::make_args;
    use crate::output::CaptureOutput;
    use crate::test_support::{FakeFs, FakeSystem, MockPrompter};

    /// The session secret is the one value this tool generates that an
    /// attacker would want to predict. A seedable generator here would
    /// still emit 64 hex chars and pass every other check.
    #[test]
    fn session_secret_is_32_bytes_and_never_repeats() {
        let draws = 1000;
        let secrets: std::collections::HashSet<String> = (0..draws)
            .map(|_| {
                let s = generate_session_secret();
                let hex = s.expose_secret().to_string();
                assert_eq!(hex.len(), 64, "32 bytes hex-encoded is 64 chars");
                hex
            })
            .collect();
        assert_eq!(secrets.len(), draws, "session secrets must not repeat");
    }

    #[test]
    fn missing_required_input_fails_under_no_prompt() {
        let mut args = make_args();
        args.org_name = None;
        std::env::remove_var("COTERIE_PROVISION_ORG_NAME");

        let sys = FakeSystem::new();
        let fs = FakeFs::new();
        let prompts = MockPrompter::new();
        let out = CaptureOutput::new();
        let err = run(args, &sys, &fs, &prompts, &out).unwrap_err();
        assert!(err.to_string().contains("org-name") || err.to_string().contains("ORG_NAME"));
    }

    #[test]
    fn stripe_bad_prefix_rejected() {
        let mut args = make_args();
        args.enable_stripe = Some(true);
        args.stripe_mode = Some(StripeMode::Live);
        args.stripe_publishable_key = Some("pk_invalid_abc".to_string());
        args.stripe_secret_key = Some(SecretString::new("sk_test_xyz".to_string()));
        args.stripe_webhook_secret = Some(SecretString::new("whsec_zzz".to_string()));

        let sys = FakeSystem::new();
        let fs = FakeFs::new();
        let prompts = MockPrompter::new();
        let out = CaptureOutput::new();
        let err = run(args, &sys, &fs, &prompts, &out).unwrap_err();
        assert!(err.to_string().contains("pk_test_") || err.to_string().contains("pk_live_"));
    }

    #[test]
    fn normalize_marketing_domain_trims_and_strips_www() {
        // Trimmed apex passes through; blank/bare-www => None; a `www.`
        // prefix is stripped to the apex so the Caddy vhost and CORS
        // allowlist derive the same apex+www pair.
        assert_eq!(
            normalize_marketing_domain("  example.org  "),
            Some("example.org".to_string())
        );
        assert_eq!(
            normalize_marketing_domain("www.example.org"),
            Some("example.org".to_string())
        );
        assert_eq!(normalize_marketing_domain("   "), None);
        assert_eq!(normalize_marketing_domain("www."), None);
    }
}
