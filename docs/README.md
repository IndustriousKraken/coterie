# Coterie documentation

The project overview and quick install live in the [root README](../README.md).
This directory holds everything else.

## Deploy & operate

Standing an instance up and keeping it running:

- [deploy/DEPLOY-DIGITALOCEAN.md](deploy/DEPLOY-DIGITALOCEAN.md) — end-to-end DigitalOcean droplet (Ubuntu, ~45 min)
- [deploy/DEPLOY-AWS.md](deploy/DEPLOY-AWS.md) — EC2 + EBS or Lightsail (Ubuntu)
- [deploy/DEPLOY-ALPINE.md](deploy/DEPLOY-ALPINE.md) — Alpine Linux + OpenRC (no Docker required)
- [deploy/STRIPE-SETUP.md](deploy/STRIPE-SETUP.md) — wiring Coterie to a Stripe account
- [deploy/OPS.md](deploy/OPS.md) — operations reference: secret rotation, logs, upgrades, routine maintenance
- [deploy/MIGRATION.md](deploy/MIGRATION.md) — moving an instance between hosts
- [deploy/RESTORE.md](deploy/RESTORE.md) — restoring from a backup
- [deploy/SETUP.md](deploy/SETUP.md) — staging environment via GitHub Actions

> Deploy artifacts (scripts, `Caddyfile.example`, systemd/OpenRC units, the
> `coterie-provision` source) live in the repo's [`deploy/`](../deploy/)
> directory, next to the binary they configure — not here.

## Understand the system

- [ARCHITECTURE.md](ARCHITECTURE.md) — the three HTTP surfaces, the security model, and how resilience is achieved
- [../CLAUDE.md](../CLAUDE.md) — working in this repo: the secure-by-default routing rules and conventions (read before adding routes or handlers)

## What's planned

- [ROADMAP.md](ROADMAP.md) — tiered priorities for what to build next
- [TODO.md](TODO.md) — the raw open-items list

## Design history

- [PLAN-stripe-billing.md](PLAN-stripe-billing.md) — the Stripe billing overhaul plan (implemented; kept for design rationale)
