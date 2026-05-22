## 1. Add the binary

- [ ] 1.1 Add a `[[bin]]` section to `Cargo.toml` after the existing `seed` binary block:
  ```toml
  [[bin]]
  name = "create_admin"
  path = "src/bin/create_admin.rs"
  ```

## 2. Implement create_admin.rs

- [ ] 2.1 Create `src/bin/create_admin.rs`. Use clap (`use clap::Parser;` — same as `seed.rs`) for arg parsing.
- [ ] 2.2 Define the `Cli` struct with the four args. `--password` and `--password-file` SHALL be a mutually-exclusive group:
  ```rust
  #[derive(Parser)]
  struct Cli {
      #[arg(long)] email: String,
      #[arg(long)] username: String,
      #[arg(long, value_name = "FULL NAME")] full_name: String,
      #[command(flatten)] password: PasswordSource,
  }

  #[derive(clap::Args)]
  #[group(required = true, multiple = false)]
  struct PasswordSource {
      #[arg(long)] password: Option<String>,
      #[arg(long)] password_file: Option<PathBuf>,
  }
  ```
- [ ] 2.3 In `main`: load `Settings::new()`, build the SQLite pool with `SqliteConnectOptions::from_str(&settings.database.url)?.create_if_missing(true)`, run `sqlx::migrate!("./migrations").run(&pool).await?`.
- [ ] 2.4 Implement the idempotency check: `SELECT EXISTS(SELECT 1 FROM members WHERE is_admin = 1)`. If true, print to stderr and `process::exit(2)`.
- [ ] 2.5 Resolve the password — read the file contents and trim if `--password-file` was supplied, otherwise use the `--password` arg directly.
- [ ] 2.6 Hash the password. If `AuthService::hash_password` is private, expose a `pub fn hash_password(plain: &str) -> Result<String>` free function in `src/auth/mod.rs` (or wherever the hashing lives) — same argon2 parameters as the existing setup-form path.
- [ ] 2.7 Insert the admin row:
  ```sql
  INSERT INTO members (id, email, username, full_name, status, membership_type_id,
                       joined_at, email_verified_at, password_hash, is_admin,
                       created_at, updated_at)
  VALUES (?, ?, ?, ?, 'Active', ?, NOW, NOW, ?, 1, NOW, NOW)
  ```
  - Use `Uuid::new_v4()` for the id.
  - For `membership_type_id`, query `SELECT id FROM membership_types WHERE is_active = 1 ORDER BY sort_order LIMIT 1` (matches the public-signup default).
  - The password hash goes in `password_hash`.
- [ ] 2.8 Print success message: `"Admin {email} created (id {uuid})."` and exit 0.
- [ ] 2.9 Make sure error paths use `anyhow::Result` and helpful messages — the wizard will surface stderr to the operator.

## 3. Tighten /setup GET

- [ ] 3.1 In `src/web/templates/setup.rs`'s setup-page GET handler, check `state.admin_exists_observed.load(Ordering::Relaxed)`. If true, return `Redirect::to("/login").into_response()` instead of rendering the form.
- [ ] 3.2 If `admin_exists_observed` is false, fall through to the existing query: `check_admin_exists(&state).await`. If true, set the flag (using `store(true, Ordering::Relaxed)` for consistency with the middleware) and redirect. If false, render the form.
- [ ] 3.3 The POST handler is unchanged — its existing inside-the-lock `check_admin_exists` is the authoritative refusal point.

## 4. Tests

- [ ] 4.1 Add `tests/create_admin_test.rs` (new integration test file). Pattern: spin up an in-memory SQLite, run migrations, invoke the binary's `run` function (extracted as `pub async fn run(...)` from `main` so it's testable).
- [ ] 4.2 Test: `happy_path_creates_admin` — fresh DB, run create_admin with valid args, assert the `members` row exists with the right fields, `is_admin=1`, `status='Active'`, password verifies.
- [ ] 4.3 Test: `refuses_when_admin_exists` — pre-insert an admin, run create_admin, assert the binary's `run` returns an error (or exit code 2 surrogate) and no second insert happens.
- [ ] 4.4 Test: `password_file_strips_trailing_newline` — write `"secret123\n"` to a temp file, run with `--password-file`, assert the resulting hash verifies against `"secret123"` (not `"secret123\n"`).
- [ ] 4.5 Test: `setup_get_redirects_when_admin_exists` — boot a test app via the router harness, pre-create an admin, GET /setup, assert 303 to /login.

## 5. Validate

- [ ] 5.1 `cargo build --bins --release` — confirm both `coterie`, `seed`, AND `create_admin` build cleanly.
- [ ] 5.2 `cargo test --features test-utils` — full suite passes.
- [ ] 5.3 Manual smoke: build the binary, run `./target/release/create_admin --help` and confirm the help text reads cleanly.
