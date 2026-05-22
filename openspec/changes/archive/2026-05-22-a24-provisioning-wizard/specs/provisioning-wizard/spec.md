## ADDED Requirements

### Requirement: deploy/provision.sh is the primary first-deploy path

The system SHALL ship a `deploy/provision.sh` script that performs an end-to-end Coterie install on a fresh Debian 13 host. From operator perspective, a single command sequence (curl + bash) takes a clean box to a running Coterie instance with a first admin already created, .env populated, optional integrations configured, and (if chosen) Caddy serving with TLS.

The script SHALL be pure bash with no exotic dependencies. It SHALL run under `set -euo pipefail` with explicit error trapping that points the operator at `uninstall.sh` for recovery.

The `README.md` deploy section SHALL recommend `provision.sh` as the primary deploy path. The curl-and-bash one-liner SHALL appear inline. Any prior manual-deploy steps in the README are demoted below the wizard (under an "Advanced / manual" heading) or replaced with a link to `DEPLOY-DIGITALOCEAN.md`. A new operator reading the README SHALL encounter the wizard before any manual alternative.

#### Scenario: First-time install on a fresh Debian 13 droplet

- **WHEN** an operator provisions a new Debian 13 droplet, curls the wizard, and runs it interactively answering the prompts
- **THEN** at the end: `/opt/coterie/coterie` is running under systemd, `/var/lib/coterie/coterie.db` exists with a first-admin row, `/opt/coterie/.env` is populated with the supplied values + a generated session secret, and `curl http://127.0.0.1:8080/health` returns 200 with JSON (not a 303 to /setup)

#### Scenario: Idempotent re-run after a partial failure

- **WHEN** an operator re-runs the wizard after a failure mid-way through
- **THEN** the wizard SHALL detect existing state (`.env` already populated, admin already exists, systemd unit already installed) and prompt before clobbering each; the operator can choose to skip steps that already succeeded

#### Scenario: --dry-run mode shows the plan

- **WHEN** the operator invokes `bash provision.sh --dry-run`
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

#### Scenario: Env-var override works

- **WHEN** `COTERIE_PROVISION_VERSION=v1.0.0` is set
- **THEN** the wizard SHALL skip the version-selection prompt and use the specified tag verbatim

#### Scenario: release.yml correctly classifies dev tags

- **WHEN** a tag matching `*dev`, `*alpha`, `*beta`, or `*rc*` is pushed
- **THEN** the published GitHub Release SHALL have `prerelease: true`; the `/releases/latest` API SHALL NOT return it; `release-deploy.sh` (no args) SHALL skip it

### Requirement: All prompts have a non-interactive equivalent

Every interactive prompt SHALL check for a `COTERIE_PROVISION_<NAME>` environment variable first and, if set, skip the prompt. If every required env var is set, the wizard runs without prompts at all (suitable for IaC pipelines).

Required env vars (only some — the full list lives in the wizard's `--help` output):
- `COTERIE_PROVISION_ORG_NAME`
- `COTERIE_PROVISION_PORTAL_DOMAIN`
- `COTERIE_PROVISION_ADMIN_EMAIL`
- `COTERIE_PROVISION_ADMIN_USERNAME`
- `COTERIE_PROVISION_ADMIN_FULL_NAME`
- `COTERIE_PROVISION_ADMIN_PASSWORD`
- `COTERIE_PROVISION_ENABLE_STRIPE` (`true`/`false`)
- `COTERIE_PROVISION_ENABLE_CADDY` (`true`/`false`)

Optional, conditional on enabling integrations: Stripe / Discord / UniFi credential vars.

#### Scenario: Fully scripted run for IaC

- **WHEN** the operator exports every required env var and runs the wizard from a CI/automation context
- **THEN** the wizard SHALL execute without any interactive prompts and complete the same end state as the interactive flow

#### Scenario: Mixed interactive + env-var run

- **WHEN** the operator sets some env vars (e.g. `COTERIE_PROVISION_ORG_NAME`, `COTERIE_PROVISION_PORTAL_DOMAIN`) but not others
- **THEN** the wizard SHALL skip the prompts whose env vars are set and prompt only for the unset ones

### Requirement: Admin password handling does not leak

The wizard SHALL handle the admin password such that it never appears in:
- The shell's process listing (no `--password "..."` on a command line)
- Bash history (read via `read -sp` which doesn't echo, in a context HISTFILE doesn't capture)
- Log files emitted by the wizard or by any process it invokes
- The .env file (passwords are hashed into the DB by `create_admin`, not stored as env vars)

The password is passed to `create_admin` via a `--password-file` whose contents are written with `chmod 600` and `shred -u`'d immediately after `create_admin` returns, regardless of whether create_admin succeeded.

#### Scenario: Password is in a chmod 600 tempfile briefly

- **WHEN** the wizard reaches the create_admin step
- **THEN** it SHALL write the password to a `mktemp`-generated file with `chmod 600`, invoke `create_admin --password-file <path>`, then `shred -u <path>` regardless of the exit code

#### Scenario: Password does not appear in process listings during create_admin

- **WHEN** another user runs `ps aux` while create_admin is in progress
- **THEN** the password SHALL NOT be visible in any process argv

### Requirement: Caddyfile generation includes the log-directory fix

When the operator chooses to install Caddy, the wizard SHALL:

1. Read `/opt/coterie/deploy/Caddyfile.example` (placed there by `release-deploy.sh`)
2. Substitute the operator's portal domain in place of `coterie.example.com`
3. Substitute the operator's marketing domain (or remove the second site block if no marketing domain was supplied)
4. Run `mkdir -p /var/log/caddy && chown -R caddy:caddy /var/log/caddy` BEFORE the Caddy reload (this was the root cause of one of this session's deploy failures)
5. Write the result to `/etc/caddy/Caddyfile`
6. Run `caddy validate --config /etc/caddy/Caddyfile`; on failure, print the error and exit (operator's choice to skip Caddy or fix manually)
7. `systemctl reload caddy` (or restart if the previous config was broken)

#### Scenario: Fresh Caddy install includes log-dir setup

- **WHEN** the wizard installs Caddy on a host where `/var/log/caddy/` doesn't exist
- **THEN** the wizard SHALL `mkdir -p` it and `chown -R caddy:caddy` it BEFORE attempting Caddy reload, so the "log writer permission denied" failure from this session doesn't reoccur

### Requirement: Wizard uses create_admin to bootstrap before service start

The wizard SHALL invoke `/opt/coterie/create_admin` with the supplied credentials AFTER `release-deploy.sh` has placed the binary and BEFORE `systemctl start coterie`. This sequence closes the unauthenticated /setup hijack window — by the time Coterie binds port 8080, an admin already exists; the setup-redirect middleware forwards normally and /setup is unreachable.

#### Scenario: Service starts with admin already present

- **WHEN** the wizard completes the create_admin step and then runs `systemctl enable --now coterie`
- **THEN** Coterie starts with an admin row in `members` already; the `admin_exists_observed` cache populates on the first request; /setup redirects to /login from the moment the service is reachable
