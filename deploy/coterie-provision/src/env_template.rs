use secrecy::{ExposeSecret, SecretString};

/// Configuration values collected from the operator that feed into
/// rendering of `/opt/coterie/.env` from the shipped `.env.example`.
#[derive(Debug, Clone)]
pub struct EnvConfig {
    pub server_host: String,
    pub server_port: u16,
    pub base_url: String,
    pub session_secret: SecretString,
    pub session_duration_hours: u32,
    pub totp_issuer: String,
    pub database_url: String,
    pub database_max_connections: u32,
    pub stripe: Option<StripeConfig>,
    pub discord: Option<DiscordConfig>,
    pub unifi: Option<UnifiConfig>,
}

#[derive(Debug, Clone)]
pub struct StripeConfig {
    pub publishable_key: String,
    pub secret_key: SecretString,
    pub webhook_secret: SecretString,
}

#[derive(Debug, Clone)]
pub struct DiscordConfig {
    pub bot_token: SecretString,
    pub guild_id: String,
    pub member_role_id: String,
    pub expired_role_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UnifiConfig {
    pub controller_url: String,
    pub username: String,
    pub password: SecretString,
    pub site_id: String,
}

impl EnvConfig {
    pub fn defaults_for(base_url: &str, session_secret: SecretString) -> Self {
        Self {
            server_host: "127.0.0.1".to_string(),
            server_port: 8080,
            base_url: base_url.to_string(),
            session_secret,
            session_duration_hours: 24,
            totp_issuer: "Coterie".to_string(),
            database_url: "sqlite://coterie.db".to_string(),
            database_max_connections: 10,
            stripe: None,
            discord: None,
            unifi: None,
        }
    }
}

/// Render `.env` from the `.env.example` template. The strategy:
///
/// 1. Replace known KEY=VALUE lines whose key is a config field we know
///    about. Lines we don't recognize are left untouched (forward-compat
///    with future `.env.example` additions).
/// 2. For optional integrations, when enabled, swap the
///    `…__ENABLED=false` line to `true`, and uncomment + fill in the
///    integration-specific values. When disabled, leave the example
///    structure as-is so a future operator can see the shape.
pub fn render_env(template: &str, config: &EnvConfig) -> String {
    let mut out = String::with_capacity(template.len());

    for line in template.split_inclusive('\n') {
        let raw = line.trim_end_matches('\n').trim_end_matches('\r');

        // Lines we always rewrite if they are present.
        if let Some(rewritten) = rewrite_required_line(raw, config) {
            out.push_str(&rewritten);
            out.push('\n');
            continue;
        }

        // Optional Stripe integration.
        if let Some(rewritten) = rewrite_stripe_line(raw, config) {
            out.push_str(&rewritten);
            out.push('\n');
            continue;
        }

        // Optional Discord integration.
        if let Some(rewritten) = rewrite_discord_line(raw, config) {
            out.push_str(&rewritten);
            out.push('\n');
            continue;
        }

        // Optional UniFi integration.
        if let Some(rewritten) = rewrite_unifi_line(raw, config) {
            out.push_str(&rewritten);
            out.push('\n');
            continue;
        }

        // Default: pass through.
        out.push_str(line);
    }
    out
}

fn rewrite_required_line(raw: &str, config: &EnvConfig) -> Option<String> {
    let (key, _) = split_kv(raw)?;
    let new_value = match key {
        "COTERIE__SERVER__HOST" => config.server_host.clone(),
        "COTERIE__SERVER__PORT" => config.server_port.to_string(),
        "COTERIE__SERVER__BASE_URL" => config.base_url.clone(),
        "COTERIE__DATABASE__URL" => config.database_url.clone(),
        "COTERIE__DATABASE__MAX_CONNECTIONS" => config.database_max_connections.to_string(),
        "COTERIE__AUTH__SESSION_SECRET" => config.session_secret.expose_secret().clone(),
        "COTERIE__AUTH__SESSION_DURATION_HOURS" => config.session_duration_hours.to_string(),
        "COTERIE__AUTH__TOTP_ISSUER" => config.totp_issuer.clone(),
        _ => return None,
    };
    Some(format!("{key}={new_value}"))
}

fn rewrite_stripe_line(raw: &str, config: &EnvConfig) -> Option<String> {
    // Toggle line first (always uncommented in the template).
    if let Some((k, _)) = split_kv(raw) {
        if k == "COTERIE__STRIPE__ENABLED" {
            let val = if config.stripe.is_some() {
                "true"
            } else {
                "false"
            };
            return Some(format!("COTERIE__STRIPE__ENABLED={val}"));
        }
    }
    // Possibly-commented credential lines.
    let stripe = config.stripe.as_ref()?;
    if let Some(k) = match_commented_key(raw, "COTERIE__STRIPE__PUBLISHABLE_KEY") {
        return Some(format!("{k}={}", stripe.publishable_key));
    }
    if let Some(k) = match_commented_key(raw, "COTERIE__STRIPE__SECRET_KEY") {
        return Some(format!("{k}={}", stripe.secret_key.expose_secret()));
    }
    if let Some(k) = match_commented_key(raw, "COTERIE__STRIPE__WEBHOOK_SECRET") {
        return Some(format!("{k}={}", stripe.webhook_secret.expose_secret()));
    }
    None
}

fn rewrite_discord_line(raw: &str, config: &EnvConfig) -> Option<String> {
    if let Some(k) = match_commented_key(raw, "COTERIE__INTEGRATIONS__DISCORD__ENABLED") {
        let val = if config.discord.is_some() {
            "true"
        } else {
            "false"
        };
        return Some(format!("{k}={val}"));
    }
    let d = config.discord.as_ref()?;
    if let Some(k) = match_commented_key(raw, "COTERIE__INTEGRATIONS__DISCORD__BOT_TOKEN") {
        return Some(format!("{k}={}", d.bot_token.expose_secret()));
    }
    if let Some(k) = match_commented_key(raw, "COTERIE__INTEGRATIONS__DISCORD__GUILD_ID") {
        return Some(format!("{k}={}", d.guild_id));
    }
    if let Some(k) = match_commented_key(raw, "COTERIE__INTEGRATIONS__DISCORD__MEMBER_ROLE_ID") {
        return Some(format!("{k}={}", d.member_role_id));
    }
    if let Some(k) = match_commented_key(raw, "COTERIE__INTEGRATIONS__DISCORD__EXPIRED_ROLE_ID") {
        if let Some(eid) = d.expired_role_id.as_ref() {
            return Some(format!("{k}={eid}"));
        }
    }
    None
}

fn rewrite_unifi_line(raw: &str, config: &EnvConfig) -> Option<String> {
    if let Some(k) = match_commented_key(raw, "COTERIE__INTEGRATIONS__UNIFI__ENABLED") {
        let val = if config.unifi.is_some() {
            "true"
        } else {
            "false"
        };
        return Some(format!("{k}={val}"));
    }
    let u = config.unifi.as_ref()?;
    if let Some(k) = match_commented_key(raw, "COTERIE__INTEGRATIONS__UNIFI__CONTROLLER_URL") {
        return Some(format!("{k}={}", u.controller_url));
    }
    if let Some(k) = match_commented_key(raw, "COTERIE__INTEGRATIONS__UNIFI__USERNAME") {
        return Some(format!("{k}={}", u.username));
    }
    if let Some(k) = match_commented_key(raw, "COTERIE__INTEGRATIONS__UNIFI__PASSWORD") {
        return Some(format!("{k}={}", u.password.expose_secret()));
    }
    if let Some(k) = match_commented_key(raw, "COTERIE__INTEGRATIONS__UNIFI__SITE_ID") {
        return Some(format!("{k}={}", u.site_id));
    }
    None
}

fn split_kv(raw: &str) -> Option<(&str, &str)> {
    if raw.starts_with('#') {
        return None;
    }
    let eq = raw.find('=')?;
    let k = &raw[..eq];
    if k.is_empty() {
        return None;
    }
    Some((k, &raw[eq + 1..]))
}

/// Match a line of the form `# COTERIE__FOO=bar` (with optional `# `
/// prefix and surrounding whitespace) where the key matches `expected`.
/// Returns the bare key (the uncommented form) on a match.
fn match_commented_key<'a>(raw: &str, expected: &'a str) -> Option<&'a str> {
    let trimmed = raw.trim_start();
    let body = trimmed
        .strip_prefix('#')
        .map(str::trim_start)
        .unwrap_or(trimmed);
    let eq = body.find('=')?;
    let key = &body[..eq];
    if key == expected {
        Some(expected)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::SecretString;

    fn fixture() -> &'static str {
        include_str!("../tests/fixtures/env_example.txt")
    }

    fn base_config() -> EnvConfig {
        EnvConfig {
            server_host: "127.0.0.1".to_string(),
            server_port: 8080,
            base_url: "https://coterie.example.com".to_string(),
            session_secret: SecretString::new("deadbeef".repeat(8)),
            session_duration_hours: 24,
            totp_issuer: "Coterie".to_string(),
            database_url: "sqlite://coterie.db".to_string(),
            database_max_connections: 10,
            stripe: None,
            discord: None,
            unifi: None,
        }
    }

    #[test]
    fn no_integrations_basic_replacement() {
        let out = render_env(fixture(), &base_config());
        assert!(out.contains("COTERIE__SERVER__BASE_URL=https://coterie.example.com"));
        assert!(out.contains(
            "COTERIE__AUTH__SESSION_SECRET=deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
        ));
        assert!(out.contains("COTERIE__STRIPE__ENABLED=false"));
        // Should not have uncommented credential lines when Stripe is
        // disabled. (The commented placeholders inherited from the
        // template are fine.)
        for line in out.lines() {
            let l = line.trim_start();
            if l.starts_with("COTERIE__STRIPE__SECRET_KEY=")
                || l.starts_with("COTERIE__STRIPE__PUBLISHABLE_KEY=")
                || l.starts_with("COTERIE__STRIPE__WEBHOOK_SECRET=")
            {
                panic!("uncommented Stripe credential leaked when disabled: {line}");
            }
        }
    }

    #[test]
    fn stripe_enabled_uncomments_keys() {
        let mut c = base_config();
        c.stripe = Some(StripeConfig {
            publishable_key: "pk_live_abc".to_string(),
            secret_key: SecretString::new("sk_live_xyz".to_string()),
            webhook_secret: SecretString::new("whsec_q".to_string()),
        });
        let out = render_env(fixture(), &c);
        assert!(out.contains("COTERIE__STRIPE__ENABLED=true"));
        assert!(out.contains("COTERIE__STRIPE__PUBLISHABLE_KEY=pk_live_abc"));
        assert!(out.contains("COTERIE__STRIPE__SECRET_KEY=sk_live_xyz"));
        assert!(out.contains("COTERIE__STRIPE__WEBHOOK_SECRET=whsec_q"));
    }

    #[test]
    fn discord_enabled_emits_credentials() {
        let mut c = base_config();
        c.discord = Some(DiscordConfig {
            bot_token: SecretString::new("token123".to_string()),
            guild_id: "111".to_string(),
            member_role_id: "222".to_string(),
            expired_role_id: Some("333".to_string()),
        });
        let out = render_env(fixture(), &c);
        assert!(out.contains("COTERIE__INTEGRATIONS__DISCORD__ENABLED=true"));
        assert!(out.contains("COTERIE__INTEGRATIONS__DISCORD__BOT_TOKEN=token123"));
        assert!(out.contains("COTERIE__INTEGRATIONS__DISCORD__GUILD_ID=111"));
        assert!(out.contains("COTERIE__INTEGRATIONS__DISCORD__MEMBER_ROLE_ID=222"));
        assert!(out.contains("COTERIE__INTEGRATIONS__DISCORD__EXPIRED_ROLE_ID=333"));
    }

    #[test]
    fn unifi_enabled_emits_credentials() {
        let mut c = base_config();
        c.unifi = Some(UnifiConfig {
            controller_url: "https://unifi.example.com:8443".to_string(),
            username: "admin".to_string(),
            password: SecretString::new("hunter2".to_string()),
            site_id: "default".to_string(),
        });
        let out = render_env(fixture(), &c);
        assert!(out.contains("COTERIE__INTEGRATIONS__UNIFI__ENABLED=true"));
        assert!(out.contains(
            "COTERIE__INTEGRATIONS__UNIFI__CONTROLLER_URL=https://unifi.example.com:8443"
        ));
        assert!(out.contains("COTERIE__INTEGRATIONS__UNIFI__USERNAME=admin"));
        assert!(out.contains("COTERIE__INTEGRATIONS__UNIFI__PASSWORD=hunter2"));
        assert!(out.contains("COTERIE__INTEGRATIONS__UNIFI__SITE_ID=default"));
    }
}
