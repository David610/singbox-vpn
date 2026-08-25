#!/usr/bin/env bash
# Canonical FAST GATE for the supported v1.0 product (see
# docs/SUPPORTED_PRODUCT.md, docs/IMPLEMENTATION_STATUS.md). Safe to run
# after every change: no root, no destructive host mutation, no VM/VPS
# required, small runtime. Aggregates the same checks CI already runs
# for the supported crate/script surface into one local command instead
# of re-inventing a new test suite:
#
#   bash deploy/lib/fast-gate.sh
#
# What it does NOT do: it never touches a real VPS, never runs
# install.sh/uninstall.sh's destructive stages, and never re-tests the
# native adaptive stack (client-daemon, transport-native, policy,
# failure-classifier, network-state, rendezvous-client, telemetry,
# services/rendezvous, services/relay-agent) — that stack is out of
# scope for v1.0 (docs/SUPPORTED_PRODUCT.md) and unaffected by this gate
# unless SUPPORTED_CRATES below is edited to add it back.
#
# For the disposable-host destructive lifecycle gate, see
# deploy/almalinux/lifecycle-acceptance.sh instead — never run that
# against a real/production VPS.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

# Kept as a package list (not a single --workspace run) so this gate
# never silently starts building/testing the native adaptive stack if a
# crate is added to the workspace later — see docs/SUPPORTED_PRODUCT.md
# "Supported code surface".
SUPPORTED_CRATES=(admin subscription compat-config common)
CARGO_PKG_ARGS=()
for c in "${SUPPORTED_CRATES[@]}"; do CARGO_PKG_ARGS+=(-p "$c"); done

SUPPORTED_SHELL_SCRIPTS=(install.sh uninstall.sh bin/singbox-vpn-uninstall deploy/almalinux/*.sh deploy/lib/*.sh deploy/lib/tests/*.sh)

SKIP_SINGBOX="${FAST_GATE_SKIP_SINGBOX:-0}"

failures=0
step() { echo; echo "== $1 =="; }
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

START_TS=$(date +%s)

step "shell syntax (bash -n)"
# shellcheck disable=SC2068
for f in ${SUPPORTED_SHELL_SCRIPTS[@]}; do
  if bash -n "$f"; then ok "$f"; else fail "bash -n $f"; fi
done

step "shellcheck"
if command -v shellcheck >/dev/null 2>&1; then
  # shellcheck disable=SC2068
  if shellcheck -S warning ${SUPPORTED_SHELL_SCRIPTS[@]}; then
    ok "shellcheck clean"
  else
    fail "shellcheck"
  fi
else
  echo "SKIP: shellcheck not installed"
fi

step "secret-logging check"
if bash deploy/lib/check-no-secret-logging.sh; then ok "no-secret-logging"; else fail "check-no-secret-logging.sh"; fi

step "no legacy pre-rename identity check"
if bash deploy/lib/check-no-legacy-identity.sh; then ok "no-legacy-identity"; else fail "check-no-legacy-identity.sh"; fi

step "supported-crate fmt check"
if cargo fmt "${CARGO_PKG_ARGS[@]}" -- --check; then ok "cargo fmt"; else fail "cargo fmt --check"; fi

step "supported-crate clippy"
if cargo clippy --locked "${CARGO_PKG_ARGS[@]}" --all-targets -- -D warnings; then
  ok "cargo clippy"
else
  fail "cargo clippy"
fi

step "supported-crate build + unit/integration tests"
if cargo test --locked "${CARGO_PKG_ARGS[@]}"; then ok "cargo test"; else fail "cargo test"; fi

step "shell fixture tests (config/ownership/permission/migration, non-destructive)"
for t in deploy/lib/tests/*.sh; do
  name="$(basename "$t")"
  if [ "$name" = "test-uninstall-idempotency.sh" ]; then
    # Requires root and is self-gated to only run as a guaranteed no-op
    # (see the script's own header); running unprivileged here is a
    # correct, expected skip, not a gate failure.
    echo "-- $name --"
    bash "$t" || fail "$name"
    continue
  fi
  echo "-- $name --"
  if bash "$t"; then ok "$name"; else fail "$name"; fi
done

if [ "$SKIP_SINGBOX" = "1" ]; then
  step "real sing-box render + check (SKIPPED: FAST_GATE_SKIP_SINGBOX=1)"
else
  step "real sing-box render + check (same pinned version as install.sh)"
  # Extracted from install.sh at run time (not hand-duplicated) so this
  # gate can never silently drift from the version/checksum the real
  # installer ships — deploy/lib/versions.env is the single authoritative
  # source install.sh and CI's singbox-validate job also read.
  # shellcheck source=/dev/null
  . deploy/lib/versions.env
  if [ -z "${SINGBOX_VERSION:-}" ] || [ -z "${SINGBOX_SHA256_AMD64:-}" ]; then
    fail "SINGBOX_VERSION/SINGBOX_SHA256_AMD64 missing from deploy/lib/versions.env"
  else
    GATE_TMP="$(mktemp -d)"
    SINGBOX_BIN="$GATE_TMP/sing-box"
    if command -v sing-box >/dev/null 2>&1 && sing-box version 2>/dev/null | grep -q "$SINGBOX_VERSION"; then
      SINGBOX_BIN="$(command -v sing-box)"
      ok "reusing already-installed sing-box $SINGBOX_VERSION"
    else
      tarball="$GATE_TMP/sing-box.tar.gz"
      url="https://github.com/SagerNet/sing-box/releases/download/v${SINGBOX_VERSION}/sing-box-${SINGBOX_VERSION}-linux-amd64.tar.gz"
      if curl -fsSL --connect-timeout 10 --max-time 120 -o "$tarball" "$url"; then
        actual="$(sha256sum "$tarball" | awk '{print $1}')"
        if [ "$actual" != "$SINGBOX_SHA256_AMD64" ]; then
          fail "sing-box checksum mismatch: expected $SINGBOX_SHA256_AMD64, got $actual"
          SINGBOX_BIN=""
        else
          tar -xzf "$tarball" -C "$GATE_TMP"
          install -m 0755 "$GATE_TMP/sing-box-${SINGBOX_VERSION}-linux-amd64/sing-box" "$SINGBOX_BIN"
          ok "downloaded + checksum-verified sing-box $SINGBOX_VERSION"
        fi
      else
        echo "SKIP: could not download sing-box (no network?) — sing-box check/render step skipped"
        SINGBOX_BIN=""
      fi
    fi

    if [ -n "$SINGBOX_BIN" ]; then
      STATE="$GATE_TMP/state"
      mkdir -p "$STATE/hysteria"
      if openssl req -x509 -newkey ed25519 -days 1 -nodes \
          -keyout "$STATE/hysteria/key.pem" -out "$STATE/hysteria/cert.pem" \
          -subj "/CN=fast-gate.example.com" >/dev/null 2>&1; then
        cat > "$GATE_TMP/deployment.toml" <<EOF
public_host = "fast-gate.example.com"
subscription_host = "sub.fast-gate.example.com"
state_dir = "$STATE"
singbox_binary = "$SINGBOX_BIN"

[reality]
listen_port = 443
handshake_server = "www.google.com"

[hysteria2]
listen_port = 443

[subscription]
listen_port = 9100
EOF
        cargo build --locked -p admin >/dev/null
        ADMIN_BIN="$REPO_ROOT/target/debug/vpn-admin"
        if SINGBOX_VPN_ALLOW_OFFLINE_MUTATION=1 "$ADMIN_BIN" --config "$GATE_TMP/deployment.toml" init \
            && SINGBOX_VPN_ALLOW_OFFLINE_MUTATION=1 "$ADMIN_BIN" --config "$GATE_TMP/deployment.toml" user create --name fast-gate-user \
            && "$ADMIN_BIN" --config "$GATE_TMP/deployment.toml" render-config \
            && "$SINGBOX_BIN" check -c "$STATE/sing-box/config.json"; then
          ok "render-config + real sing-box check"
        else
          fail "render-config / sing-box check"
        fi
      else
        fail "could not generate test TLS cert for sing-box render fixture"
      fi
    fi
    rm -rf "$GATE_TMP"
  fi
fi

END_TS=$(date +%s)
step "summary"
echo "elapsed: $((END_TS - START_TS))s"
if [ "$failures" -eq 0 ]; then
  echo "FAST GATE: PASS"
  exit 0
else
  echo "FAST GATE: FAIL ($failures failing step(s))"
  exit 1
fi
