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
    /// Base directory for all persistent data (database, uploads, etc.)
    /// Defaults to "./data". In containers, mount a volume here.
    #[serde(default = "default_data_dir")]
    pub data_dir: String,
    /// Directory for uploaded files. Defaults to "{data_dir}/uploads"
    pub uploads_dir: Option<String>,
    /// Force the Secure flag on session cookies. When None, inferred from
    /// base_url: https:// → true, anything else → false. Override to `true`
    /// when Coterie sits behind a TLS-terminating reverse proxy that
    /// presents itself as http:// internally.
    pub secure_cookies: Option<bool>,
    /// Allowed CORS origins for the public API (comma-separated).
    /// Example: "https://yoursite.com,https://www.yoursite.com"
    /// If empty or omitted, only same-origin requests are allowed.
    #[serde(default)]
    pub cors_origins: Option<String>,
    /// Whether to trust X-Forwarded-For / X-Real-Ip headers for client IP
    /// detection. When unset, defaults to whatever cookies_are_secure()
    /// returns — i.e. trust the headers when deployed over TLS (assumed
    /// to be behind a reverse proxy) but not during local HTTP dev.
    /// Set to false explicitly if this server faces untrusted clients
    /// directly, to prevent IP spoofing.
    pub trust_forwarded_for: Option<bool>,
}

impl ServerConfig {
    /// Get the uploads directory, defaulting to {data_dir}/uploads
    pub fn uploads_path(&self) -> String {
        self.uploads_dir.clone().unwrap_or_else(|| {
            format!("{}/uploads", self.data_dir)
        })
    }

    pub fn cookies_are_secure(&self) -> bool {
        self.secure_cookies
            .unwrap_or_else(|| self.base_url.starts_with("https://"))
    }

    /// Whether to trust X-Forwarded-For / X-Real-Ip headers for client IP
    /// detection. Defaults to the same logic as `cookies_are_secure` —
    /// a TLS-terminated deployment is assumed to be behind a trusted proxy.
    pub fn trust_forwarded_for(&self) -> bool {
        self.trust_forwarded_for
            .unwrap_or_else(|| self.cookies_are_secure())
    }
}

fn default_data_dir() -> String {
    // Check locations in order of preference:
    // 1. /var/lib/coterie - standard Linux service location (if it exists)
    // 2. /data - container convention (if in container)
    // 3. ./data - local development fallback

    let var_lib = std::path::Path::new("/var/lib/coterie");
    if var_lib.exists() {
        return "/var/lib/coterie".to_string();
    }

    if std::path::Path::new("/.dockerenv").exists() || std::env::var("CONTAINER").is_ok() {
        return "/data".to_string();
    }

    "./data".to_string()
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
    pub publishable_key: Option<String>,
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
            // Add config files if they exist (in order of precedence, later overrides earlier)
            .add_source(File::with_name("/etc/coterie/config").required(false))  // System-wide
            .add_source(File::with_name("config/default").required(false))        // App default
            .add_source(File::with_name("config/local").required(false))          // Local overrides
            .add_source(File::with_name("config/seed").required(false))           // Seed data

            // Add environment variables (with COTERIE__ prefix, double underscore separates levels)
            .add_source(Environment::with_prefix("COTERIE").separator("__"))

            .build()?;

        config.try_deserialize()
    }

    /// Get the database URL, resolving relative paths against data_dir
    pub fn database_url(&self) -> String {
        let url = &self.database.url;
        // If it's a simple sqlite filename (not a full path), put it in data_dir
        // Handle both "sqlite://filename" and "sqlite:filename" forms
        let filename = url.strip_prefix("sqlite://")
            .or_else(|| url.strip_prefix("sqlite:"));
        if let Some(filename) = filename {
            if !filename.contains('/') {
                return format!("sqlite://{}/{}", self.server.data_dir, filename);
            }
        }
        url.clone()
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
