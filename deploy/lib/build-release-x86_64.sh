#!/usr/bin/env bash
# Single source of truth for the pinned-container x86_64 release build
# (v1.0.0-rc.4 fix). release.yml's "build" job and a cheap pre-tag CI
# smoke job both call this same script instead of maintaining one Docker
# invocation in YAML and a second, divergent copy elsewhere.
#
# rc.4 incident: the build container mounted the HOST's CARGO_HOME
# (`$HOME/.cargo`) read-only and pointed CARGO_HOME at it, so any crate
# not already present in that exact snapshot (a new dependency, or an
# empty/never-populated registry) made `cargo build` fail with
# "Read-only file system (os error 30)" the moment it tried to update the
# crates.io index or download a crate — before a single line of
# production code compiled. All later release stages (ABI check,
# packaging, attestation, runtime-compat, publish) were skipped as a
# result. See docs/RELEASE.md / the "build" job comment in release.yml
# for the full incident writeup.
#
# Fix, matching the "toolchain read-only, Cargo state writable" split:
#   - the pinned Rust toolchain BINARIES (rustc/cargo/rustup under
#     ~/.cargo/bin, installed on the runner by dtolnay/rust-toolchain)
#     are bind-mounted READ-ONLY, at a container path distinct from the
#     host path — rustup's proxies (cargo/rustc are symlinks to the
#     `rustup` binary) resolve the active toolchain via the RUSTUP_HOME
#     env var and settings.toml alone, not via any hardcoded host path,
#     so remapping the mount point is transparent to it.
#   - RUSTUP_HOME itself is bind-mounted read-WRITE, not read-only: a
#     real GitHub Actions run of the --environment-check smoke job below
#     caught rustup writing a temp file under $RUSTUP_HOME/tmp on a
#     plain `rustc --version` (rustup performs an internal channel-
#     manifest consistency check on invocation; a local sandbox re-run
#     against an already-fully-synced RUSTUP_HOME never happened to
#     exercise this path, which is exactly why real CI — not local
#     reasoning — caught it). This is the identical bug class as the
#     rc.4 CARGO_HOME failure, but safe here in a way rc.4's fix was
#     not: the container always runs `--user "<runner uid>:<runner
#     gid>"` (below), the exact UID that already owns RUSTUP_HOME on the
#     host, so a write there is the same user writing to their own
#     directory — never root polluting another user's files. Nothing in
#     the build actually modifies the installed toolchain binaries
#     themselves, only rustup's own bookkeeping.
#   - a dedicated CARGO_HOME, seeded from the host's own (Swatinem/
#     rust-cache-restored) registry when present so a warm cache is
#     never re-downloaded from scratch, is bind-mounted READ-WRITE.
#   - the container runs `dnf install` as root ONCE, inside its own
#     writable container layer (never a bind mount) via `docker
#     create`/`start`/`commit`, producing a local image with build
#     dependencies baked in. The actual compile then runs in a fresh
#     container from that image with `--user "<runner uid>:<runner
#     gid>"` — the same UID/GID that already owns every bind-mounted
#     writable path on the host (the workspace checkout, the seeded
#     Cargo home) because the runner created them. No chown, no setpriv,
#     no root-owned files ever land in $RUNNER_TEMP or the workspace.
#
# Usage:
#   RELEASE_BUILD_IMAGE=... RELEASE_GLIBC_BASELINE=... \
#     bash deploy/lib/build-release-x86_64.sh [--environment-check]
#
# --environment-check: prove the container/toolchain/CARGO_HOME contract
# (pull, toolchain visibility, CARGO_HOME writability, compiler
# availability, GLIBC baseline) in seconds, WITHOUT compiling the
# workspace. Intended for a cheap pre-tag CI smoke job so a broken
# release-container contract fails on every relevant PR instead of only
# being discovered after a tag is cut (the rc.4 process gap: release.yml
# only triggers on tags/dispatch, so this exact container invocation was
# never exercised by PR #53's own CI run).
set -euo pipefail

# ---------------------------------------------------------------------
# Pure contract check (no Docker) — kept separate from the CLI/Docker
# plumbing below, same reasoning as check-glibc-baseline.sh's
# glibc_version_le(): deploy/lib/tests/test-release-container-environment.sh
# sources this file and calls this function directly against a REAL
# read-only bind mount to reproduce the exact v1.0.0-rc.4 CARGO_HOME
# configuration, without needing Docker or the pinned AlmaLinux image at
# all. Production usage (below, inside run_in_container) executes this
# identical function body inside the build container via `declare -f`,
# so the test and production never risk drifting into two different
# implementations of "is CARGO_HOME actually writable".
assert_cargo_home_writable() {
  local cargo_home="$1"
  if [ ! -w "$cargo_home" ]; then
    echo "::error::CARGO_HOME is not writable: $cargo_home" >&2
    return 1
  fi
  if ! mkdir -p "$cargo_home/registry/cache" 2>/dev/null; then
    echo "::error::cannot create $cargo_home/registry/cache — CARGO_HOME cannot hold the crates.io registry cache cargo build requires" >&2
    return 1
  fi
  if ! { touch "$cargo_home/.write-test" 2>/dev/null && rm -f "$cargo_home/.write-test"; }; then
    echo "::error::cannot write a test file inside CARGO_HOME: $cargo_home" >&2
    return 1
  fi
  echo "CARGO_HOME is writable: $cargo_home"
}

main() {
  MODE="build"
  case "${1:-}" in
    "") ;;
    --environment-check) MODE="check" ;;
    *)
      echo "usage: $0 [--environment-check]" >&2
      exit 2
      ;;
  esac

  : "${RELEASE_BUILD_IMAGE:?RELEASE_BUILD_IMAGE must be set (digest-pinned AlmaLinux release build image)}"
  : "${RELEASE_GLIBC_BASELINE:?RELEASE_GLIBC_BASELINE must be set (e.g. 2.28)}"

  WORKSPACE_DIR="${WORKSPACE_DIR:-$PWD}"
  CARGO_BIN_DIR="${CARGO_BIN_DIR:-$HOME/.cargo/bin}"
  RUSTUP_HOME_DIR="${RUSTUP_HOME_DIR:-${RUSTUP_HOME:-$HOME/.rustup}}"
  HOST_CARGO_REGISTRY_DIR="${HOST_CARGO_REGISTRY_DIR:-$HOME/.cargo/registry}"
  RELEASE_CARGO_HOME="${RELEASE_CARGO_HOME:-${RUNNER_TEMP:-/tmp}/release-cargo-home}"
  BUILD_IMAGE_TAG="singbox-vpn-release-build:local"

  [ -d "$CARGO_BIN_DIR" ] || { echo "::error::cargo bin dir not found: $CARGO_BIN_DIR (expected the Rust toolchain to already be installed, e.g. via dtolnay/rust-toolchain)" >&2; exit 1; }
  [ -d "$RUSTUP_HOME_DIR" ] || { echo "::error::rustup home not found: $RUSTUP_HOME_DIR" >&2; exit 1; }

  # Pin the exact toolchain by NAME (RUSTUP_TOOLCHAIN), rather than
  # letting rustup's proxies resolve it via WORKSPACE_DIR's
  # rust-toolchain.toml directory override. A real GitHub Actions run
  # showed the override path is not a pure local lookup: rustup treated
  # a plain `rustc`/`cargo` invocation as needing to reconcile the
  # toolchain against the override file's full declared spec (channel +
  # components), attempted to download a missing component (clippy —
  # this job's dtolnay/rust-toolchain step, like release.yml's build
  # job, only requests `targets:`, not `components:`), and died with a
  # broken-pipe (exit 141) partway through — this container has no
  # business reaching the network for toolchain state at all. Setting
  # RUSTUP_TOOLCHAIN explicitly is the same mechanism the host-side
  # `rustc +1.94.1 --version --verbose` step (dtolnay/rust-toolchain's
  # own verification, which runs and succeeds moments earlier in every
  # log) already uses successfully: it resolves the toolchain by name
  # directly, with no directory-override lookup and no component
  # reconciliation. Parsed from rust-toolchain.toml itself rather than
  # plumbed through as a separate workflow input, so there is exactly
  # one place this repository's pinned channel is written down.
  RUST_TOOLCHAIN_NAME="${RUST_TOOLCHAIN_NAME:-$(sed -nE 's/^channel = "([^"]+)"$/\1/p' "$WORKSPACE_DIR/rust-toolchain.toml")}"
  [ -n "$RUST_TOOLCHAIN_NAME" ] || { echo "::error::could not determine the pinned Rust channel from $WORKSPACE_DIR/rust-toolchain.toml" >&2; exit 1; }

  mkdir -p "$RELEASE_CARGO_HOME"

  host_uid="$(id -u)"
  host_gid="$(id -g)"

  echo "== preparing release build image (installing build dependencies as root, once, inside the container's own layer) =="
  deps_container="$(docker create "$RELEASE_BUILD_IMAGE" bash -c 'set -euo pipefail; dnf install -y gcc gcc-c++ make >/dev/null')"
  cleanup_deps_container() { docker rm -f "$deps_container" >/dev/null 2>&1 || true; }
  trap cleanup_deps_container EXIT
  docker start -a "$deps_container"
  docker commit "$deps_container" "$BUILD_IMAGE_TAG" >/dev/null
  cleanup_deps_container
  trap - EXIT

  run_in_container() {
    docker run --rm \
      --user "${host_uid}:${host_gid}" \
      -v "$WORKSPACE_DIR:/workspace" -w /workspace \
      -v "$CARGO_BIN_DIR:/opt/rust-bin:ro" \
      -v "$RUSTUP_HOME_DIR:/opt/rustup" \
      -v "$RELEASE_CARGO_HOME:/cargo-home" \
      -e CARGO_HOME=/cargo-home \
      -e RUSTUP_HOME=/opt/rustup \
      -e RUSTUP_TOOLCHAIN="$RUST_TOOLCHAIN_NAME" \
      -e HOME=/cargo-home \
      -e PATH="/opt/rust-bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" \
      "$BUILD_IMAGE_TAG" \
      bash -c "$1"
  }

  echo "== verifying build container environment (toolchain, compiler, CARGO_HOME writability) =="
  check_out="$(run_in_container "
    set -euo pipefail
    echo '--- rustc --version ---'; rustc --version
    echo '--- cargo --version ---'; cargo --version
    echo '--- gcc --version ---'; gcc --version
    echo '--- ldd --version (build container glibc) ---'; ldd --version

    $(declare -f assert_cargo_home_writable)
    assert_cargo_home_writable \"\$CARGO_HOME\"
  ")"
  echo "$check_out"

  # Assert the actual build container GLIBC baseline rather than merely
  # printing it — if RELEASE_BUILD_IMAGE ever drifts to a newer AlmaLinux
  # digest with a newer glibc, this must fail immediately, not silently
  # ship a binary with a higher ABI requirement than declared.
  container_glibc="$(printf '%s\n' "$check_out" | grep -m1 -oE '[0-9]+\.[0-9]+$' || true)"
  if [ -z "$container_glibc" ]; then
    echo "::error::could not determine build container glibc version from ldd --version output" >&2
    exit 1
  fi
  if [ "$container_glibc" != "$RELEASE_GLIBC_BASELINE" ]; then
    echo "::error::build container glibc is $container_glibc, expected exactly $RELEASE_GLIBC_BASELINE. RELEASE_BUILD_IMAGE may have drifted to a newer ABI baseline." >&2
    exit 1
  fi
  echo "build container glibc baseline confirmed: $container_glibc"

  if [ "$MODE" = "check" ]; then
    echo "== --environment-check: contract verified, skipping full workspace compile =="
    exit 0
  fi

  # Seed the writable, dedicated Cargo home from whatever registry a
  # prior job/run already restored on the host (typically Swatinem/
  # rust-cache, pointed at this same directory via CARGO_HOME — see
  # release.yml), so a warm cache never triggers a full crates.io index
  # re-clone. A cold cache (nothing restored yet, or nothing to seed
  # from) is a correct, if slower, first run — never an error. Deferred
  # until here (never for --environment-check, which needs no registry
  # contents) so the smoke path run on every relevant PR never pays for
  # copying a warm registry.
  if [ -d "$HOST_CARGO_REGISTRY_DIR" ] && [ ! -e "$RELEASE_CARGO_HOME/registry" ]; then
    echo "seeding $RELEASE_CARGO_HOME/registry from $HOST_CARGO_REGISTRY_DIR"
    cp -a "$HOST_CARGO_REGISTRY_DIR" "$RELEASE_CARGO_HOME/registry"
  fi

  echo "== compiling release binaries (vpn-admin, subscription) =="
  run_in_container '
    set -euo pipefail
    cargo build --locked --release --target x86_64-unknown-linux-gnu -p admin -p subscription
  '
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  main "$@"
fi
