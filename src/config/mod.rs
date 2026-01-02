use serde::Deserialize;
use config::{Config, ConfigError, Environment, File};

#[derive(Debug, Deserialize, Clone)]
pub struct Settings {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub auth: AuthConfig,
    #[serde(default)]
    pub stripe: StripeConfig,
    #[serde(default)]
    pub integrations: IntegrationConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub base_url: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AuthConfig {
    pub session_secret: String,
    pub session_duration_hours: i64,
    pub totp_issuer: String,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct StripeConfig {
    pub secret_key: Option<String>,
    pub webhook_secret: Option<String>,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct IntegrationConfig {
    pub discord: Option<DiscordConfig>,
    pub unifi: Option<UnifiConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DiscordConfig {
    pub enabled: bool,
    pub bot_token: String,
    pub guild_id: String,
    pub member_role_id: String,
    pub expired_role_id: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct UnifiConfig {
    pub enabled: bool,
    pub controller_url: String,
    pub username: String,
    pub password: String,
    pub site_id: String,
}

impl Settings {
    pub fn new() -> Result<Self, ConfigError> {
        let config = Config::builder()
            // Start with default values
            .set_default("server.host", "127.0.0.1")?
            .set_default("server.port", 8080)?
            .set_default("database.max_connections", 10)?
            .set_default("auth.session_duration_hours", 24)?
            .set_default("stripe.enabled", false)?
            
            // Add config file if it exists
            .add_source(File::with_name("config/default").required(false))
            .add_source(File::with_name("config/local").required(false))
            
            // Add environment variables (with COTERIE__ prefix, double underscore separates levels)
            .add_source(Environment::with_prefix("COTERIE").separator("__"))
            
            .build()?;

        config.try_deserialize()
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 8080,
                base_url: "http://localhost:8080".to_string(),
            },
            database: DatabaseConfig {
                url: "sqlite://coterie.db".to_string(),
                max_connections: 10,
            },
            auth: AuthConfig {
                session_secret: "change-me-in-production".to_string(),
                session_duration_hours: 24,
                totp_issuer: "Coterie".to_string(),
            },
            stripe: StripeConfig {
                secret_key: None,
                webhook_secret: None,
                enabled: false,
            },
            integrations: IntegrationConfig {
                discord: None,
                unifi: None,
            },
        }
    }
}