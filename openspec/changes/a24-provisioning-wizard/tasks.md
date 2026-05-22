## 1. Scaffold the coterie-provision crate

- [ ] 1.1 Decide crate placement: workspace member (preferred — add to root `Cargo.toml`'s `[workspace.members]`) or standalone crate in `deploy/coterie-provision/`. Either is acceptable; pick whichever requires fewer changes to existing Cargo setup. If the existing root Cargo.toml has no `[workspace]` table, add one and convert.
- [ ] 1.2 Create `deploy/coterie-provision/Cargo.toml`. Dependencies: `clap` (derive feature), `anyhow`, `serde` + `serde_json`, `reqwest` (blocking + rustls-tls), `inquire` (or `dialoguer`), `secrecy`, `tempfile`, `rand`, `hex`, `regex`. Dev-dependencies: `pretty_assertions`, `tempfile`. Pin a single target — musl-static x86_64-linux for the release artifact.
- [ ] 1.3 Skeleton `src/main.rs`: clap derive struct with two subcommands `Install` and `SwitchStripeToLive` (the latter is filled in by `a25`; leave a `todo!()` stub here). Top-level error handling: `anyhow::Result<()>` returned from `main`; pretty-print the error chain on exit.
- [ ] 1.4 Skeleton modules: `src/install.rs`, `src/env_template.rs`, `src/caddyfile.rs`, `src/version_selector.rs`, `src/prompts.rs`, `src/stripe_check.rs`, `src/system.rs` (traits), `src/fs_ops.rs` (traits), `src/github_api.rs`, `src/lib.rs` (re-exports for tests).

## 2. Trait abstractions for OS effects

- [ ] 2.1 In `src/system.rs`: define `pub trait SystemCommand { fn run(&self, ...) -> Result<CommandOutput>; ... }`. Methods needed: `run(cmd, args)`, `run_with_stdin(cmd, args, stdin_bytes)`, `run_interactive(cmd, args)` (for invoking caddy/apt where the operator may see output). `CommandOutput` is a struct with `status`, `stdout`, `stderr`.
- [ ] 2.2 Implement `RealSystem` in same file using `std::process::Command`.
- [ ] 2.3 In `src/fs_ops.rs`: define `pub trait FileSystem` with methods `read_to_string`, `write`, `append`, `create_dir_all`, `chmod`, `chown`, `exists`, `is_file`, `is_dir`, `rename`, `remove_file`, `remove_dir_all`.
- [ ] 2.4 Implement `RealFs` using `std::fs` + `nix::unistd::{chown, Uid, Gid}` for `chown`. (Or use a `chown` subprocess to avoid the `nix` dependency — implementer's call.)
- [ ] 2.5 Implement `FakeSystem` and `FakeFs` in `src/lib.rs` (or a `pub mod test_support`) feature-gated behind `#[cfg(any(test, feature = "test-support"))]`. `FakeSystem` records every call in `RefCell<Vec<RecordedCall>>` and looks up responses from a configurable `HashMap<(cmd, args), CommandOutput>` (with a default of "exit 0, empty stdout"). `FakeFs` is an in-memory `HashMap<PathBuf, Vec<u8>>` + a record of every operation.

## 3. Pure-function modules with golden tests

- [ ] 3.1 `src/env_template.rs`: `pub fn render_env(template: &str, config: &EnvConfig) -> String`. `EnvConfig` is a struct with fields for every input the wizard collects. Use simple string-replace on `${VAR}` markers in the template (no full templating engine needed).
- [ ] 3.2 Test fixtures: `tests/fixtures/env_example.txt` (a copy of `.env.example` at the time of writing — keep updated as `.env.example` evolves). Tests assert `render_env(fixture, config)` matches a golden output for: (a) live mode with all integrations enabled, (b) live mode with no optional integrations, (c) integration-specific permutations.
- [ ] 3.3 `src/caddyfile.rs`: `pub fn render_caddyfile(template: &str, portal_domain: &str, marketing_domain: Option<&str>) -> String`. Same substitution pattern.
- [ ] 3.4 Tests in `tests/caddyfile.rs`: golden tests for portal-only, portal + marketing, weird-but-valid domains.
- [ ] 3.5 `src/version_selector.rs`: `pub fn parse_releases(json: &str) -> Result<Vec<Release>>` and `pub fn select_default_stable(releases: &[Release]) -> Option<&Release>`. `Release` has `tag_name`, `prerelease`, `published_at`. The default-stable filter is `prerelease == false`, sorted descending by `published_at`.
- [ ] 3.6 Tests: feed a `tests/fixtures/github_releases.json` blob with a mix of stable and prerelease tags; assert the default-stable picker returns the expected one.
- [ ] 3.7 `src/stripe_check.rs`: `pub fn validate_prefix(value: &str, expected: &str) -> Result<()>` for `pk_test_`, `sk_test_`, `pk_live_`, `sk_live_`, `whsec_`. Unit tests for each.

## 4. Prompts layer (interactive + non-interactive)

- [ ] 4.1 `src/prompts.rs`: `pub fn resolve<T: FromStr>(name: &str, env_var: &str, cli_value: Option<T>, default: Option<T>, prompt_fn: impl FnOnce() -> Result<T>) -> Result<T>`. Order: cli_value → env var → (interactive prompt OR default if `--no-prompt`).
- [ ] 4.2 Concrete prompt helpers: `prompt_text(message, default)`, `prompt_secret(message)` (uses `inquire::Password::new` with confirmation), `prompt_yn(message, default)`, `prompt_select(message, items)`.
- [ ] 4.3 Tests: provide a `MockPrompter` trait (or inject prompts via a closure) so tests can avoid the real `inquire` terminal interaction.

## 5. CLI argument parsing

- [ ] 5.1 `src/main.rs`: clap derive struct with `coterie-provision install [flags]` and `coterie-provision switch-stripe-to-live [flags]` subcommands.
- [ ] 5.2 `install` flags: `--org-name`, `--portal-domain`, `--marketing-domain`, `--contact-email`, `--admin-email`, `--admin-username`, `--admin-full-name`, `--admin-password`, `--enable-stripe true|false`, `--enable-caddy true|false`, `--enable-discord true|false`, `--enable-unifi true|false`, integration credential flags, `--version <tag>`, `--no-prompt`, `--dry-run`.
- [ ] 5.3 Tests: parsing fixtures cover defaults, all-flags-set, mixed.

## 6. Install flow as a state machine

- [ ] 6.1 `src/install.rs`: `pub fn run<S: SystemCommand, F: FileSystem>(args: InstallArgs, sys: &S, fs: &F, prompts: &impl Prompter) -> Result<()>`. The function is parametric over the traits — same code runs in prod (with `RealSystem`, `RealFs`) and tests (with `FakeSystem`, `FakeFs`).
- [ ] 6.2 Implement the steps in order:
   1. Preflight: refuse if not root (skipped under `--dry-run`), refuse if not Debian, warn if `/var/lib/coterie` isn't a mount point, detect idempotency state.
   2. Gather inputs via the prompts layer.
   3. Confirmation summary + final y/N (skipped under `--no-prompt` or `--dry-run`).
   4. Print + execute (or just print under `--dry-run`):
      - `apt-get update`
      - `apt-get install -y --no-install-recommends curl python3 tar sqlite3 ca-certificates openssl` (+ `caddy` if chosen, with Caddy apt repo setup beforehand)
      - Fetch `release-deploy.sh` to `/usr/local/bin/coterie-release-deploy`, chmod +x
      - Invoke `/usr/local/bin/coterie-release-deploy <selected_tag>`
      - Assert `/opt/coterie/coterie`, `/opt/coterie/create_admin` exist
      - Render and write `/opt/coterie/.env` via `render_env`
      - Generate session secret via `rand::thread_rng().gen::<[u8; 32]>()` + hex encode
      - Bootstrap admin via `create_admin --password-file` (tempfile)
      - Render and write `/etc/caddy/Caddyfile` if Caddy chosen + mkdir + chown + `caddy validate` + reload
      - `systemctl enable --now coterie`, wait for active
      - Smoke test via reqwest blocking GET `http://127.0.0.1:8080/health`
   5. Print exit summary.
- [ ] 6.3 The `--dry-run` path uses the fake traits even at runtime (or a "log only" wrapper around the real traits) so it prints what it would do without executing.

## 7. Idempotency detection

- [ ] 7.1 Detect `/opt/coterie/.env` exists → prompt overwrite (or honor `COTERIE_PROVISION_OVERWRITE_ENV=true`).
- [ ] 7.2 Detect admin already exists in DB (via `create_admin` exit code 2) → log + skip create-admin.
- [ ] 7.3 Detect `/etc/caddy/Caddyfile` exists with a Coterie marker comment → prompt overwrite.
- [ ] 7.4 Tests: integration test invokes the install twice against the same `FakeFs`; assert no clobbers, all idempotency prompts fire, and end state is consistent.

## 8. Bash bootstrap

- [ ] 8.1 Write `deploy/provision.sh` (~50 lines). Sections: shebang, `set -euo pipefail`, ERR trap, root check, OS detect, GitHub API tag lookup, asset download, SHA256 verify, extract, exec.
- [ ] 8.2 Bootstrap accepts `--tag <v...>` to override the auto-discovered latest stable tag; passes all other args through to `coterie-provision install`.
- [ ] 8.3 Bootstrap uses `mktemp -d` for the download/extract dir, with a `trap` to clean up on exit (success or failure).

## 9. Release workflow updates

- [ ] 9.1 Edit `.github/workflows/release.yml`. Add a third matrix entry / build step for `coterie-provision`:
   - `cargo build --release --target x86_64-unknown-linux-musl -p coterie-provision`
   - Tar the binary: `tar -czf coterie-provision-${TAG}-x86_64-unknown-linux-musl.tar.gz coterie-provision`
   - Generate SHA256 sidecar: `sha256sum coterie-provision-${TAG}-x86_64-unknown-linux-musl.tar.gz > coterie-provision-${TAG}-x86_64-unknown-linux-musl.tar.gz.sha256`
   - Attach both as release assets.
- [ ] 9.2 Add a "Determine prerelease flag" step. Compute prerelease based on the tag name:
   ```yaml
   - name: Determine prerelease flag
     id: prerelease
     run: |
       TAG="${{ steps.version.outputs.tag }}"
       if [[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
         echo "prerelease=false" >> $GITHUB_OUTPUT
       else
         echo "prerelease=true" >> $GITHUB_OUTPUT
       fi
   ```
- [ ] 9.3 In the `softprops/action-gh-release` step, add `prerelease: ${{ steps.prerelease.outputs.prerelease }}` to honor the classification.

## 10. Documentation

- [ ] 10.1 Update `deploy/DEPLOY-DIGITALOCEAN.md`: replace step 5 (current happy path) with a pointer to `provision.sh`. Demote the manual flow (build-from-source, rsync, install.sh) to "what the wizard does internally" or "manual fallback if you want to understand each step."
- [ ] 10.2 Update `README.md`: the deployment section SHALL recommend `provision.sh` as the primary/default path. Include the curl-and-bash one-liner inline. Any pre-existing manual-deploy instructions are demoted below the wizard (under a heading like "Manual deploy (advanced)") or replaced with a link to `DEPLOY-DIGITALOCEAN.md`. The wizard should be the first thing a new operator sees in the README's deploy section.
- [ ] 10.3 Keep `STRIPE-SETUP.md`, `uninstall.sh` documentation in place — the wizard composes them.

## 11. Validation (autocoder-runnable)

- [ ] 11.1 `cargo build -p coterie-provision --target x86_64-unknown-linux-musl --release` — confirm the binary builds for the release target.
- [ ] 11.2 `cargo test -p coterie-provision` — all unit + integration tests pass.
- [ ] 11.3 `cargo clippy -p coterie-provision -- --deny warnings` — clean.
- [ ] 11.4 `cargo fmt --check` — clean.
- [ ] 11.5 `bash -n deploy/provision.sh` — bootstrap syntax check.

## 12. Operator-side validation (NOT for the autocoder)

These tasks are documented in the PR description for the operator (Rab) to run as a manual gate before merging. The autocoder SHALL NOT claim to have completed them.

- [ ] 12.1 Operator: spin up a fresh Debian 13 droplet. Run the curl-and-bash bootstrap interactively. Confirm it completes and Coterie is reachable at the chosen domain.
- [ ] 12.2 Operator: re-run on the same box to test idempotency. Every step should either skip (because already done) or prompt before overwriting.
- [ ] 12.3 Operator: test `--dry-run`. Confirm no side effects, output is the plan.
- [ ] 12.4 Operator: test fully non-interactive with all env vars set. Confirm no prompts appear.
- [ ] 12.5 Operator: test recovery — provision → `uninstall.sh --all` → provision again. Should succeed both times.
