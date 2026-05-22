## Context

After this session's deploy walkthrough, the substrate for an automated wizard is mostly in place:

- `release-deploy.sh` — fetches a tagged GitHub Release, places files, runs install.sh if first install, restarts service on update
- `install.sh` — creates the `coterie` user, dirs, systemd unit (idempotent)
- `uninstall.sh` — recovery path with `--data` and `--all` modes
- `create_admin` (separate spec, prerequisite) — CLI for the first admin
- `Caddyfile.example` — template with the right structure, just needs domain + log-dir setup
- `STRIPE-SETUP.md` — operator-facing reference for the secrets the wizard collects

The wizard is the conductor: it asks the operator the org-specific questions, then orchestrates all of the above in the right order. It's a curl-and-bash entrypoint because every Debian box has curl + bash; requiring Python or Ruby or Node would be extra friction we don't need.

## Goals / Non-Goals

**Goals:**
- A single `curl -o /tmp/provision.sh && bash /tmp/provision.sh` brings a fresh Debian 13 box to "Coterie running, admin created, TLS provisioned (if Caddy chosen), .env populated, ready for traffic."
- Interactive prompts for org-specific values; non-interactive mode via env vars for IaC.
- Idempotent — re-running detects state and prompts about overwrites.
- `--dry-run` flag prints the planned actions without executing.
- Useful enough that we'd recommend it as the primary path in the deploy doc, with manual deploy retained only for "I want to understand the steps."

**Non-Goals:**
- Provisioning the droplet/VM itself. The operator provisions the box (DO, AWS, bare metal); the wizard runs on it.
- DNS configuration. The operator points DNS at the box; the wizard waits for it (TLS provisioning will retry naturally).
- Volume mounting. If the operator has a separate block volume, they mount it at `/var/lib/coterie` before running the wizard. The wizard detects whether that mountpoint exists and warns if not.
- Multi-host setups. The wizard targets single-VM deploys (which is what Coterie supports).
- Updates after first deploy. `release-deploy.sh` handles updates; the wizard is bootstrap-only.
- Distros other than Debian 13. The wizard assumes `apt`, `systemd`, paths like `/etc/caddy/`, etc. An Alpine-flavored variant is a separate concern.

## Decisions

### D1. Pure bash, no exotic deps

Bash + standard tools (curl, openssl, awk, sed, sqlite3) only. No `whiptail`, `dialog`, Python `prompt-toolkit`, etc. The UX is plain `read -p` prompts. Less pretty than a TUI, but works on any minimal Debian without `apt install`-ing UI libraries.

If we ever want a richer experience, a follow-up Rust-based `coterie provision` subcommand could replace this — but that requires Coterie's binary to exist before bootstrap, which is exactly what this script avoids.

### D2. The script is `set -euo pipefail` with explicit error trapping

Every `cmd` exits the script on non-zero (no silent failures). An `ERR` trap prints the failing command + line number plus a "what to do next" hint pointing at `uninstall.sh`:

```sh
trap 'on_error $? $LINENO' ERR
on_error() {
    echo "Provision failed: exit $1 at line $2"
    echo "Recovery: bash /opt/coterie/deploy/uninstall.sh"
    exit "$1"
}
```

### D3. Interactive + non-interactive modes share the same code path

Every prompt checks for a pre-set env var first; only prompts if unset. This makes IaC-friendly automation natural:

```sh
COTERIE_PROVISION_ORG_NAME="Neon Temple" \
COTERIE_PROVISION_PORTAL_DOMAIN="coterie.theneontemple.com" \
COTERIE_PROVISION_ADMIN_EMAIL="rab@theneontemple.com" \
... \
bash provision.sh
```

The script enumerates the env vars at the start so operators can grep them out for their IaC tooling.

### D3a. Version selection — default to latest stable, allow rollback

Before any install steps, the wizard fetches `/repos/IndustriousKraken/coterie/releases?per_page=10` from the GitHub API. From the response, it builds two lists:

- **Stable releases** — those with `prerelease: false`. Sorted newest-first.
- **All releases** — sorted newest-first (includes prereleases marked as dev/alpha/beta/rc).

Default: the newest stable release's tag (e.g. `v1.0.0`). The prompt presents the top ~5 stable releases as a numbered menu, plus an option to "see all releases including prereleases" for operators who specifically want a dev version (e.g. installing a not-yet-promoted bugfix).

Env-var equivalent: `COTERIE_PROVISION_VERSION=v1.0.0`. If set, the wizard uses it verbatim with no prompt.

Once selected, the tag is passed to `release-deploy.sh <TAG>` for the actual install.

This pairs with the workflow change (proposal): tags ending in non-numeric suffixes are published as prereleases, so the GitHub API correctly classifies them. Without the workflow change, every dev tag would show up in the "stable" list.

### D4. Password handling

The admin password is:
1. Prompted via `read -sp` (no echo) twice for confirmation.
2. Written to `$(mktemp /tmp/coterie-bootstrap.XXXXXX.pw)` with `chmod 600`.
3. Passed to `create_admin --password-file <path>`.
4. `shred -u <path>` immediately after, regardless of create_admin's exit code.

The password never appears in:
- The process listing (would happen if we passed `--password "..."`)
- The shell history (the heredoc / read trick keeps it out of HISTFILE)
- Any log file (we don't echo it)
- The .env file (it's not an env var; it's hashed into the DB by create_admin)

For non-interactive mode, the operator provides `COTERIE_PROVISION_ADMIN_PASSWORD` as an env var. Documented warning: env vars are visible to other users via `/proc/<pid>/environ` if your script is readable. Acceptable for CI where the box is single-user; the operator should know what they're getting into.

### D5. .env generation

The wizard's .env template is embedded in the script as a heredoc, with `${VAR}` substitution. The template lives alongside `.env.example` (with the same field set + same comments), so a single source-of-truth for .env structure. The wizard's template SHALL be regenerated from `.env.example` programmatically — i.e., the wizard reads `/opt/coterie/.env.example` (which it just installed via `release-deploy.sh`) and produces `.env` by substituting values for the known keys.

Decision: the wizard works from `.env.example` shipped in the release tarball. This keeps the wizard's .env shape in lock-step with whatever release version the operator is installing, even if `.env.example` evolves over time.

### D6. Session secret generation

`COTERIE__AUTH__SESSION_SECRET=$(openssl rand -hex 32)` — generated once at provision time, embedded in `.env`. The operator never sees it (it's a 64-character hex blob); it just lives in `.env` until the operator chooses to rotate it.

Same for any other secret that should be generated rather than supplied (currently only the session secret; if Coterie ever grows others, they're added to this section).

### D7. Caddyfile generation when Caddy is chosen

The wizard reads `/opt/coterie/deploy/Caddyfile.example` and substitutes:

- `coterie.example.com` → the operator's portal domain
- `example.com, www.example.com` → the operator's marketing domain (or removes the block if not specified)

Then:
- `mkdir -p /var/log/caddy && chown -R caddy:caddy /var/log/caddy` (the bug we just fixed manually)
- `caddy validate --config /etc/caddy/Caddyfile`
- `systemctl reload caddy`

If `caddy validate` fails, the wizard prints the validation error and exits — operator fixes the Caddyfile manually and re-runs, or skips Caddy.

### D8. Stripe / Discord / UniFi conditional config

Each integration has an "enable?" prompt. If yes, the integration's env vars are prompted and written to `.env`. If no, the wizard writes the `ENABLED=false` line and leaves the value lines commented out (preserving the .env.example structure).

For Stripe specifically, the wizard prints a one-liner at the end pointing at `deploy/STRIPE-SETUP.md` to walk the operator through the dashboard side of webhook registration. The wizard collects the keys but can't talk to Stripe's API on the operator's behalf without their authenticated dashboard session.

### D9. Final smoke test

After everything's running, the wizard runs:

```sh
curl -sf http://127.0.0.1:8080/health | head -c 200
```

Expected: a 200 with JSON. Crucially, NOT a 303 to /setup — because the admin already exists from the create_admin step, the setup-redirect middleware forwards normally.

If the smoke test fails (no response, or a redirect, or anything but 200), the wizard prints a diagnostic block:

- Show last 50 lines of `journalctl -u coterie --no-pager`
- Show `systemctl status coterie --no-pager`
- Suggest manual debugging steps

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

## Risks / Trade-offs

- **Risk**: bash is fragile for complex flow control. Edge cases in input validation, error handling, idempotency checks can hide bugs. → Mitigation: `--dry-run` for review, extensive testing on a fresh VM before promoting to "recommended path" in docs.
- **Risk**: the wizard becomes a maintenance burden as Coterie's config evolves. → Mitigation: the wizard works from `.env.example` shipped in the release tarball — when `.env.example` changes, the wizard's behavior updates automatically (assuming new fields are optional or the wizard learns to handle them in the same release).
- **Risk**: operator runs the wizard partway, it fails, leaves the box in a half-configured state. → Mitigation: idempotency checks let the operator re-run safely; if state is genuinely confused, `uninstall.sh --data` and re-provision is a clear recovery path.
- **Risk**: prompts ambiguous or confusing on first run. → Mitigation: every prompt has a one-line explanation + an example + a default if applicable. Test on people who've never deployed Coterie before.
- **Trade-off**: the script duplicates some knowledge from `DEPLOY-DIGITALOCEAN.md` (the `chown -R /var/log/caddy` step, etc.). When something changes, both have to update. Mitigation: the wizard is the source of truth; the doc becomes a high-level walkthrough that says "here's what happens; the wizard does it for you."

## Migration Plan

Single PR.

1. Write `deploy/provision.sh` with the prompts, idempotency checks, integration enablement, etc.
2. Update `DEPLOY-DIGITALOCEAN.md` to make `provision.sh` the primary path; demote the manual section to "what the wizard does, step by step."
3. Test on a fresh Debian 13 VM (DO droplet or local VM). Run through interactively. Then run again to test idempotency. Then run with `--dry-run`.
4. Test non-interactive mode by populating every env var and running. Confirm no prompts appear.
5. Test recovery: provision, then `uninstall.sh`, then provision again. Should succeed each time.
6. Update the README to mention "5-minute deploy" with a link to the wizard.
