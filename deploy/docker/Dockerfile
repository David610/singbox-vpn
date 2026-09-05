# syntax=docker/dockerfile:1
#
# Builds the `subscription` service as an optional containerized
# alternative to running it natively via systemd
# (deploy/almalinux/install.sh).
#
# Deliberately NOT included here: sing-box (external binary, not built from
# this source), WireGuard, Outline/shadowbox — none of these exist in this
# repository. vpn-admin (apps/admin) is also not built here: it manages
# host state (systemctl, /etc/vpn/compat) directly and containerizing it
# would only add indirection without any isolation benefit.
#
#   docker build -f deploy/docker/Dockerfile --target subscription -t singbox-vpn/subscription .
# or use deploy/docker/docker-compose.yml, which does this for you.
#
# (This used to be a multi-target build also producing the native
# adaptive-transport stack's rendezvous/relay-agent/test-service images.
# That stack was removed from `main` entirely — see
# docs/SUPPORTED_PRODUCT.md and the archive/native-adaptive-stack-2026
# branch, which still has the original multi-target Dockerfile.)

# Pinned by digest, not a floating tag — matches rust-toolchain.toml's
# channel = "1.94.1" so debug_assertions/lints behave the same as local
# `cargo build`, and the runtime base below is the same Debian release so
# glibc versions match (no dynamic-link surprises at runtime).
FROM rust:1.94.1-slim-bookworm@sha256:cf9dd0ec73e75f827fe59123fff9dc65af1a1c8363c3c31ee8d7f8ad0b6a5fb2 AS chef
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential \
    && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef --locked --version ^0.1

# Compute the dependency "recipe" from manifests only, so the expensive
# `cargo chef cook` layer below is cache-keyed on Cargo.toml/Cargo.lock and
# survives source-only edits.
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release -p subscription --recipe-path recipe.json
COPY . .
RUN cargo build --release -p subscription

# Non-root user, curl for the HEALTHCHECK (the service doesn't otherwise
# need a shell or userspace tools — kept minimal rather than distroless
# specifically so HEALTHCHECK has something to run).
FROM debian:12-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241 AS subscription
RUN apt-get update && apt-get install -y --no-install-recommends \
      curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 vpnsvc \
    && useradd --system --uid 10001 --gid vpnsvc --no-create-home --shell /usr/sbin/nologin vpnsvc
COPY --from=builder /app/target/release/subscription /usr/local/bin/subscription
USER vpnsvc
EXPOSE 9100
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -fsS http://127.0.0.1:9100/healthz || exit 1
ENTRYPOINT ["/usr/local/bin/subscription"]
CMD ["--config", "/etc/vpn/deployment.toml"]
