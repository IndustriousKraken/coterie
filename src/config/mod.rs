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
    #[serde(default)]
    pub seed: SeedConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub base_url: String,
    #[serde(default = "default_uploads_dir")]
    pub uploads_dir: String,
}

fn default_uploads_dir() -> String {
    "uploads".to_string()
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

#[derive(Debug, Deserialize, Clone)]
pub struct SeedConfig {
    pub admin: AdminSeedConfig,
    pub test_users: Vec<TestUserConfig>,
    pub membership_types: Vec<MembershipTypeSeedConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AdminSeedConfig {
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub password: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TestUserConfig {
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub password: String,
    pub membership_type: String,
    pub status: String,
    pub months_active: i64,
    pub bypass_dues: bool,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MembershipTypeSeedConfig {
    pub name: String,
    pub slug: String,
    pub color: String,
    pub fee_cents: i64,
    pub billing_frequency: String, // "monthly" or "yearly"
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
            .add_source(File::with_name("config/seed").required(false))
            
            // Add environment variables (with COTERIE__ prefix, double underscore separates levels)
            .add_source(Environment::with_prefix("COTERIE").separator("__"))
            
            .build()?;

        config.try_deserialize()
    }
}

impl Default for SeedConfig {
    fn default() -> Self {
        Self {
            admin: AdminSeedConfig {
                email: "admin@coterie.local".to_string(),
                username: "admin".to_string(),
                full_name: "Admin User".to_string(),
                password: "admin123".to_string(),
            },
            test_users: vec![
                TestUserConfig {
                    email: "alice@example.com".to_string(),
                    username: "alice".to_string(),
                    full_name: "Alice Johnson".to_string(),
                    password: "password123".to_string(),
                    membership_type: "regular".to_string(),
                    status: "active".to_string(),
                    months_active: 18,
                    bypass_dues: false,
                    notes: None,
                },
                TestUserConfig {
                    email: "bob@example.com".to_string(),
                    username: "bob".to_string(),
                    full_name: "Bob Smith".to_string(),
                    password: "password123".to_string(),
                    membership_type: "student".to_string(),
                    status: "active".to_string(),
                    months_active: 12,
                    bypass_dues: false,
                    notes: None,
                },
                TestUserConfig {
                    email: "charlie@example.com".to_string(),
                    username: "charlie".to_string(),
                    full_name: "Charlie Brown".to_string(),
                    password: "password123".to_string(),
                    membership_type: "regular".to_string(),
                    status: "expired".to_string(),
                    months_active: 8,
                    bypass_dues: false,
                    notes: None,
                },
                TestUserConfig {
                    email: "dave@example.com".to_string(),
                    username: "dave".to_string(),
                    full_name: "Dave Wilson".to_string(),
                    password: "password123".to_string(),
                    membership_type: "regular".to_string(),
                    status: "pending".to_string(),
                    months_active: 0,
                    bypass_dues: false,
                    notes: None,
                },
            ],
            membership_types: vec![
                MembershipTypeSeedConfig {
                    name: "Regular".to_string(),
                    slug: "regular".to_string(),
                    color: "#2196F3".to_string(),
                    fee_cents: 5000, // $50
                    billing_frequency: "yearly".to_string(),
                },
                MembershipTypeSeedConfig {
                    name: "Student".to_string(),
                    slug: "student".to_string(),
                    color: "#4CAF50".to_string(),
                    fee_cents: 2500, // $25
                    billing_frequency: "yearly".to_string(),
                },
                MembershipTypeSeedConfig {
                    name: "Corporate".to_string(),
                    slug: "corporate".to_string(),
                    color: "#9C27B0".to_string(),
                    fee_cents: 50000, // $500
                    billing_frequency: "yearly".to_string(),
                },
                MembershipTypeSeedConfig {
                    name: "Lifetime".to_string(),
                    slug: "lifetime".to_string(),
                    color: "#FF9800".to_string(),
                    fee_cents: 50000, // $500 one-time
                    billing_frequency: "yearly".to_string(),
                },
            ],
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 8080,
                base_url: "http://localhost:8080".to_string(),
                uploads_dir: "uploads".to_string(),
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
            seed: SeedConfig::default(),
        }
    }
}