# syntax=docker/dockerfile:1.7
#
# Coterie production image.
#
# Three stages:
#   1. css-builder  — downloads Tailwind CLI and produces static/style.css
#   2. rust-builder — compiles the release binary, with a dependency
#                     caching layer so day-to-day rebuilds skip the
#                     dependency compile (Rust's slow part)
#   3. runtime      — debian-slim with ca-certificates, non-root user,
#                     binary + static assets only
#
# Build:   docker build -t coterie:latest .
# Run:     docker run --rm -p 8080:8080 \
#            --env-file .env \
#            -v coterie-data:/data \
#            coterie:latest
#
# The container expects:
#   - a writable volume mounted at /data (SQLite file + uploads)
#   - .env (or equivalent envs) with COTERIE__* settings
# It does NOT do TLS — put Caddy / a load balancer in front.

# ---------------------------------------------------------------------
# Stage 1: Tailwind CSS
# ---------------------------------------------------------------------
FROM alpine:3.20 AS css-builder
ARG TAILWIND_VERSION=3.4.17
ARG TARGETARCH

RUN apk add --no-cache curl ca-certificates

# Map docker buildx TARGETARCH to Tailwind's release naming.
# linux/amd64 → linux-x64; linux/arm64 → linux-arm64.
RUN set -eux; \
    case "$TARGETARCH" in \
        amd64) TW_ARCH=linux-x64 ;; \
        arm64) TW_ARCH=linux-arm64 ;; \
        *) echo "Unsupported TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    curl -fsSL -o /usr/local/bin/tailwindcss \
        "https://github.com/tailwindlabs/tailwindcss/releases/download/v${TAILWIND_VERSION}/tailwindcss-${TW_ARCH}"; \
    chmod +x /usr/local/bin/tailwindcss

WORKDIR /work
COPY tailwind.config.js ./
COPY static/input.css static/input.css
COPY templates ./templates
RUN tailwindcss -i static/input.css -o static/style.css --minify

# ---------------------------------------------------------------------
# Stage 2: Rust build
# ---------------------------------------------------------------------
FROM rust:1.83-bookworm AS rust-builder

# Coterie is fully rustls (sqlx, reqwest, lettre, async-stripe all
# configured to use rustls + ring). No system OpenSSL link, so the
# build needs nothing beyond what the base image already provides.
# pkg-config still useful for future deps that probe for system libs.
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# --- Dependency cache layer ---
# Copy ONLY manifest + lockfile, fabricate the source tree the manifest
# refers to, build it. This produces a layer with all crates.io deps
# compiled. Subsequent rebuilds reuse it as long as Cargo.toml /
# Cargo.lock don't change.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src src/bin \
    && echo "fn main() {}" > src/main.rs \
    && echo "" > src/lib.rs \
    && echo "fn main() {}" > src/bin/seed.rs \
    && cargo build --release --bin coterie \
    && rm -rf src

# --- Real build ---
COPY src ./src
COPY migrations ./migrations
COPY templates ./templates
# Touch entrypoints so cargo doesn't think they're stale-but-cached
RUN touch src/main.rs src/lib.rs src/bin/seed.rs \
    && cargo build --release --bin coterie

# ---------------------------------------------------------------------
# Stage 3: Runtime
# ---------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        tini \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 coterie \
    && useradd --system --uid 10001 --gid coterie --no-create-home \
        --home-dir /app --shell /usr/sbin/nologin coterie \
    && mkdir -p /app /data \
    && chown coterie:coterie /app /data

WORKDIR /app

COPY --from=rust-builder --chown=coterie:coterie /build/target/release/coterie /app/coterie
COPY --from=css-builder --chown=coterie:coterie /work/static /app/static

USER coterie

# Default data directory inside the container. The startup code
# auto-detects this via /.dockerenv / CONTAINER, but pin it explicitly
# so behaviour matches whether the host uses /.dockerenv or not (e.g.
# Podman rootless writes a different sentinel).
ENV CONTAINER=1
ENV COTERIE__SERVER__DATA_DIR=/data
ENV COTERIE__SERVER__HOST=0.0.0.0

VOLUME ["/data"]
EXPOSE 8080

# tini reaps zombies and forwards signals so SIGTERM from the
# orchestrator gets a graceful shutdown instead of a kill -9.
ENTRYPOINT ["/usr/bin/tini", "--", "/app/coterie"]
