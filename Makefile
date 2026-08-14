# Coterie Build System
# Run `make help` for available targets.

TAILWIND_VERSION := 3.4.17
TAILWIND_BIN     := ./tailwindcss
UNAME_S          := $(shell uname -s)
UNAME_M          := $(shell uname -m)

# Detect platform for Tailwind CLI download
ifeq ($(UNAME_S),Darwin)
  ifeq ($(UNAME_M),arm64)
    TAILWIND_PLATFORM := macos-arm64
  else
    TAILWIND_PLATFORM := macos-x64
  endif
else
  ifeq ($(UNAME_M),aarch64)
    TAILWIND_PLATFORM := linux-arm64
  else
    TAILWIND_PLATFORM := linux-x64
  endif
endif

TAILWIND_URL := https://github.com/tailwindlabs/tailwindcss/releases/download/v$(TAILWIND_VERSION)/tailwindcss-$(TAILWIND_PLATFORM)

.PHONY: help dev build release css watch-css setup seed clean check test audit

# ---------------------------------------------------------------------------
# Advisory waivers (see the `audit` target)
#
# One entry per RustSec identifier, waived individually — no wildcard, no
# crate-wide suppression. A finding belongs here only when there is no fixed
# version to move to and no way to stop building the code; anything else gets
# fixed instead. RUSTSEC-2026-0213 (ammonia XSS via SVG `animate`/`set`) is not
# listed because it was fixed by upgrading to ammonia 4.1.4.
#
# RUSTSEC-2023-0071 — rsa 0.9.10, Marvin timing sidechannel. No fixed version
#   exists upstream. Not reachable: `rsa` comes from `sqlx-mysql`, and this
#   application is SQLite-only. `sqlx` is declared `default-features = false`
#   (Cargo.toml), so neither the MySQL nor the PostgreSQL driver is compiled —
#   `cargo tree --target all -i rsa -e normal,build` prints nothing. It stays
#   in Cargo.lock anyway: Cargo resolves the lockfile with all features on so
#   the lock is valid under any feature selection, which records unactivated
#   optional dependencies. `cargo audit` reads the lockfile, so the finding
#   outlives a removal that did happen. This waiver covers that gap, not the
#   crate. Revisit 2027-02-13, or sooner if that `cargo tree` ever prints a
#   path — that would mean a driver came back.
#
# RUSTSEC-2026-0104 — rustls-webpki 0.101.7, panic parsing a CRL.
# RUSTSEC-2026-0098 — rustls-webpki 0.101.7, URI name constraints accepted.
# RUSTSEC-2026-0099 — rustls-webpki 0.101.7, name constraints accepted for a
#   certificate asserting a wildcard name. Fixed in >=0.103.12/0.103.13; the
#   0.101 line has no fix. Reached only by the outbound Stripe client:
#   rustls-webpki 0.101.7 <- rustls 0.21.12 <- hyper-rustls 0.24.2 <-
#   async-stripe 0.39.1. The application's own TLS — server, lettre, reqwest,
#   sqlx — is rustls 0.23 on rustls-webpki 0.103.13 and is unaffected. That
#   0.21 stack validates exactly one peer, api.stripe.com, a fixed host with a
#   public-CA certificate; all three advisories concern validation of an
#   attacker-supplied chain, and there is none on that connection. No upgrade
#   exists to take: async-stripe 0.39.1 is the newest stable release and every
#   runtime feature but native-tls (refused — it would put OpenSSL back in the
#   static musl binary) routes through hyper-rustls 0.24. The only newer
#   publication is the 1.0.0-rc line, a pre-release API rewrite; migrating the
#   payment path onto a release candidate is the larger risk. Revisit
#   2027-02-13, or when async-stripe 1.0 ships stable, whichever is first.
# ---------------------------------------------------------------------------
AUDIT_IGNORES := \
  --ignore RUSTSEC-2023-0071 \
  --ignore RUSTSEC-2026-0104 \
  --ignore RUSTSEC-2026-0098 \
  --ignore RUSTSEC-2026-0099

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}'

# ---------------------------------------------------------------------------
# Development
# ---------------------------------------------------------------------------

dev: css ## Build CSS then run the dev server
	cargo run --bin coterie

watch-css: $(TAILWIND_BIN) ## Rebuild CSS on file changes (run in a second terminal)
	$(TAILWIND_BIN) -i static/input.css -o static/style.css --watch

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

css: $(TAILWIND_BIN) ## Build Tailwind CSS (minified)
	$(TAILWIND_BIN) -i static/input.css -o static/style.css --minify

check: css ## Compile-check everything (CSS + Rust)
	cargo check

build: css ## Debug build (CSS + Rust)
	cargo build

release: css ## Release build (CSS + Rust)
	cargo build --release

test: css ## Run tests
	cargo test

# The CI advisory job runs this same target, so a local run and the one that
# gates a pull request read the identical waiver list. Keeping the list here
# rather than duplicating it in the workflow is the only reason they cannot
# drift apart.
audit: ## Check the resolved dependency graph against RustSec
	cargo audit $(AUDIT_IGNORES)

# ---------------------------------------------------------------------------
# Setup & utilities
# ---------------------------------------------------------------------------

$(TAILWIND_BIN): ## Download Tailwind CLI if missing
	@echo "Downloading tailwindcss v$(TAILWIND_VERSION) for $(TAILWIND_PLATFORM)..."
	@curl -sL $(TAILWIND_URL) -o $(TAILWIND_BIN)
	@chmod +x $(TAILWIND_BIN)
	@echo "Done."

setup: $(TAILWIND_BIN) ## First-time setup (download tools, build CSS)
	$(MAKE) css
	@echo ""
	@echo "Setup complete. Copy .env.example to .env and fill in your values, then:"
	@echo "  make dev     - start the development server"
	@echo "  make seed    - populate test data"

seed: ## Seed the database with test data
	cargo run --bin seed

clean: ## Remove build artifacts
	cargo clean
	rm -f static/style.css
	rm -f $(TAILWIND_BIN)
