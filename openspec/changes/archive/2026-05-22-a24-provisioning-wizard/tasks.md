## 1. Skeleton + framing

- [x] 1.1 Create `deploy/provision.sh` with the shebang `#!/usr/bin/env bash` and `set -euo pipefail`. Add the `ERR` trap pointing operators at `uninstall.sh` for recovery.
- [x] 1.2 Define helper functions at the top: `prompt(name, message, default)` — checks env var first, returns its value if set, otherwise prompts. `prompt_secret(name, message)` — same but uses `read -sp` and confirms by re-prompting. `prompt_yn(name, message, default)` — y/n with default. `info(msg)`, `warn(msg)`, `die(msg, exit_code)` for output.
- [x] 1.3 Parse top-level flags: `--dry-run`, `--help`. `--help` prints the env-var list + a brief description of what each does. `--dry-run` sets a global flag the rest of the script checks before executing.

## 2. Preflight checks

- [x] 2.1 Refuse to run if not root (`[ $EUID -ne 0 ] && die ...`).
- [x] 2.2 Refuse to run on non-Debian (check `/etc/os-release` for `ID=debian`). Print a hint about other deploy guides for non-Debian hosts.
- [x] 2.3 Warn (don't refuse) if `/var/lib/coterie` is a directory but not a mount point — operator might have forgotten to mount their block volume. Continue anyway.
- [x] 2.4 Idempotency detection: check for existing `/opt/coterie/.env`, existing `members.is_admin = 1` in DB, existing `/etc/caddy/Caddyfile`. For each, set a flag the relevant step consults.

## 3. Collect inputs

- [x] 3.1 Prompt for org name (`COTERIE_PROVISION_ORG_NAME`).
- [x] 3.2 Prompt for portal domain (`COTERIE_PROVISION_PORTAL_DOMAIN`).
- [x] 3.2a Version selection: fetch `/repos/IndustriousKraken/coterie/releases?per_page=10` via curl + Python parse (same pattern as release-deploy.sh). Filter into two lists — stable (`prerelease == false`) and all-releases. Default is the newest stable tag. Present a numbered menu: top ~5 stable releases + an option "show all releases (including prereleases)". Env-var override: `COTERIE_PROVISION_VERSION=<tag>` skips the prompt and uses the tag verbatim. The chosen tag is later passed to `release-deploy.sh` (task 5.2).
- [x] 3.3 Prompt for optional marketing domain (`COTERIE_PROVISION_MARKETING_DOMAIN`).
- [x] 3.4 Prompt for org contact email (`COTERIE_PROVISION_CONTACT_EMAIL`).
- [x] 3.5 Admin credentials: email, username, full name, password (twice for confirmation). Skip the prompts if `COTERIE_PROVISION_ADMIN_*` env vars are set.
- [x] 3.6 Stripe enable? (default no). If yes, prompt for publishable key, secret key, webhook secret.
- [x] 3.7 Discord enable? (default no). If yes, prompt for bot token, guild id, announcements channel id.
- [x] 3.8 UniFi enable? (default no). If yes, prompt for controller URL, username, password.
- [x] 3.9 Caddy install? (default yes). If yes, the Caddyfile generation step will run later.
- [x] 3.10 Display a summary of what's about to happen + a final y/N confirmation prompt before any modifications start. In dry-run mode, print the summary and exit.

## 4. Install system dependencies

- [x] 4.1 `apt-get update`.
- [x] 4.2 `apt-get install -y --no-install-recommends curl python3 tar sqlite3 ca-certificates openssl` (the always-needed set).
- [x] 4.3 If Caddy chosen: add the Caddy apt repo (Cloudsmith), `apt-get update`, `apt-get install -y caddy`.

## 5. Pull and install Coterie

- [x] 5.1 Curl `release-deploy.sh` from the master branch of the GitHub repo into `/usr/local/bin/coterie-release-deploy`, chmod +x.
- [x] 5.2 Run `/usr/local/bin/coterie-release-deploy "$SELECTED_VERSION"` where `$SELECTED_VERSION` comes from task 3.2a (or empty for "latest" — release-deploy.sh accepts both shapes per its existing arg handling).
- [x] 5.3 After release-deploy completes, verify `/opt/coterie/coterie` and `/opt/coterie/create_admin` exist and are executable.

## 6. Generate .env

- [x] 6.1 Read `/opt/coterie/.env.example` as the template.
- [x] 6.2 Generate a session secret via `openssl rand -hex 32`. Substitute into the template.
- [x] 6.3 Substitute the org name, contact email, portal domain (as `CORS_ORIGINS` source for the marketing domain if any), Stripe/Discord/UniFi credentials as collected.
- [x] 6.4 If an integration was NOT enabled, leave its block as `_ENABLED=false` with credential lines commented out (preserving the .env.example structure).
- [x] 6.5 Write the result to `/opt/coterie/.env` with `chown coterie:coterie` and `chmod 0640`.
- [x] 6.6 If `/opt/coterie/.env` already exists from a prior run, prompt before clobbering (or skip if non-interactive and the env var `COTERIE_PROVISION_OVERWRITE_ENV=true` is set).

## 7. Bootstrap the first admin

- [x] 7.1 Create a tempfile via `mktemp` with mode 0600. Write the admin password to it.
- [x] 7.2 Invoke `/opt/coterie/create_admin --email "$ADMIN_EMAIL" --username "$ADMIN_USERNAME" --full-name "$ADMIN_FULL_NAME" --password-file "$TMPFILE"`.
- [x] 7.3 In a `finally`-style trap, `shred -u "$TMPFILE"` regardless of exit code.
- [x] 7.4 If create_admin returned exit code 2 (admin already exists), print a warning and continue — the admin exists from a prior run.
- [x] 7.5 Any other non-zero exit code: print create_admin's stderr and die.

## 8. Configure Caddy (if chosen)

- [x] 8.1 Read `/opt/coterie/deploy/Caddyfile.example`.
- [x] 8.2 Substitute `coterie.example.com` with the operator's portal domain.
- [x] 8.3 If a marketing domain was supplied, substitute `example.com, www.example.com` with `${MARKETING_DOMAIN}, www.${MARKETING_DOMAIN}` in the second site block. If no marketing domain, remove the entire second site block.
- [x] 8.4 `mkdir -p /var/log/caddy && chown -R caddy:caddy /var/log/caddy` BEFORE writing the Caddyfile (this was a real bug we hit manually).
- [x] 8.5 Write the substituted result to `/etc/caddy/Caddyfile`.
- [x] 8.6 `caddy validate --config /etc/caddy/Caddyfile`. On failure, print the validation error and die.
- [x] 8.7 `systemctl reload caddy` (or `systemctl restart caddy` if Caddy isn't yet running).
- [x] 8.8 Verify Caddy is active: `systemctl is-active caddy`. If not active, die with a hint about checking journalctl.

## 9. Start Coterie

- [x] 9.1 `systemctl enable --now coterie`.
- [x] 9.2 Wait up to 30 seconds for the service to come up: poll `systemctl is-active coterie` once per second.
- [x] 9.3 If after 30 seconds it's not active, dump `journalctl -u coterie -n 50 --no-pager` and die.

## 10. Smoke test

- [x] 10.1 `curl -sf http://127.0.0.1:8080/health` and assert the response is 200 with a JSON body (not a 303 to /setup, which would indicate the admin row wasn't found).
- [x] 10.2 If Caddy was configured, also `curl -k https://127.0.0.1/health` (the `-k` because the SNI hostname mismatch makes the cert untrusted on localhost, but the proxy chain works).
- [x] 10.3 If either smoke test fails, dump diagnostics + die.

## 11. Final summary

- [x] 11.1 Print the success summary block per design.md D10. Include the portal URL, admin email, service status, and next steps (DNS pointing, Stripe webhook registration if applicable, login).

## 11a. Mark dev tags as prereleases in the workflow

- [x] 11a.1 Edit `.github/workflows/release.yml`. Compute `prerelease` based on the tag name. Add a step that sets an output variable:
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
- [x] 11a.2 In the `softprops/action-gh-release` step, add `prerelease: ${{ steps.prerelease.outputs.prerelease }}` to honor the classification.
- [x] 11a.3 This affects FUTURE releases only. Existing tags (`v1.0.0dev` etc.) keep their existing classification unless manually updated in the GitHub Releases UI.

## 12. Documentation

- [x] 12.1 Update `deploy/DEPLOY-DIGITALOCEAN.md`: replace step 5 (current happy path) with a pointer to `provision.sh`. Demote the manual flow (build-from-source, rsync, install.sh) to "what the wizard does internally" or "manual fallback if you want to understand each step."
- [x] 12.2 Update `README.md`: the deployment section SHALL recommend `provision.sh` as the primary/default path. Include the curl-and-bash one-liner inline. Any pre-existing manual-deploy instructions are demoted below the wizard (under a heading like "Manual deploy (advanced)") or replaced with a link to `DEPLOY-DIGITALOCEAN.md`. The wizard should be the first thing a new operator sees in the README's deploy section.
- [x] 12.3 Keep `STRIPE-SETUP.md`, `uninstall.sh` documentation in place — the wizard composes them.

## 13. Validate

- [x] 13.1 `bash -n deploy/provision.sh` — syntax check.
- [x] 13.2 `shellcheck deploy/provision.sh` if available. Address any warnings (some may be ignorable, document why if so).
- [ ] 13.3 Manual smoke: spin up a fresh Debian 13 droplet (or local Docker container with systemd + apt). Run the wizard interactively. Confirm it completes and Coterie is reachable.
- [ ] 13.4 Re-run on the same box to test idempotency: every step should either skip (because already done) or prompt before overwriting.
- [ ] 13.5 Test `--dry-run`: confirm no side effects, output is the plan.
- [ ] 13.6 Test non-interactive: export all env vars, run, confirm no prompts.
- [ ] 13.7 Test recovery: provision → `uninstall.sh --all` → provision again. Should succeed both times.
