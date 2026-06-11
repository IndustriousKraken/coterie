## Why

The manual deploy walkthrough in `DEPLOY-DIGITALOCEAN.md` is ~15 numbered steps spanning OS prereqs, volume mounting, fstab editing, Coterie install, .env config, Caddy config, log-dir creation, TLS provisioning, and the /setup wizard. Even with the recent improvements (`release-deploy.sh`, `uninstall.sh`, `install.sh` fixes, `STRIPE-SETUP.md`), the first-deploy experience is hours of work where every step has at least one foot-gun. This session alone surfaced eight of them — tmpfs as build dir, `$EDITOR` unbound under `set -u`, the install.sh next-steps message lagging the actual state, root-owned log files in /var/log/caddy, missing `?mode=rwc` on the SQLite URL, CDN-hosted JS deps failing in browsers, the domain typo cascade, and the unauthenticated /setup hijack window.

The fix is to script the happy path end-to-end. One curl + one invocation, interactive prompts for the org-specific bits, takes the operator from a fresh Debian 13 droplet to a running Coterie with TLS, first admin, and optional integrations enabled — without the operator ever needing to remember `mkdir -p /var/log/caddy && chown -R caddy:caddy` or look up how to generate a session secret. This is the table-stakes deploy experience modern operators expect.

The wizard is the user-facing entry point; everything we've built earlier (`release-deploy.sh`, `install.sh`, `create_admin`, the GitHub Release artifacts, `Caddyfile.example`) is the substrate the wizard calls into.

## What Changes

- **New Rust crate `coterie-provision`** (workspace member, or standalone crate inside `deploy/coterie-provision/` — whichever fits cleanest with the existing Cargo setup). Builds to a musl-static binary alongside `coterie` and `create_admin` in the release workflow. Contains the wizard logic AND the `a25` switchover subcommand (`coterie-provision install` and `coterie-provision switch-stripe-to-live`).
- **Thin bash bootstrap `deploy/provision.sh`** (~50 lines). Its only jobs: check `$EUID -eq 0`, detect Debian via `/etc/os-release`, fetch the latest stable release tag via the GitHub API, download the `coterie-provision-<tag>-x86_64-unknown-linux-musl.tar.gz` asset, verify the SHA256 against the release's `*.sha256` asset, extract `coterie-provision` to `/tmp`, `exec` it with `install` and any pass-through flags. Operator-friendly so it can be `curl | bash` audited at a glance.
- **Curl-runnable from the repo**:
  ```sh
  curl -sfL https://raw.githubusercontent.com/IndustriousKraken/coterie/master/deploy/provision.sh \
      -o /tmp/provision.sh
  bash /tmp/provision.sh
  ```
  Recommend `curl -o + bash` over `curl | bash` so the operator can read it first. Both work.
- **Release workflow updates** (`.github/workflows/release.yml`):
  - Add `coterie-provision` to the build matrix as a third musl-static binary, packaged into its own tarball + SHA256.
  - Tags matching `^v\d+\.\d+\.\d+$` (pure-semver, e.g. `v1.0.0`) are stable releases; anything else (`v1.0.0dev`, `v1.0.0-rc1`, `v1.0.0alpha`, etc.) is published with `prerelease: true`. This makes GitHub's `/releases/latest` endpoint return only stable releases, so the bootstrap's "default to latest stable" semantics and `release-deploy.sh` (no args) both pick safe versions. Operators can still explicitly install prereleases by tag.
- **Interactive prompts** (in the Rust binary, via `inquire` or `dialoguer`):
  - Coterie version to install (default: latest stable per the workflow's prerelease marking; offer a list of the most recent ~5 releases via the GitHub Releases API so the operator can roll forward or back. Filter `prerelease: false` for the default; operators can opt into prereleases by selecting them from the list.)
  - Org name (free text — used in emails, page titles, receipts)
  - Portal domain (e.g. `coterie.example.com`)
  - Marketing domain (optional, e.g. `example.com` — for the second Caddy vhost)
  - Org contact email (for AdminAlert delivery)
  - First admin: email, username, full name, password (entered twice for confirmation)
  - Stripe enable? If yes: publishable key, secret key, webhook signing secret
  - Discord enable? If yes: bot token, guild id, default announcements channel id
  - UniFi enable? If yes: controller URL, username, password
  - Caddy: install + configure? (yes/no)
- **Idempotency**: the binary detects existing state and prompts before clobbering:
  - Existing `/opt/coterie/.env` → ask "overwrite?"
  - Existing admin in DB (via `create_admin` exit code 2) → skip create-admin step, surface the conflict
  - Existing `/etc/caddy/Caddyfile` → diff and ask
  - Existing systemd unit → leave alone (install.sh is idempotent)
- **Steps performed by `coterie-provision install`**:
  1. `apt-get update && apt-get install -y` the required packages (curl, python3, tar, sqlite3, ca-certificates, openssl) plus caddy if chosen.
  2. Shell out to `release-deploy.sh <tag>` (existing infrastructure) for the actual file placement of `coterie` / `create_admin`.
  3. Generate `/opt/coterie/.env` from `/opt/coterie/.env.example` + the collected inputs. Session secret via `rand` crate (64 hex chars).
  4. Write the admin password to a `tempfile`-managed file with mode 0600, invoke `/opt/coterie/create_admin --password-file <path> --email ... --username ... --full-name ...`, then `shred -u` regardless of create_admin's exit code (RAII drop guard).
  5. If Caddy chosen: write `/etc/caddy/Caddyfile` from `Caddyfile.example` with domain substitution, `mkdir -p /var/log/caddy && chown -R caddy:caddy /var/log/caddy`, `caddy validate && systemctl reload caddy`.
  6. `systemctl enable --now coterie`.
  7. Smoke test: `GET http://127.0.0.1:8080/health` — expect 200 + the health JSON (not a 303 to /setup, because the admin already exists).
  8. Final summary: print the portal URL, admin email, next steps (point DNS at the box, register Stripe webhook, etc.).
- **Non-interactive mode** (for IaC and re-provisioning): all prompts have a matching env var (e.g. `COTERIE_PROVISION_ORG_NAME`, `COTERIE_PROVISION_ADMIN_EMAIL`, `COTERIE_PROVISION_STRIPE_SK`) AND a matching `--flag` argument. If every required input is supplied via env or flag, the binary runs without prompts. Mixed mode (some env vars set, others prompted) also works.
- **Bonus**: `coterie-provision install --dry-run` (or `--plan`) prints the steps it would take without executing anything. Useful for review before a real run.

## Capabilities

### New Capabilities
- `provisioning-wizard`: a Rust binary that takes a fresh Debian 13 box from clean to "Coterie running with TLS and a first admin, with optional Stripe/Discord/UniFi configured." Distributed via the release tarball; bootstrapped by a thin bash script for the chicken-and-egg "operator has nothing on the box yet" case.

### Modified Capabilities

(None — the wizard composes existing capabilities, doesn't change them. Modest changes to the deploy docs to point at the wizard as the primary path.)

## Impact

- **Code**:
  - New Rust crate `coterie-provision` (~1500–2500 LOC across the wizard logic, switchover logic, prompts, pure-function modules, and integration tests). Standard Rust toolchain — `cargo test`, `cargo clippy`, `cargo fmt`.
  - Thin bash bootstrap `deploy/provision.sh` (~50 lines, includes `set -euo pipefail`, GitHub API lookup, asset download, SHA256 verification, extract, exec).
  - Release workflow gets a third build target + tarball + SHA256 sidecar for `coterie-provision`.
  - Small updates to `DEPLOY-DIGITALOCEAN.md` to point at the wizard as the primary path and demote the manual flow to "the long way / debug mode." `README.md`'s deploy section recommends the wizard as the primary path.
- **Wire shape**: no runtime change to Coterie. The wizard just automates what an operator would do manually.
- **Dependency**: depends on `create-admin-cli` (`a23`) being shipped so `create_admin` is in the release tarball. The wizard fails fast if the binary isn't present.
- **Tests**:
  - Pure-function modules (`.env` generation, Caddyfile substitution, prefix validation, version-list parsing) covered by `cargo test` with golden snapshots.
  - Integration paths (apt-get, systemctl, create_admin invocation, curl) wrapped behind `SystemCommand` and `FileSystem` traits with both real and fake implementations; integration tests use the fakes.
  - `cargo test`, `cargo clippy --deny warnings`, `cargo fmt --check` are the autocoder's validation gates — all available in the sandbox.
  - VM-level smoke test (curl + bash on a fresh Debian box) is operator-side, documented in the PR template, not a task the autocoder claims to complete.
- **Risk**: medium. New crate + new release artifact + new install path. Mitigations: trait-based separation between pure logic (tested) and side-effecting calls (mocked in tests, real in prod); `--dry-run` mode; idempotency checks; `uninstall.sh` for recovery; PR includes operator smoke instructions.
- **Documentation**: the wizard's source IS the documentation, in the sense that reading the Rust code tells you what a Coterie deploy needs. Reduces the docs-vs-code drift surface.
