//! `create_admin` — CLI tool to bootstrap the first admin without HTTP.
//!
//! Closes the security window in which an unauthenticated `/setup` page
//! is reachable on a freshly-deployed Coterie instance. Provisioning
//! automation calls this binary BEFORE starting the server, so by the
//! time the HTTP listener binds, an admin already exists and the setup
//! redirect is inert.
//!
//! Refuses (exit code 2) if any admin already exists — the binary is
//! bootstrap-only; subsequent admin creation goes through the portal UI.

use std::process;
use std::str::FromStr;

use anyhow::{Context, Result};
use clap::Parser;
use coterie::{
    admin_cli::{dispatch, Cli},
    config::Settings,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

#[tokio::main]
async fn main() {
    // Load .env if present so COTERIE__DATABASE__URL etc. work the same
    // way they do for the server binary.
    dotenvy::dotenv().ok();

    let cli = Cli::parse();
    if let Err(e) = run(cli).await {
        eprintln!("create_admin: {}", e);
        for cause in e.chain().skip(1) {
            eprintln!("  caused by: {}", cause);
        }
        process::exit(1);
    }
}

/// Build the pool, run migrations, hand off to the shared dispatcher.
async fn run(cli: Cli) -> Result<()> {
    let settings = Settings::new().context("loading configuration (Settings::new)")?;
    let pool = open_pool(&settings).await?;
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("running migrations")?;
    dispatch(&cli, &pool).await
}

/// Open the SQLite pool with `create_if_missing(true)` so a totally
/// fresh deployment can run this binary as its very first DB operation.
async fn open_pool(settings: &Settings) -> Result<SqlitePool> {
    let url = settings.database_url();
    let opts = SqliteConnectOptions::from_str(&url)
        .with_context(|| format!("parsing database URL {}", url))?
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(settings.database.max_connections.max(1))
        .connect_with(opts)
        .await
        .with_context(|| format!("opening SQLite pool at {}", url))?;
    Ok(pool)
}
