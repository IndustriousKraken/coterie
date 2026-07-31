# provisioning-wizard Specification

## MODIFIED Requirements

### Requirement: coterie-provision is the primary first-deploy path

The system SHALL ship a Rust binary `coterie-provision` (alongside `coterie` and `create_admin` in the release tarball) that performs an end-to-end Coterie install on a fresh Debian 13 host. The binary's `install` subcommand performs the wizard flow.

A thin bash bootstrap `deploy/provision.sh` SHALL exist in the repo, curl-able from `master`. The bootstrap SHALL run under `set -euo pipefail`, refuse if not root, refuse if not Debian, fetch the latest stable release tag (or accept `--tag`), download the `coterie-provision-<tag>-x86_64-unknown-linux-musl.tar.gz` asset, verify it against the matching `.sha256` asset, extract `coterie-provision`, and `exec` it with `install` and any pass-through flags. The bootstrap SHALL be short enough (~50 lines) that a curious operator can read it before running.

From operator perspective, a single command sequence (curl + bash) takes a clean box to a running Coterie instance with a first admin already created, .env populated, optional integrations configured, (if chosen) Caddy serving with TLS, **and scheduled backups running**.

The wizard SHALL install and enable the backup timer as part of the install, so a provisioned deployment is protected without a separate operator action. A backup script that nothing schedules does not constitute delivered backups: the omission is silent, the operator reasonably assumes the feature is active because it exists in the repository, and the gap surfaces only when a restore is attempted.

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

#### Scenario: A provisioned host has backups scheduled

- **WHEN** an operator completes the wizard on a fresh host
- **THEN** the backup timer SHALL be installed and enabled, and `systemctl list-timers` SHALL show it scheduled

#### Scenario: Re-running the wizard does not duplicate the backup schedule

- **WHEN** the wizard is re-run on a host that already has the backup timer installed
- **THEN** it SHALL detect the existing timer and leave it enabled rather than installing a second copy, consistent with the idempotency rule above
