//! Shared logic for the `create_admin` bootstrap binary.
//!
//! Lives in the library crate (rather than directly in `src/bin/`) so
//! integration tests can drive `run_with_pool` directly against an
//! in-memory SQLite pool. The binary in `src/bin/create_admin.rs` is a
//! thin shell that parses args, builds the pool, runs migrations, and
//! delegates here.

use std::path::PathBuf;
use std::process;

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::auth::AuthService;

/// Bootstrap the first admin for a Coterie deployment.
#[derive(Parser, Debug)]
#[command(name = "create_admin", author, version, about, long_about = None)]
pub struct Cli {
    /// Admin email address (must be unique across all members).
    #[arg(long)]
    pub email: String,

    /// Login username (must be unique).
    #[arg(long)]
    pub username: String,

    /// Display name shown in the UI and emails.
    #[arg(long, value_name = "FULL NAME")]
    pub full_name: String,

    #[command(flatten)]
    pub password: PasswordSource,
}

/// Exactly one of these is required. `--password` is convenient for
/// testing but appears in `ps aux`; production automation should use
/// `--password-file` with chmod 0600 and shred-after-use.
#[derive(clap::Args, Debug)]
#[group(required = true, multiple = false)]
pub struct PasswordSource {
    /// Password as a literal CLI arg. Visible in process listings.
    #[arg(long)]
    pub password: Option<String>,

    /// Path to a file containing the password. Trailing whitespace is
    /// trimmed (so a trailing newline from `echo` is fine).
    #[arg(long)]
    pub password_file: Option<PathBuf>,
}

impl PasswordSource {
    /// Resolve to the actual password string. For `--password-file`,
    /// trailing whitespace (including a trailing newline) is stripped
    /// so heredocs and `echo`-piped files behave intuitively.
    pub fn resolve(&self) -> Result<String> {
        if let Some(p) = &self.password {
            return Ok(p.clone());
        }
        let path = self
            .password_file
            .as_ref()
            .ok_or_else(|| anyhow!("no password source supplied"))?;
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("reading password file {}", path.display()))?;
        Ok(contents
            .trim_end_matches(|c: char| c.is_whitespace())
            .to_string())
    }
}

/// Outcome of `run_with_pool` when an admin already exists. The binary
/// translates this into `process::exit(2)`; tests assert against the
/// variant directly.
#[derive(Debug)]
pub enum CreateAdminOutcome {
    Created(Uuid),
    AlreadyExists,
}

/// The binary's core operation, parameterised over the pool so tests
/// can drive it against an in-memory SQLite instance.
///
/// Returns `Created(id)` on success, `AlreadyExists` if the idempotency
/// check tripped. The binary wrapper turns `AlreadyExists` into
/// `process::exit(2)` with a stderr message.
pub async fn run_with_pool(cli: &Cli, pool: &SqlitePool) -> Result<CreateAdminOutcome> {
    if admin_exists(pool).await? {
        return Ok(CreateAdminOutcome::AlreadyExists);
    }

    let password = cli.password.resolve()?;
    if password.is_empty() {
        return Err(anyhow!("password is empty after trimming"));
    }

    // Same Argon2 parameters as the manual /setup form — `AuthService::
    // hash_password` is the single source of truth so a CLI-bootstrapped
    // admin's password is interchangeable with a wizard-bootstrapped
    // one (including future rehashing on param bumps).
    let password_hash = AuthService::hash_password(&password)
        .await
        .map_err(|e| anyhow!("hashing password: {}", e))?;

    // Pick the first active membership type, matching the public-signup
    // default. Migration 001 seeds defaults; orgs that ran create_admin
    // before configuring types will pick whichever migration-seeded row
    // sorts first.
    let mt_row: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM membership_types \
         WHERE is_active = 1 \
         ORDER BY sort_order ASC, name ASC \
         LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .context("resolving default membership_type")?;
    let membership_type_id = match mt_row {
        Some((id_str,)) => id_str,
        None => {
            return Err(anyhow!(
                "no active membership_types in database — \
                 migrations may not have run cleanly"
            ));
        }
    };

    let id = Uuid::new_v4();
    let id_str = id.to_string();
    let now = chrono::Utc::now().naive_utc();

    sqlx::query(
        r#"
        INSERT INTO members (
            id, email, username, full_name, password_hash,
            status, membership_type_id, joined_at, bypass_dues,
            email_verified_at, is_admin, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, 'Active', ?, ?, 1, ?, 1, ?, ?)
        "#,
    )
    .bind(&id_str)
    .bind(&cli.email)
    .bind(&cli.username)
    .bind(&cli.full_name)
    .bind(&password_hash)
    .bind(&membership_type_id)
    .bind(now)
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .context("inserting admin row")?;

    Ok(CreateAdminOutcome::Created(id))
}

/// Convenience wrapper for the binary: maps `AlreadyExists` to a
/// process exit-2 with a clear stderr message, prints a success banner
/// on `Created`. Tests should call `run_with_pool` directly.
pub async fn dispatch(cli: &Cli, pool: &SqlitePool) -> Result<()> {
    match run_with_pool(cli, pool).await? {
        CreateAdminOutcome::Created(id) => {
            println!("Admin {} created (id {}).", cli.email, id);
            Ok(())
        }
        CreateAdminOutcome::AlreadyExists => {
            eprintln!(
                "Admin already exists; refusing to create another via CLI. \
                 Use the portal admin UI instead."
            );
            process::exit(2);
        }
    }
}

/// `SELECT EXISTS(...)` against the authoritative `is_admin` column,
/// matching the `require_setup` middleware's check.
async fn admin_exists(pool: &SqlitePool) -> Result<bool> {
    let row: (i64,) = sqlx::query_as(
        "SELECT EXISTS(SELECT 1 FROM members WHERE is_admin = 1)",
    )
    .fetch_one(pool)
    .await
    .context("checking for existing admin")?;
    Ok(row.0 != 0)
}
