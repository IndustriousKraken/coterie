use secrecy::ExposeSecret;
use secrecy::Secret;

#[derive(Debug, Clone)]
pub struct EnvConfig {
    pub server_host: String,
    pub server_port: u16,
    pub base_url: String,
    pub data_dir: Option<String>,
    pub database_url: String,
    pub database_max_connections: u32,
    pub session_secret: Secret<String>,
    pub session_duration_hours: u32,
    pub totp_issuer: String,

    pub stripe_enabled: bool,
    pub stripe_publishable_key: Option<String>,
    pub stripe_secret_key: Option<Secret<String>>,
    pub stripe_webhook_secret: Option<Secret<String>>,

    pub discord_enabled: bool,
    pub discord_bot_token: Option<Secret<String>>,
    pub discord_guild_id: Option<String>,
    pub discord_member_role_id: Option<String>,
    pub discord_expired_role_id: Option<String>,
    pub discord_announce_channel_id: Option<String>,

    pub unifi_enabled: bool,
    pub unifi_controller_url: Option<String>,
    pub unifi_username: Option<String>,
    pub unifi_password: Option<Secret<String>>,
    pub unifi_site_id: Option<String>,
}

impl Default for EnvConfig {
    fn default() -> Self {
        Self {
            server_host: "127.0.0.1".into(),
            server_port: 8080,
            base_url: String::new(),
            data_dir: None,
            database_url: "sqlite://coterie.db".into(),
            database_max_connections: 10,
            session_secret: Secret::new(String::new()),
            session_duration_hours: 24,
            totp_issuer: "Coterie".into(),
            stripe_enabled: false,
            stripe_publishable_key: None,
            stripe_secret_key: None,
            stripe_webhook_secret: None,
            discord_enabled: false,
            discord_bot_token: None,
            discord_guild_id: None,
            discord_member_role_id: None,
            discord_expired_role_id: None,
            discord_announce_channel_id: None,
            unifi_enabled: false,
            unifi_controller_url: None,
            unifi_username: None,
            unifi_password: None,
            unifi_site_id: None,
        }
    }
}

/// Render `.env` content by replacing/uncommenting known KEY=VALUE lines
/// in `template`. Lines that aren't recognized are kept verbatim — this
/// means the .env.example evolution rides along automatically.
pub fn render_env(template: &str, config: &EnvConfig) -> String {
    let mut out = String::with_capacity(template.len() + 256);
    for raw in template.lines() {
        out.push_str(&render_line(raw, config));
        out.push('\n');
    }
    out
}

fn render_line(line: &str, config: &EnvConfig) -> String {
    if let Some((key, _)) = parse_kv_line(line) {
        if let Some(replacement) = replace_for_key(key, config) {
            return replacement;
        }
    }
    line.to_string()
}

fn parse_kv_line(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_start();
    let bare = trimmed.strip_prefix("# ").unwrap_or(trimmed);
    let (key, value) = bare.split_once('=')?;
    let key = key.trim();
    if key.is_empty() || !key.starts_with("COTERIE__") {
        return None;
    }
    Some((key, value))
}

fn replace_for_key(key: &str, config: &EnvConfig) -> Option<String> {
    match key {
        "COTERIE__SERVER__HOST" => Some(format!("COTERIE__SERVER__HOST={}", config.server_host)),
        "COTERIE__SERVER__PORT" => Some(format!("COTERIE__SERVER__PORT={}", config.server_port)),
        "COTERIE__SERVER__BASE_URL" => {
            Some(format!("COTERIE__SERVER__BASE_URL={}", config.base_url))
        }
        "COTERIE__SERVER__DATA_DIR" => config
            .data_dir
            .as_ref()
            .map(|d| format!("COTERIE__SERVER__DATA_DIR={d}")),
        "COTERIE__DATABASE__URL" => Some(format!("COTERIE__DATABASE__URL={}", config.database_url)),
        "COTERIE__DATABASE__MAX_CONNECTIONS" => Some(format!(
            "COTERIE__DATABASE__MAX_CONNECTIONS={}",
            config.database_max_connections
        )),
        "COTERIE__AUTH__SESSION_SECRET" => Some(format!(
            "COTERIE__AUTH__SESSION_SECRET={}",
            config.session_secret.expose_secret()
        )),
        "COTERIE__AUTH__SESSION_DURATION_HOURS" => Some(format!(
            "COTERIE__AUTH__SESSION_DURATION_HOURS={}",
            config.session_duration_hours
        )),
        "COTERIE__AUTH__TOTP_ISSUER" => {
            Some(format!("COTERIE__AUTH__TOTP_ISSUER={}", config.totp_issuer))
        }
        "COTERIE__STRIPE__ENABLED" => Some(format!(
            "COTERIE__STRIPE__ENABLED={}",
            config.stripe_enabled
        )),
        "COTERIE__STRIPE__PUBLISHABLE_KEY" => render_optional(
            "COTERIE__STRIPE__PUBLISHABLE_KEY",
            config.stripe_publishable_key.as_deref(),
            config.stripe_enabled,
        ),
        "COTERIE__STRIPE__SECRET_KEY" => render_optional_secret(
            "COTERIE__STRIPE__SECRET_KEY",
            config.stripe_secret_key.as_ref(),
            config.stripe_enabled,
        ),
        "COTERIE__STRIPE__WEBHOOK_SECRET" => render_optional_secret(
            "COTERIE__STRIPE__WEBHOOK_SECRET",
            config.stripe_webhook_secret.as_ref(),
            config.stripe_enabled,
        ),
        "COTERIE__INTEGRATIONS__DISCORD__ENABLED" => Some(format!(
            "COTERIE__INTEGRATIONS__DISCORD__ENABLED={}",
            config.discord_enabled
        )),
        "COTERIE__INTEGRATIONS__DISCORD__BOT_TOKEN" => render_optional_secret(
            "COTERIE__INTEGRATIONS__DISCORD__BOT_TOKEN",
            config.discord_bot_token.as_ref(),
            config.discord_enabled,
        ),
        "COTERIE__INTEGRATIONS__DISCORD__GUILD_ID" => render_optional(
            "COTERIE__INTEGRATIONS__DISCORD__GUILD_ID",
            config.discord_guild_id.as_deref(),
            config.discord_enabled,
        ),
        "COTERIE__INTEGRATIONS__DISCORD__MEMBER_ROLE_ID" => render_optional(
            "COTERIE__INTEGRATIONS__DISCORD__MEMBER_ROLE_ID",
            config.discord_member_role_id.as_deref(),
            config.discord_enabled,
        ),
        "COTERIE__INTEGRATIONS__DISCORD__EXPIRED_ROLE_ID" => render_optional(
            "COTERIE__INTEGRATIONS__DISCORD__EXPIRED_ROLE_ID",
            config.discord_expired_role_id.as_deref(),
            config.discord_enabled,
        ),
        "COTERIE__INTEGRATIONS__UNIFI__ENABLED" => Some(format!(
            "COTERIE__INTEGRATIONS__UNIFI__ENABLED={}",
            config.unifi_enabled
        )),
        "COTERIE__INTEGRATIONS__UNIFI__CONTROLLER_URL" => render_optional(
            "COTERIE__INTEGRATIONS__UNIFI__CONTROLLER_URL",
            config.unifi_controller_url.as_deref(),
            config.unifi_enabled,
        ),
        "COTERIE__INTEGRATIONS__UNIFI__USERNAME" => render_optional(
            "COTERIE__INTEGRATIONS__UNIFI__USERNAME",
            config.unifi_username.as_deref(),
            config.unifi_enabled,
        ),
        "COTERIE__INTEGRATIONS__UNIFI__PASSWORD" => render_optional_secret(
            "COTERIE__INTEGRATIONS__UNIFI__PASSWORD",
            config.unifi_password.as_ref(),
            config.unifi_enabled,
        ),
        "COTERIE__INTEGRATIONS__UNIFI__SITE_ID" => render_optional(
            "COTERIE__INTEGRATIONS__UNIFI__SITE_ID",
            config.unifi_site_id.as_deref(),
            config.unifi_enabled,
        ),
        _ => None,
    }
}

fn render_optional(key: &str, value: Option<&str>, enabled: bool) -> Option<String> {
    if enabled {
        if let Some(v) = value {
            return Some(format!("{key}={v}"));
        }
        return Some(format!("# {key}="));
    }
    Some(format!("# {key}="))
}

fn render_optional_secret(
    key: &str,
    value: Option<&Secret<String>>,
    enabled: bool,
) -> Option<String> {
    if enabled {
        if let Some(v) = value {
            return Some(format!("{key}={}", v.expose_secret()));
        }
        return Some(format!("# {key}="));
    }
    Some(format!("# {key}="))
}

/// Generate a fresh 64-hex-char session secret.
pub fn generate_session_secret() -> Secret<String> {
    use rand::RngCore;
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    Secret::new(hex::encode(buf))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn base_config() -> EnvConfig {
        EnvConfig {
            server_host: "127.0.0.1".into(),
            server_port: 8080,
            base_url: "https://coterie.example.org".into(),
            data_dir: None,
            database_url: "sqlite://coterie.db".into(),
            database_max_connections: 10,
            session_secret: Secret::new("a".repeat(64)),
            session_duration_hours: 24,
            totp_issuer: "Coterie".into(),
            ..Default::default()
        }
    }

    #[test]
    fn replaces_required_fields() {
        let template = include_str!("../tests/fixtures/env_example.txt");
        let cfg = base_config();
        let rendered = render_env(template, &cfg);
        assert!(rendered.contains("COTERIE__SERVER__BASE_URL=https://coterie.example.org"));
        assert!(rendered.contains("COTERIE__AUTH__SESSION_SECRET="));
        assert!(!rendered.contains("replace-with-a-long-random-string"));
        assert!(rendered.contains("COTERIE__STRIPE__ENABLED=false"));
    }

    #[test]
    fn live_with_all_integrations() {
        let template = include_str!("../tests/fixtures/env_example.txt");
        let cfg = EnvConfig {
            stripe_enabled: true,
            stripe_publishable_key: Some("pk_live_abc".into()),
            stripe_secret_key: Some(Secret::new("sk_live_def".into())),
            stripe_webhook_secret: Some(Secret::new("whsec_xyz".into())),
            discord_enabled: true,
            discord_bot_token: Some(Secret::new("bot.token".into())),
            discord_guild_id: Some("1234".into()),
            discord_member_role_id: Some("5678".into()),
            discord_expired_role_id: Some("9012".into()),
            discord_announce_channel_id: Some("3456".into()),
            unifi_enabled: true,
            unifi_controller_url: Some("https://unifi.example.com:8443".into()),
            unifi_username: Some("admin".into()),
            unifi_password: Some(Secret::new("p@ss".into())),
            unifi_site_id: Some("default".into()),
            ..base_config()
        };
        let rendered = render_env(template, &cfg);
        assert!(rendered.contains("COTERIE__STRIPE__ENABLED=true"));
        assert!(rendered.contains("COTERIE__STRIPE__PUBLISHABLE_KEY=pk_live_abc"));
        assert!(rendered.contains("COTERIE__STRIPE__SECRET_KEY=sk_live_def"));
        assert!(rendered.contains("COTERIE__STRIPE__WEBHOOK_SECRET=whsec_xyz"));
        assert!(rendered.contains("COTERIE__INTEGRATIONS__DISCORD__ENABLED=true"));
        assert!(rendered.contains("COTERIE__INTEGRATIONS__DISCORD__BOT_TOKEN=bot.token"));
        assert!(rendered.contains("COTERIE__INTEGRATIONS__UNIFI__ENABLED=true"));
        assert!(rendered.contains(
            "COTERIE__INTEGRATIONS__UNIFI__CONTROLLER_URL=https://unifi.example.com:8443"
        ));
    }

    #[test]
    fn no_optional_integrations_leaves_them_commented() {
        let template = include_str!("../tests/fixtures/env_example.txt");
        let rendered = render_env(template, &base_config());
        // Disabled integrations: the ENABLED line is false, the
        // credential lines stay commented out with empty values.
        assert!(rendered.contains("COTERIE__STRIPE__ENABLED=false"));
        assert!(rendered.contains("# COTERIE__STRIPE__PUBLISHABLE_KEY="));
        assert!(rendered.contains("# COTERIE__INTEGRATIONS__DISCORD__BOT_TOKEN="));
        assert!(rendered.contains("# COTERIE__INTEGRATIONS__UNIFI__CONTROLLER_URL="));
    }

    #[test]
    fn discord_only_permutation() {
        let template = include_str!("../tests/fixtures/env_example.txt");
        let cfg = EnvConfig {
            discord_enabled: true,
            discord_bot_token: Some(Secret::new("bot".into())),
            discord_guild_id: Some("g".into()),
            discord_member_role_id: Some("m".into()),
            discord_expired_role_id: Some("e".into()),
            ..base_config()
        };
        let rendered = render_env(template, &cfg);
        assert!(rendered.contains("COTERIE__STRIPE__ENABLED=false"));
        assert!(rendered.contains("COTERIE__INTEGRATIONS__DISCORD__ENABLED=true"));
        assert!(rendered.contains("COTERIE__INTEGRATIONS__DISCORD__BOT_TOKEN=bot"));
        assert!(rendered.contains("COTERIE__INTEGRATIONS__UNIFI__ENABLED=false"));
    }

    #[test]
    fn session_secret_is_hex_64() {
        let s = generate_session_secret();
        let raw = s.expose_secret();
        assert_eq!(raw.len(), 64);
        assert!(raw.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
