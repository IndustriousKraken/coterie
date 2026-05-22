## Why

The manual deploy walkthrough in `DEPLOY-DIGITALOCEAN.md` is ~15 numbered steps spanning OS prereqs, volume mounting, fstab editing, Coterie install, .env config, Caddy config, log-dir creation, TLS provisioning, and the /setup wizard. Even with the recent improvements (`release-deploy.sh`, `uninstall.sh`, `install.sh` fixes, `STRIPE-SETUP.md`), the first-deploy experience is hours of work where every step has at least one foot-gun. This session alone surfaced eight of them — tmpfs as build dir, `$EDITOR` unbound under `set -u`, the install.sh next-steps message lagging the actual state, root-owned log files in /var/log/caddy, missing `?mode=rwc` on the SQLite URL, CDN-hosted JS deps failing in browsers, the domain typo cascade, and the unauthenticated /setup hijack window.

The fix is to script the happy path end-to-end. One curl + one invocation, interactive prompts for the org-specific bits, takes the operator from a fresh Debian 13 droplet to a running Coterie with TLS, first admin, and optional integrations enabled — without the operator ever needing to remember `mkdir -p /var/log/caddy && chown -R caddy:caddy` or look up how to generate a session secret. This is the table-stakes deploy experience modern operators expect.

The wizard is the user-facing entry point; everything we've built earlier (`release-deploy.sh`, `install.sh`, `create_admin`, the GitHub Release artifacts, `Caddyfile.example`) is the substrate the wizard calls into.

## What Changes

- **New script**: `deploy/provision.sh`. Pure bash (no exotic deps; runs on a fresh Debian 13 install with the standard tools).
- **Curl-runnable from the repo**:
  ```sh
  curl -sfL https://raw.githubusercontent.com/IndustriousKraken/coterie/master/deploy/provision.sh \
      -o /tmp/provision.sh
  bash /tmp/provision.sh
  ```
  Recommend `curl -o + bash` over `curl | bash` so the operator can read it first. Both work.
- **Workflow tweak** (`.github/workflows/release.yml`): tags matching `^v\d+\.\d+\.\d+$` (pure-semver, e.g. `v1.0.0`) are stable releases; anything else (`v1.0.0dev`, `v1.0.0-rc1`, `v1.0.0alpha`, etc.) is published with `prerelease: true`. This makes GitHub's `/releases/latest` endpoint return only stable releases, so the wizard's "default to latest" semantics and `release-deploy.sh` (no args) both pick safe versions. Operators can still explicitly install prereleases by tag.
- **Interactive prompts** (each with a sensible default where one exists):
  - Coterie version to install (default: latest stable release per the workflow's prerelease marking; offer a list of the most recent ~5 releases via the GitHub Releases API so the operator can roll forward or back. Filter `prerelease: false` for the default; operators can opt into prereleases by selecting them from the list)
  - Org name (free text — used in emails, page titles, receipts)
  - Portal domain (e.g. `coterie.example.com`)
  - Marketing domain (optional, e.g. `example.com` — for the second Caddy vhost)
  - Org contact email (for AdminAlert delivery)
  - First admin: email, username, full name, password (entered twice for confirmation)
  - Stripe enable? If yes: publishable key, secret key, webhook signing secret
  - Discord enable? If yes: bot token, guild id, default announcements channel id
  - UniFi enable? If yes: controller URL, username, password
  - Caddy: install + configure? (yes/no)
- **Idempotency**: detect existing state and prompt before clobbering:
  - Existing `/opt/coterie/.env` → ask "overwrite?"
  - Existing admin in DB (via `create_admin` exit code 2) → skip create-admin step, surface the conflict
  - Existing `/etc/caddy/Caddyfile` → diff and ask
  - Existing systemd unit → leave alone (install.sh is idempotent)
- **Steps performed**:
  1. `apt-get update && apt-get install -y` the required packages (curl, python3, tar, sqlite3, ca-certificates, openssl) plus caddy if chosen.
  2. Fetch and run `release-deploy.sh` (which fetches the latest GitHub Release tarball, runs `install.sh`).
  3. Generate `/opt/coterie/.env` from the collected inputs. Session secret via `openssl rand -hex 32`.
  4. Write the admin password to `/tmp/coterie-bootstrap.pw` with `chmod 600`, run `/opt/coterie/create_admin --password-file /tmp/coterie-bootstrap.pw --email ... --username ... --full-name ...`, then `shred -u /tmp/coterie-bootstrap.pw`.
  5. If Caddy chosen: write `/etc/caddy/Caddyfile` from `Caddyfile.example` with domain substitution, `mkdir -p /var/log/caddy && chown -R caddy:caddy /var/log/caddy`, `caddy validate && systemctl reload caddy`.
  6. `systemctl enable --now coterie`.
  7. Smoke test: `curl -sf http://127.0.0.1:8080/health` — expect 200 + the health JSON (not a 303 to /setup, because the admin already exists).
  8. Final summary: print the portal URL, admin email, next steps (point DNS at the box, register Stripe webhook, etc.).
- **Non-interactive mode** (for IaC and re-provisioning): all prompts have a matching env var (e.g. `COTERIE_PROVISION_ORG_NAME`, `COTERIE_PROVISION_ADMIN_EMAIL`, `COTERIE_PROVISION_STRIPE_SK`). If every required var is set, the wizard runs without prompts. Mixed mode (some env vars set, others prompted) also works.
- **Bonus**: `deploy/provision.sh --dry-run` prints the steps it would take without executing anything. Useful for review before a real run.

## Capabilities

### New Capabilities
- `provisioning-wizard`: a single bash script that takes a fresh Debian 13 box from clean to "Coterie running with TLS and a first admin, with optional Stripe/Discord/UniFi configured."

### Modified Capabilities

(None — the wizard composes existing capabilities, doesn't change them. Modest changes to the deploy docs to point at the wizard as the primary path.)

## Impact

- **Code**:
  - New `deploy/provision.sh` (~500–600 lines including prompt logic, idempotency checks, .env templating, Caddyfile substitution, final summary). Bash with `set -euo pipefail`, well-commented.
  - Small updates to `DEPLOY-DIGITALOCEAN.md` to point at the wizard as the primary path and demote the manual flow to "the long way / debug mode."
- **Wire shape**: no runtime change. The wizard just automates what an operator would do manually.
- **Dependency**: depends on `create-admin-cli` (uses `create_admin` to bootstrap the admin) and the existing `release-deploy.sh` infrastructure. Both already exist (create_admin is a prerequisite change). The wizard fails fast if the binary isn't present in the release tarball it pulls.
- **Tests**:
  - Unit tests via bash itself are awkward; rely on manual smoke testing on a fresh Debian VM. CI can add a Vagrant/Docker-based integration test as a follow-up.
  - The wizard's `--dry-run` mode is the testability hook — operators can audit before running, and CI can run `bash provision.sh --dry-run < expected_inputs.txt` for a basic sanity check.
- **Risk**: medium. Bash + interactive prompts + multiple `sudo`-style operations + writes to /etc and /opt = lots of surface area. Mitigations: extensive idempotency checks, `--dry-run` mode, clear error messages with rollback hints, the underlying `uninstall.sh` for recovery.
- **Documentation**: the wizard IS the documentation, in the sense that reading the script tells you what a Coterie deploy needs. Reduces the docs-vs-code drift surface.
