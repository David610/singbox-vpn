#!/usr/bin/env bash
# Offline qualification gate for an Arcana node release candidate. It never
# installs on, connects to, or mutates a VPS. Network is used only by the
# existing pinned sing-box download and release-container checks when enabled.
set -Eeuo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
# Test HTTP servers bind loopback; hostile/global CI proxy settings must never
# divert those requests away from the local fault-injection server.
export NO_PROXY="${NO_PROXY:+$NO_PROXY,}localhost,127.0.0.1,::1"
export no_proxy="${no_proxy:+$no_proxy,}localhost,127.0.0.1,::1"
MODE="${1:---full}"
case "$MODE" in --full|--quick) ;; *) echo "usage: $0 [--quick|--full]" >&2; exit 2;; esac
run() { printf '\n==> %q' "$1"; shift; printf ' %q' "$@"; printf '\n'; "$@"; }

run contract bash deploy/lib/tests/test-node-offline-contract.sh
run agent-tests cargo test --locked -p provisioning-agent
run authorization-tests cargo test --locked -p compat-config authorization
run two-hop-tests cargo test --locked -p compat-config --test two_hop_system
run nftables-tests bash deploy/lib/tests/test-nftables-egress-lifecycle.sh
run systemd-tests bash deploy/lib/tests/test-systemd-resource-protection.sh
run release-contract bash deploy/lib/tests/test-release-reproducibility.sh
run sbom-contract bash deploy/lib/tests/test-release-sbom.sh

if [ "$MODE" = "--quick" ]; then
  echo 'OFFLINE RELEASE GATE: QUICK PASS (full build/audit/gate intentionally omitted)'
  exit 0
fi
run fast-gate bash deploy/lib/fast-gate.sh
SBOM_DIR="${TMPDIR:-/tmp}/arcana-offline-sbom.$$"
trap 'rm -rf "$SBOM_DIR"' EXIT
run sbom bash deploy/lib/generate-sbom.sh --version offline-rc --output-dir "$SBOM_DIR"
python3 - "$SBOM_DIR/singbox-vpn-sbom.cdx.json" <<'PY'
import json,sys
x=json.load(open(sys.argv[1])); assert x['bomFormat']=='CycloneDX' and len(x['components'])>10
print('CycloneDX SBOM content: PASS')
PY
if command -v cargo-audit >/dev/null 2>&1; then
  run dependency-audit cargo audit --no-fetch
else
  echo 'FAIL: cargo-audit is required for --full (install it or use CI)' >&2
  exit 1
fi
run release-build cargo build --locked --workspace --release
run release-repro-build bash deploy/lib/tests/test-release-stability-regressions.sh
echo 'OFFLINE-QUALIFIED NODE RELEASE CANDIDATE: PASS (VPS validation is still mandatory)'
