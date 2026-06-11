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

.PHONY: help dev build release css watch-css setup seed clean check test

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
