# provisioning-wizard Specification

## Purpose
TBD - created by archiving change a24-provisioning-wizard. Update Purpose after archive.
## Requirements
### Requirement: coterie-provision is the primary first-deploy path

The system SHALL ship a Rust binary `coterie-provision` (alongside `coterie` and `create_admin` in the release tarball) that performs an end-to-end Coterie install on a fresh Debian 13 host. The binary's `install` subcommand performs the wizard flow.

A thin bash bootstrap `deploy/provision.sh` SHALL exist in the repo, curl-able from `master`. The bootstrap SHALL run under `set -euo pipefail`, refuse if not root, refuse if not Debian, fetch the latest stable release tag (or accept `--tag`), download the `coterie-provision-<tag>-x86_64-unknown-linux-musl.tar.gz` asset, verify it against the matching `.sha256` asset, extract `coterie-provision`, and `exec` it with `install` and any pass-through flags. The bootstrap SHALL be short enough (~50 lines) that a curious operator can read it before running.

From operator perspective, a single command sequence (curl + bash) takes a clean box to a running Coterie instance with a first admin already created, .env populated, optional integrations configured, and (if chosen) Caddy serving with TLS.

The `README.md` deploy section SHALL recommend the wizard as the primary deploy path. The curl-and-bash one-liner SHALL appear inline. Any prior manual-deploy steps in the README are demoted below the wizard (under an "Advanced / manual" heading) or replaced with a link to `DEPLOY-DIGITALOCEAN.md`. A new operator reading the README SHALL encounter the wizard before any manual alternative.

#### Scenario: First-time install on a fresh Debian 13 droplet

- **WHEN** an operator provisions a new Debian 13 droplet, curls the bootstrap, and runs it interactively answering the prompts
- **THEN** at the end: `/opt/coterie/coterie` is running under systemd, `/var/lib/coterie/coterie.db` exists with a first-admin row, `/opt/coterie/.env` is populated with the supplied values + a generated session secret, and a `GET http://127.0.0.1:8080/health` returns 200 with JSON (not a 303 to /setup)

#### Scenario: Idempotent re-run after a partial failure

- **WHEN** an operator re-runs the wizard after a failure mid-way through
- **THEN** the wizard SHALL detect existing state (`.env` already populated, admin already exists, systemd unit already installed) and prompt before clobbering each; the operator can choose to skip steps that already succeeded

#### Scenario: --dry-run mode shows the plan

- **WHEN** the operator invokes `coterie-provision install --dry-run`
- **THEN** the wizard SHALL print every step it would take (with the actual command lines + the .env content + the Caddyfile substitution result) WITHOUT executing any of them; no side effects occur

#### Scenario: README points new operators at the wizard

- **WHEN** a new operator reads the README's deploy section
- **THEN** the first thing presented SHALL be the wizard (with the curl-and-bash one-liner); manual deploy instructions, if retained, SHALL appear below under an "Advanced / manual" heading or as a link to `DEPLOY-DIGITALOCEAN.md`

### Requirement: Wizard offers version selection with safe defaults

The wizard SHALL allow the operator to choose which Coterie version to install. The default SHALL be the **latest stable release** (most recent release where `prerelease: false` in the GitHub API response). Operators can optionally view older stable releases (for rollback) or prereleases (for installing dev builds).

The release workflow (`.github/workflows/release.yml`) SHALL mark tags matching `^v\d+\.\d+\.\d+$` as stable releases and all other tags (e.g. `v1.0.0dev`, `v1.0.0-rc1`, `v1.0.0alpha`) with `prerelease: true`. This ensures GitHub's API correctly classifies each tag and the wizard's "latest stable" semantics work cleanly.

#### Scenario: Default is latest stable

- **WHEN** the wizard runs and the operator accepts the default version selection
- **THEN** the latest tag matching `^v\d+\.\d+\.\d+$` SHALL be selected; dev/alpha/beta/rc tags SHALL NOT be picked as default

#### Scenario: Operator can pin a specific stable version

- **WHEN** the operator selects an older stable release from the menu (e.g. for rollback)
- **THEN** the wizard SHALL pass that tag to `release-deploy.sh` for installation

#### Scenario: Operator can opt into prereleases explicitly

- **WHEN** the operator selects "show all releases including prereleases" from the menu
- **THEN** the wizard SHALL list all recent releases (stable + prerelease) and let the operator pick a dev/rc tag

#### Scenario: Env-var or flag override works

- **WHEN** `COTERIE_PROVISION_VERSION=v1.0.0` is set OR `--version v1.0.0` is passed
- **THEN** the wizard SHALL skip the version-selection prompt and use the specified tag verbatim

#### Scenario: release.yml correctly classifies dev tags

- **WHEN** a tag matching `*dev`, `*alpha`, `*beta`, or `*rc*` is pushed
- **THEN** the published GitHub Release SHALL have `prerelease: true`; the `/releases/latest` API SHALL NOT return it; `release-deploy.sh` (no args) SHALL skip it

### Requirement: All prompts have a non-interactive equivalent

Every interactive prompt SHALL check for a `COTERIE_PROVISION_<NAME>` environment variable AND a matching `--flag` argument first, and if either is set, skip the prompt. If every required input is supplied via env or flag, the wizard runs without prompts (suitable for IaC pipelines).

The binary's `--help` output SHALL enumerate all env vars and flags.

Required inputs (only some — the full list is in `--help`):
- `COTERIE_PROVISION_ORG_NAME` / `--org-name`
- `COTERIE_PROVISION_PORTAL_DOMAIN` / `--portal-domain`
- `COTERIE_PROVISION_ADMIN_EMAIL` / `--admin-email`
- `COTERIE_PROVISION_ADMIN_USERNAME` / `--admin-username`
- `COTERIE_PROVISION_ADMIN_FULL_NAME` / `--admin-full-name`
- `COTERIE_PROVISION_ADMIN_PASSWORD` / `--admin-password` (env preferred for secrets)
- `COTERIE_PROVISION_ENABLE_STRIPE` / `--enable-stripe` (`true`/`false`)
- `COTERIE_PROVISION_ENABLE_CADDY` / `--enable-caddy` (`true`/`false`)

Optional, conditional on enabling integrations: Stripe / Discord / UniFi credential vars and flags.

#### Scenario: Fully scripted run for IaC

- **WHEN** the operator exports every required env var and runs `coterie-provision install --no-prompt`
- **THEN** the wizard SHALL execute without any interactive prompts and complete the same end state as the interactive flow

#### Scenario: Mixed interactive + non-interactive run

- **WHEN** the operator sets some env vars (e.g. `COTERIE_PROVISION_ORG_NAME`, `COTERIE_PROVISION_PORTAL_DOMAIN`) but not others
- **THEN** the wizard SHALL skip the prompts whose env vars/flags are set and prompt only for the unset ones

#### Scenario: --no-prompt errors when required inputs are missing

- **WHEN** `--no-prompt` is passed but a required env var or flag is missing
- **THEN** the wizard SHALL exit non-zero with a clear message listing every missing required input

### Requirement: Admin password handling does not leak

The wizard SHALL handle the admin password such that it never appears in:
- The shell's process listing (no `--password "..."` on a command line to create_admin)
- Any log file emitted by the wizard or by any process it invokes
- Debug output (the password is wrapped in `secrecy::Secret<String>` whose `Debug` impl prints `[REDACTED]`)
- The .env file (passwords are hashed into the DB by `create_admin`, not stored as env vars)

The password is passed to `create_admin` via a `--password-file` whose contents are written with mode 0600 and removed (via `tempfile::NamedTempFile`'s `Drop` impl, plus an explicit overwrite-with-zeros step) immediately after `create_admin` returns, regardless of whether create_admin succeeded.

#### Scenario: Password is in a mode-0600 tempfile briefly

- **WHEN** the wizard reaches the create_admin step
- **THEN** it SHALL write the password to a `tempfile`-managed file with mode 0600, invoke `create_admin --password-file <path>`, then overwrite-and-unlink the file regardless of the exit code

#### Scenario: Password does not appear in process listings during create_admin

- **WHEN** another user runs `ps aux` while create_admin is in progress
- **THEN** the password SHALL NOT be visible in any process argv

### Requirement: Caddyfile generation includes the log-directory fix

When the operator chooses to install Caddy, the wizard SHALL:

1. Read `/opt/coterie/deploy/Caddyfile.example` (placed there by `release-deploy.sh`)
2. Substitute the operator's portal domain in place of `coterie.example.com`
3. Substitute the operator's marketing domain (or remove the second site block if no marketing domain was supplied) — substitution logic lives in a pure function `render_caddyfile` with golden test coverage
4. `mkdir -p /var/log/caddy && chown -R caddy:caddy /var/log/caddy` BEFORE the Caddy reload (this was the root cause of one of this session's deploy failures)
5. Write the result to `/etc/caddy/Caddyfile`
6. Run `caddy validate --config /etc/caddy/Caddyfile`; on failure, surface the error with full context (treat as a wizard bug, not an operator-fixable input issue)
7. `systemctl reload caddy` (or restart if the previous config was broken)

#### Scenario: Fresh Caddy install includes log-dir setup

- **WHEN** the wizard installs Caddy on a host where `/var/log/caddy/` doesn't exist
- **THEN** the wizard SHALL `mkdir -p` it and `chown -R caddy:caddy` it BEFORE attempting Caddy reload, so the "log writer permission denied" failure from this session doesn't reoccur

#### Scenario: render_caddyfile is unit-tested

- **WHEN** the test suite runs
- **THEN** golden tests SHALL cover `render_caddyfile` for: portal-only (no marketing domain), portal + marketing, edge cases (domain with hyphens, subdomain depth); the test SHALL NOT require running `caddy validate` (which isn't available in the autocoder sandbox)

### Requirement: Wizard uses create_admin to bootstrap before service start

The wizard SHALL invoke `/opt/coterie/create_admin` with the supplied credentials AFTER `release-deploy.sh` has placed the binary and BEFORE `systemctl start coterie`. This sequence closes the unauthenticated /setup hijack window — by the time Coterie binds port 8080, an admin already exists; the setup-redirect middleware forwards normally and /setup is unreachable.

#### Scenario: Service starts with admin already present

- **WHEN** the wizard completes the create_admin step and then runs `systemctl enable --now coterie`
- **THEN** Coterie starts with an admin row in `members` already; the `admin_exists_observed` cache populates on the first request; /setup redirects to /login from the moment the service is reachable

### Requirement: Side-effecting code is behind testable traits

All side-effecting operations (process invocation, filesystem read/write/chmod/chown, HTTP calls) SHALL go through `SystemCommand` and `FileSystem` (or similar) trait abstractions. Production code uses `Real*` implementations that call `std::process::Command`, `std::fs`, and `reqwest`. Test code uses `Fake*` implementations that record calls and return canned responses.

This requirement exists so the autocoder can fully validate the wizard via `cargo test` without spinning up VMs or invoking real apt-get/systemctl.

#### Scenario: Integration test drives install end-to-end with fakes

- **WHEN** the test `tests/install_flow.rs` runs `coterie-provision install` against `FakeSystem` and `FakeFs`
- **THEN** the test SHALL assert: the expected commands were invoked in the expected order, the .env file's would-be contents match a golden snapshot, the Caddyfile's would-be contents match a golden snapshot, and rerunning the same flow against the same fakes produces the same idempotent end state

#### Scenario: cargo test passes in the autocoder sandbox

- **WHEN** the autocoder runs `cargo test -p coterie-provision`
- **THEN** all tests SHALL pass without requiring shellcheck, a VM, or any tooling outside the standard Rust toolchain

### Requirement: Wizard offers a test-mode-or-live-mode choice

The `coterie-provision install` subcommand SHALL prompt the operator to choose between test mode and live mode when configuring Stripe. The prompt SHALL be presented if and only if Stripe is being enabled. Default selection: **live mode** (matching the `a24` wizard's baseline behavior).

If test mode is selected:
- The wizard SHALL collect test-mode Stripe credentials (`pk_test_*`, `sk_test_*`, test webhook signing secret `whsec_…`). Prefix validation reuses `a24`'s `validate_prefix` helper.
- The wizard SHALL configure `.env` with `DATABASE_URL` pointing at `coterie-test.db`.
- The wizard MAY (operator's choice) ALSO collect live-mode credentials and stash them in `/opt/coterie/.env.live` (chmod 0640, owned by `coterie`, written via the `FileSystem` trait) for the future switchover.
- After the wizard completes, a verification checklist SHALL be printed describing the manual flows to test and the command to run when ready to switch to live mode.

If live mode is selected:
- Wizard behavior is identical to the `a24` baseline (collects live credentials, `coterie.db` is the database).

#### Scenario: Test mode selected, switchover guidance printed

- **WHEN** the wizard runs with test mode selected
- **THEN** the final output SHALL include a verification checklist (suggested flows + how to run each) and the command line to invoke `coterie-provision switch-stripe-to-live` when ready

#### Scenario: Live mode selected behaves identically to a24

- **WHEN** the wizard runs with live mode selected
- **THEN** the resulting Coterie instance SHALL match the `a24` baseline behavior — `.env` configured with live credentials, `coterie.db` as the database, no `coterie-test.db` or `.env.live` artifacts on disk

#### Scenario: Env-var or flag override skips the test-or-live prompt

- **WHEN** `COTERIE_PROVISION_STRIPE_MODE=test` (or `=live`) is set, OR `--stripe-mode test` (or `live`) is passed
- **THEN** the wizard SHALL skip the prompt and use the specified mode

#### Scenario: Test-mode path is covered by integration test

- **WHEN** the autocoder runs `cargo test -p coterie-provision`
- **THEN** an integration test SHALL drive the install subcommand in test mode against `FakeSystem` and `FakeFs`, asserting that `.env` is written with `coterie-test.db` as the DATABASE_URL, `.env.live` is created if live creds were also supplied, and the verification checklist text appears in the captured stdout

