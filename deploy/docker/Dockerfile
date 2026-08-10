# syntax=docker/dockerfile:1
#
# Multi-stage, multi-target build for the four pure-Rust network services in
# this workspace: subscription, rendezvous, relay-agent, test-service.
#
# Deliberately NOT included here: sing-box (external binary, not built from
# this source), WireGuard, Outline/shadowbox — none of these exist in this
# repository (see docs/DEPLOYMENT.md "What is and isn't containerized").
# vpn-admin (apps/admin) is also not built here: it manages host state
# (systemctl, /etc/vpn/compat) directly and containerizing it would only
# add indirection without any isolation benefit.
#
# Build one target at a time from the REPO ROOT as build context (the
# workspace's Cargo.lock/members span the whole tree):
#   docker build -f deploy/docker/Dockerfile --target subscription -t vpn1/subscription .
#   docker build -f deploy/docker/Dockerfile --target rendezvous   -t vpn1/rendezvous   .
#   docker build -f deploy/docker/Dockerfile --target relay-agent  -t vpn1/relay-agent  .
#   docker build -f deploy/docker/Dockerfile --target test-service -t vpn1/test-service .
# or use deploy/docker/docker-compose.yml, which does this for you.

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
RUN cargo chef cook --release \
      -p subscription -p rendezvous -p relay-agent -p test-service \
      --recipe-path recipe.json
COPY . .
RUN cargo build --release \
      -p subscription -p rendezvous -p relay-agent -p test-service

# Shared runtime base: non-root user, curl+netcat for HEALTHCHECKs (these
# services don't otherwise need a shell or userspace tools — kept minimal
# rather than distroless specifically so HEALTHCHECK has something to run).
FROM debian:12-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241 AS runtime-base
RUN apt-get update && apt-get install -y --no-install-recommends \
      curl \
      netcat-openbsd \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 vpnsvc \
    && useradd --system --uid 10001 --gid vpnsvc --no-create-home --shell /usr/sbin/nologin vpnsvc

FROM runtime-base AS subscription
COPY --from=builder /app/target/release/subscription /usr/local/bin/subscription
USER vpnsvc
EXPOSE 9100
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -fsS http://127.0.0.1:9100/healthz || exit 1
ENTRYPOINT ["/usr/local/bin/subscription"]
CMD ["--config", "/etc/vpn/deployment.toml"]

FROM runtime-base AS rendezvous
COPY --from=builder /app/target/release/rendezvous /usr/local/bin/rendezvous
USER vpnsvc
EXPOSE 9000
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -fsS http://127.0.0.1:9000/healthz || exit 1
ENTRYPOINT ["/usr/local/bin/rendezvous"]
CMD ["--bind", "0.0.0.0:9000"]

FROM runtime-base AS relay-agent
COPY --from=builder /app/target/release/relay-agent /usr/local/bin/relay-agent
USER vpnsvc
# No HTTP surface to probe (it's a raw TLS/QUIC forwarder, not a web
# service) — a bare TCP-connect check against the configured TLS port is
# the best available signal. RELAY_TLS_PORT must match --bind-tls's port;
# left unset, HEALTHCHECK degrades to "process still running" (nc against
# port 0 fails fast, matching Docker's own default when no check applies).
ENV RELAY_TLS_PORT=""
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD [ -z "$RELAY_TLS_PORT" ] || nc -z 127.0.0.1 "$RELAY_TLS_PORT" || exit 1
ENTRYPOINT ["/usr/local/bin/relay-agent"]

FROM runtime-base AS test-service
COPY --from=builder /app/target/release/test-service /usr/local/bin/test-service
USER vpnsvc
EXPOSE 8081
ENV TEST_SERVICE_BIND=0.0.0.0:8081
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8081/ || exit 1
ENTRYPOINT ["/usr/local/bin/test-service"]
