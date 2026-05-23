## Context

After this session's deploy walkthrough, the substrate for an automated wizard is mostly in place:

- `release-deploy.sh` — fetches a tagged GitHub Release, places files, runs install.sh if first install, restarts service on update
- `install.sh` — creates the `coterie` user, dirs, systemd unit (idempotent)
- `uninstall.sh` — recovery path with `--data` and `--all` modes
- `create_admin` (separate spec `a23`, prerequisite) — CLI for the first admin
- `Caddyfile.example` — template with the right structure, just needs domain + log-dir setup
- `STRIPE-SETUP.md` — operator-facing reference for the secrets the wizard collects

The wizard is the conductor: it asks the operator the org-specific questions, then orchestrates all of the above in the right order.

The first iteration of this spec specified pure bash. That was rejected after one autocoder run because the validation tasks (shellcheck, fresh-VM smoke) aren't available in the sandbox — the autocoder honestly reported "did manual review and skipped the VM smoke" which is exactly the gap we wanted to close. Switching to a Rust binary with a thin bash bootstrap solves that: the wizard logic is `cargo test`-able in the same sandbox the rest of Coterie's CI uses, and the bash bootstrap is small enough (~50 lines) that manual review is the right tool for it anyway.

## Goals / Non-Goals

**Goals:**
- A single `curl -o /tmp/provision.sh && bash /tmp/provision.sh` brings a fresh Debian 13 box to "Coterie running, admin created, TLS provisioned (if Caddy chosen), .env populated, ready for traffic."
- Interactive prompts for org-specific values; non-interactive mode via env vars and/or `--flag` args for IaC.
- Idempotent — re-running detects state and prompts about overwrites.
- `--dry-run` (or `--plan`) flag prints the planned actions without executing.
- Wizard logic fully testable in the autocoder sandbox via `cargo test`. No "we'll know if it works when an operator runs it" gaps in the validation tasks.
- Useful enough that the README and deploy docs recommend it as the primary path.

**Non-Goals:**
- Provisioning the droplet/VM itself. The operator provisions the box (DO, AWS, bare metal); the wizard runs on it.
- DNS configuration. The operator points DNS at the box; the wizard waits for it (TLS provisioning will retry naturally).
- Volume mounting. If the operator has a separate block volume, they mount it at `/var/lib/coterie` before running the wizard. The wizard detects whether that mountpoint exists and warns if not.
- Multi-host setups. The wizard targets single-VM deploys (which is what Coterie supports).
- Updates after first deploy. `release-deploy.sh` handles updates; the wizard is bootstrap-only.
- Distros other than Debian 13. The wizard assumes `apt`, `systemd`, paths like `/etc/caddy/`, etc. An Alpine-flavored variant is a separate concern.

## Decisions

### D1. Rust binary + thin bash bootstrap

The wizard is a Rust binary `coterie-provision`, shipped in the release tarball alongside `coterie` and `create_admin`. Bash is reserved for the ~50-line bootstrap script that downloads the binary the first time (chicken-and-egg).

Why not pure bash:
- The autocoder sandbox doesn't have `shellcheck`, can't spin up VMs, and can't smoke-test interactive prompts. Specifying "validate via shellcheck + VM smoke" produces validation tasks the autocoder skips and self-reports about. Specifying "validate via `cargo test`" produces validation the sandbox can actually run.
- Type-safe state machine for the install flow. `enum InstallState { GatherInputs, InstallPackages, PullRelease, ... }` with a `step(state) -> Result<NextState>` function is hard to express cleanly in bash.
- Better error messages via `anyhow::Context`.
- Trait abstractions over OS effects (`trait SystemCommand`, `trait FileSystem`) let integration code be tested with fakes in the sandbox and real in prod.
- A nicer TUI via `inquire`/`dialoguer` than `read -p`.
- Easier to extend later (Alpine, Ubuntu) without rewriting bash.

Why bash for the bootstrap:
- Operator has nothing on the box yet. Bash + curl is the universal "first contact" surface.
- ~50 lines is short enough to audit by reading once.
- Keeps the "one command, no exotic deps" UX intact.

The bootstrap's responsibilities (and nothing else):
1. `set -euo pipefail` + `ERR` trap.
2. Refuse if not root.
3. Refuse if not Debian.
4. Hit `https://api.github.com/repos/IndustriousKraken/coterie/releases/latest` to get the latest stable tag (or accept `--tag <v...>` to override).
5. `curl -fL` the `coterie-provision-<tag>-x86_64-unknown-linux-musl.tar.gz` asset to `/tmp/`.
6. `curl -fL` the matching `.sha256` asset.
7. `sha256sum -c` to verify.
8. `tar -xzf` to extract `coterie-provision`.
9. `exec /tmp/coterie-provision install "$@"` (pass through all flags).

If any step fails, exit non-zero with a clear message. There's no recovery logic in bash — that's all in Rust.

### D2. Error handling: anyhow + structured context

Rust binary uses `anyhow::Result<T>` throughout, with `.context("…")` annotations at every state transition. On failure, the operator sees a clear chain like:

```
Error: failed to configure Caddy

Caused by:
    0: caddy validate returned non-zero
    1: invalid configuration at line 12
```

A top-level `eprintln!` + `std::process::exit(1)` block prints the full chain. The bootstrap script wraps this with its own ERR trap so a panic from Rust (shouldn't happen, but) is still surfaced clearly.

### D3. Interactive + non-interactive modes share the same code path

Every prompt is a function that takes `(env_var_name, cli_flag_value, prompt_message, default)` and:
1. Returns `cli_flag_value` if it's `Some(_)`.
2. Returns the env var value if it's set.
3. Otherwise, calls the interactive prompt (inquire / dialoguer).

This makes IaC-friendly automation natural — every required input is settable via env or flag. The binary enumerates all of them in `--help` output.

Example invocation:

```sh
COTERIE_PROVISION_ORG_NAME="Neon Temple" \
COTERIE_PROVISION_PORTAL_DOMAIN="coterie.theneontemple.com" \
COTERIE_PROVISION_ADMIN_EMAIL="rab@theneontemple.com" \
... \
coterie-provision install --no-prompt
```

`--no-prompt` is an explicit flag that turns interactive mode off; if any required input is missing in that mode, the binary errors out listing what's needed.

### D3a. Version selection — default to latest stable, allow rollback

Before any install steps, the wizard fetches `/repos/IndustriousKraken/coterie/releases?per_page=10` from the GitHub API (using `reqwest` blocking, with a `User-Agent` header per GitHub's requirement). From the response, it builds two lists:

- **Stable releases** — those with `prerelease: false`. Sorted newest-first.
- **All releases** — sorted newest-first (includes prereleases marked as dev/alpha/beta/rc).

Default: the newest stable release's tag (e.g. `v1.0.0`). The prompt presents the top ~5 stable releases as a numbered menu, plus an option to "see all releases including prereleases" for operators who specifically want a dev version.

Env-var / flag equivalent: `COTERIE_PROVISION_VERSION=v1.0.0` or `--version v1.0.0`. If set, the wizard uses it verbatim with no prompt.

Once selected, the tag is passed to `release-deploy.sh <TAG>` for the actual install of `coterie` and `create_admin` binaries.

This pairs with the workflow change (proposal): tags ending in non-numeric suffixes are published as prereleases, so the GitHub API correctly classifies them. Without the workflow change, every dev tag would show up in the "stable" list.

### D4. Password handling

The admin password is:
1. Prompted via `inquire::Password::new()` with `WithConfirmation` (no echo, asks twice for verification).
2. Wrapped in `secrecy::Secret<String>` to prevent accidental logging or debug-print.
3. Written via `tempfile::NamedTempFile` with mode 0600 (set on the file via `std::os::unix::fs::PermissionsExt`).
4. Passed to `create_admin --password-file <path>`.
5. The `NamedTempFile`'s `Drop` impl removes the file; an explicit shred-then-drop guard (writing zeros over the file before unlink) handles the paranoid case.

The password never appears in:
- The process listing (would happen if we passed `--password "..."` to create_admin)
- The shell history (it's never typed in a shell context)
- Any log file or error message (`secrecy::Secret<String>`'s `Debug` impl prints `[REDACTED]`)
- The .env file (it's not an env var; it's hashed into the DB by create_admin)

For non-interactive mode, the operator provides `COTERIE_PROVISION_ADMIN_PASSWORD`. Documented warning: env vars are visible to other users via `/proc/<pid>/environ` if your script is readable. Acceptable for CI where the box is single-user.

### D5. .env generation

The wizard reads `/opt/coterie/.env.example` (placed there by `release-deploy.sh`) and produces `.env` by substituting values for known keys via a pure function:

```rust
fn render_env(template: &str, config: &Config) -> String
```

Where `Config` is a struct of all collected inputs. Golden tests cover: every integration enabled, no integrations enabled, mixed states, edge cases (org name with special chars, etc.). The function is also `pub` so tests can call it directly.

This keeps the wizard's .env shape in lock-step with whatever release version the operator is installing — `.env.example` evolves with Coterie, the wizard adapts automatically.

### D6. Session secret generation

`COTERIE__AUTH__SESSION_SECRET = hex::encode(rand::thread_rng().gen::<[u8; 32]>())` — a 64-character hex blob. Generated once at provision time, embedded in `.env`. The operator never sees it; it just lives in `.env` until rotated.

Same pattern for any other generated secret (currently only the session secret).

### D7. Caddyfile generation

The wizard reads `/opt/coterie/deploy/Caddyfile.example` and substitutes via a pure function:

```rust
fn render_caddyfile(template: &str, portal_domain: &str, marketing_domain: Option<&str>) -> String
```

Substitutions:
- `coterie.example.com` → portal_domain
- `example.com, www.example.com` → `${MARKETING_DOMAIN}, www.${MARKETING_DOMAIN}` (or the entire second site block is removed if no marketing domain)

Golden tests cover: portal-only, portal + marketing, edge cases. The function is also `pub` so tests can call it directly.

After writing the Caddyfile:
- `mkdir -p /var/log/caddy && chown -R caddy:caddy /var/log/caddy` (the bug we hit manually this session)
- Shell out to `caddy validate --config /etc/caddy/Caddyfile`
- `systemctl reload caddy`

If `caddy validate` fails, that's a wizard bug (the inputs were validated upstream). The error is surfaced with full context and exit code 1.

### D8. Stripe / Discord / UniFi conditional config

Each integration has an "enable?" prompt. If yes, the integration's credentials are prompted and written to `.env`. If no, the wizard writes the `ENABLED=false` line and leaves the value lines commented out (preserving the .env.example structure).

For Stripe specifically, the wizard prints a one-liner at the end pointing at `deploy/STRIPE-SETUP.md` to walk the operator through the dashboard side of webhook registration.

### D9. Final smoke test

After everything's running, the wizard issues:

```rust
reqwest::blocking::get("http://127.0.0.1:8080/health")?
    .error_for_status()?
```

Expected: a 200 with JSON. Crucially, NOT a 303 to /setup — because the admin already exists from the create_admin step, the setup-redirect middleware forwards normally.

If the smoke test fails, the wizard prints a diagnostic block:
- Last 50 lines of `journalctl -u coterie --no-pager`
- `systemctl status coterie --no-pager`
- A hint about manual debugging

The smoke test invocation goes through the `SystemCommand` trait — in tests, the fake implementation returns a canned response; in prod, it's a real HTTP call.

### D10. Exit summary

Final output (success path):

```
============================================================
Coterie installation complete.

  Org name:         Neon Temple
  Portal URL:       https://coterie.theneontemple.com
  Admin email:      rab@theneontemple.com
  Service status:   active (running)

Next steps:
  1. Point DNS for coterie.theneontemple.com at this box's
     public IP. Caddy will auto-provision a TLS cert on the
     first inbound HTTPS request.
  2. Register a Stripe webhook (if Stripe enabled):
     URL:    https://coterie.theneontemple.com/api/payments/webhook/stripe
     Events: see deploy/STRIPE-SETUP.md
  3. Log in: visit https://coterie.theneontemple.com/login
============================================================
```

### D11. Testability via trait abstractions

The autocoder must be able to validate everything it writes via `cargo test`. To make that possible, all side-effecting code is behind one of two traits:

```rust
pub trait SystemCommand {
    fn run(&self, cmd: &str, args: &[&str]) -> Result<CommandOutput>;
    fn run_with_stdin(&self, cmd: &str, args: &[&str], stdin: &[u8]) -> Result<CommandOutput>;
}

pub trait FileSystem {
    fn read_to_string(&self, path: &Path) -> Result<String>;
    fn write(&self, path: &Path, contents: &[u8]) -> Result<()>;
    fn create_dir_all(&self, path: &Path) -> Result<()>;
    fn chmod(&self, path: &Path, mode: u32) -> Result<()>;
    fn chown(&self, path: &Path, user: &str, group: &str) -> Result<()>;
    fn exists(&self, path: &Path) -> bool;
    fn rename(&self, from: &Path, to: &Path) -> Result<()>;
    fn remove_file(&self, path: &Path) -> Result<()>;
}
```

Production: `RealSystem` and `RealFs` implementations call `std::process::Command` and `std::fs` respectively.

Tests: `FakeSystem` and `FakeFs` record calls and return canned responses. The crate's `tests/` directory has integration tests that drive `coterie-provision install` end-to-end against the fakes, asserting on:
- The expected sequence of commands invoked (`apt-get update`, `apt-get install`, `release-deploy.sh`, `create_admin`, `caddy validate`, `systemctl ...`).
- The final state of the filesystem (what `.env` contents would be, what `/etc/caddy/Caddyfile` would contain).
- Idempotency: running twice produces the same final state with appropriate "skip, already done" decisions in between.

This is the same pattern that's worked for similar Rust CLIs (`ci-cd-imple-agents` precedent).

## Risks / Trade-offs

- **Risk**: a Rust crate is more setup than a single bash file. → Mitigation: workspace integration means it builds with the rest of Coterie automatically. The release workflow already builds Rust binaries; adding a third target is incremental.
- **Risk**: the wizard becomes a maintenance burden as Coterie's config evolves. → Mitigation: the wizard works from `.env.example` shipped in the release tarball — when `.env.example` changes, the wizard's behavior updates automatically (assuming new fields are optional or the wizard learns to handle them in the same release).
- **Risk**: operator runs the wizard partway, it fails, leaves the box in a half-configured state. → Mitigation: idempotency checks let the operator re-run safely; if state is genuinely confused, `uninstall.sh --data` and re-provision is a clear recovery path.
- **Risk**: prompts ambiguous or confusing on first run. → Mitigation: every prompt has a one-line explanation + an example + a default if applicable. `--help` enumerates everything.
- **Risk**: the trait abstractions get in the way of writing the binary fast. → Mitigation: keep the traits minimal — `SystemCommand` and `FileSystem` cover ~all of it. Don't over-abstract.
- **Trade-off**: the bash bootstrap duplicates a small amount of GitHub-API logic that's also in the Rust binary (release tag discovery). When the API contract changes, both update. Mitigation: documented in a comment, single owner.

## Migration Plan

Single PR.

1. Scaffold the `coterie-provision` crate as a workspace member.
2. Implement the pure-function modules (`env_template`, `caddyfile`, `version_selector`, prefix validators) with golden tests.
3. Implement the `SystemCommand` / `FileSystem` traits + their `Real*` and `Fake*` implementations.
4. Implement the wizard flow as a state machine using the traits.
5. Implement `clap` argument parsing and the env-var fallback layer.
6. Write `deploy/provision.sh` bootstrap.
7. Update `.github/workflows/release.yml` to build `coterie-provision` as a third musl-static binary + tarball + SHA256.
8. Update `DEPLOY-DIGITALOCEAN.md` to make the wizard the primary path; demote the manual section.
9. Update `README.md` deploy section to lead with the wizard.
10. `cargo test`, `cargo clippy --deny warnings`, `cargo fmt --check` — the autocoder's validation gates.
11. PR description includes operator smoke instructions (fresh Debian 13 VM, run the curl-and-bash) so the operator can run them post-merge as a manual gate.
