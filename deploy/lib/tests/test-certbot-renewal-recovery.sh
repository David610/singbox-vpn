#!/usr/bin/env bash
# Regression coverage for the certbot deploy-hook's stale-transaction
# recovery (P7): an interrupted renewal (SIGKILL/OOM/host power loss
# between the live swap and the final cleanup) used to leave a
# `*.renew-bak` file that made every SUBSEQUENT renewal `die` immediately
# and permanently, requiring manual intervention. This exercises both
# recovery branches — "the interrupted run's swap already completed" and
# "it was interrupted mid-swap, leaving a mismatched pair" — plus the
# genuinely-unrecoverable case, entirely against a throwaway temp dir (no
# real /etc/vpn, no real sing-box/nginx, no real systemd).
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
HOOK="$ROOT/deploy/almalinux/certbot-deploy-hook.sh"
bash -n "$HOOK"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

PUBLIC_HOST="vpn.example.com"

# The hook's `install -g sing-box` calls (production ownership, unrelated
# to and not weakened by this test) require that group to exist. Real
# deployments get it from install.sh; this test suite runs as root in CI
# the same way apps/admin's own ownership-dependent tests already do
# (see docs/INCIDENT_2026-08-11...: "real getgrnam(3) calls ... create
# real OS groups"), so create it here if missing rather than skip
# coverage of the ownership-preserving install calls.
if ! getent group sing-box >/dev/null 2>&1; then
  groupadd sing-box 2>/dev/null || {
    echo "skipping: cannot create the 'sing-box' group (not root?) — this test requires root, same as the production hook itself" >&2
    exit 0
  }
fi

gen_cert_pair() {
  # $1=cert path $2=key path $3=CN/SAN — a fresh, validly-SAN'd,
  # currently-unexpired self-signed pair, matching what the real hook
  # validates (openssl x509 -checkend 0, DNS SAN, matching pubkey hash).
  local cert=$1 key=$2 host=$3
  openssl req -x509 -newkey rsa:2048 -nodes -keyout "$key" -out "$cert" \
    -days 2 -subj "/CN=${host}" -addext "subjectAltName=DNS:${host}" \
    >/dev/null 2>&1
}

setup_env() {
  local dir=$1
  mkdir -p "$dir/state/hysteria" "$dir/state/sing-box" "$dir/renewed/$PUBLIC_HOST"
  cat > "$dir/deployment.toml" <<EOF
public_host = "$PUBLIC_HOST"
subscription_host = "$PUBLIC_HOST"
EOF
  echo '{}' > "$dir/state/sing-box/config.json"

  # Fake sing-box: `check -c PATH` always succeeds.
  cat > "$dir/sing-box" <<'EOF'
#!/usr/bin/env bash
case "$1" in
  check) exit 0 ;;
esac
exit 0
EOF
  chmod +x "$dir/sing-box"

  # Fake systemctl: every verb succeeds, no real systemd needed. `nginx`
  # is deliberately NOT put on PATH, so the optional nginx-reload branch
  # is skipped entirely (this is a Hysteria2-only test host).
  mkdir -p "$dir/bin"
  cat > "$dir/bin/systemctl" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
  chmod +x "$dir/bin/systemctl"
}

run_hook() {
  local dir=$1 renewed_lineage=$2
  env -i \
    PATH="$dir/bin:/usr/bin:/bin" \
    RENEWED_LINEAGE="$renewed_lineage" \
    STATE_DIR="$dir/state" \
    DEPLOYMENT_TOML="$dir/deployment.toml" \
    SINGBOX_BIN="$dir/sing-box" \
    SINGBOX_VPN_LOCK_FILE="$dir/singbox-vpn.lock" \
    bash "$HOOK" 2>&1
}

live_pair_matches() {
  local dir=$1 want_cert=$2
  cmp -s "$dir/state/hysteria/cert.pem" "$want_cert"
}

# --- Test A: interrupted run whose live swap already completed --------
# Live files are already the NEW (renewed) pair; a stale backup of the
# OLD pair is left over, as if the process died right after the two `mv`s
# but before cleanup. Recovery must recognize the live pair is already
# valid, discard the stale backup, and continue successfully.
dir="$WORK/a"
setup_env "$dir"
gen_cert_pair "$dir/old-cert.pem" "$dir/old-key.pem" "$PUBLIC_HOST"
gen_cert_pair "$dir/new-cert.pem" "$dir/new-key.pem" "$PUBLIC_HOST"
cp "$dir/new-cert.pem" "$dir/state/hysteria/cert.pem"
cp "$dir/new-key.pem" "$dir/state/hysteria/key.pem"
cp "$dir/old-cert.pem" "$dir/state/hysteria/cert.pem.renew-bak"
cp "$dir/old-key.pem" "$dir/state/hysteria/key.pem.renew-bak"
cp "$dir/new-cert.pem" "$dir/renewed/$PUBLIC_HOST/fullchain.pem"
cp "$dir/new-key.pem" "$dir/renewed/$PUBLIC_HOST/privkey.pem"

out=$(run_hook "$dir" "$dir/renewed/$PUBLIC_HOST") || { echo "Test A FAILED: hook exited non-zero:"; echo "$out"; exit 1; }
echo "$out" | grep -q "already completed safely" || { echo "Test A FAILED: did not recognize the already-completed swap:"; echo "$out"; exit 1; }
live_pair_matches "$dir" "$dir/new-cert.pem" || { echo "Test A FAILED: live cert is not the renewed cert"; exit 1; }
[ ! -e "$dir/state/hysteria/cert.pem.renew-bak" ] || { echo "Test A FAILED: stale backup was not cleaned up"; exit 1; }
echo "Test A (swap already completed before interruption): PASS"

# --- Test B: interrupted run mid-swap, live pair mismatched -----------
# Live cert is the NEW cert but live key is still the OLD key (as if the
# process died between the two separate `mv` calls). Recovery must detect
# the mismatch, restore the OLD matched pair from backup, and THEN
# proceed to install the actual renewal on top of that recovered state.
dir="$WORK/b"
setup_env "$dir"
gen_cert_pair "$dir/old-cert.pem" "$dir/old-key.pem" "$PUBLIC_HOST"
gen_cert_pair "$dir/new-cert.pem" "$dir/new-key.pem" "$PUBLIC_HOST"
cp "$dir/new-cert.pem" "$dir/state/hysteria/cert.pem"   # new cert...
cp "$dir/old-key.pem" "$dir/state/hysteria/key.pem"     # ...but old key: mismatched
cp "$dir/old-cert.pem" "$dir/state/hysteria/cert.pem.renew-bak"
cp "$dir/old-key.pem" "$dir/state/hysteria/key.pem.renew-bak"
cp "$dir/new-cert.pem" "$dir/renewed/$PUBLIC_HOST/fullchain.pem"
cp "$dir/new-key.pem" "$dir/renewed/$PUBLIC_HOST/privkey.pem"

out=$(run_hook "$dir" "$dir/renewed/$PUBLIC_HOST") || { echo "Test B FAILED: hook exited non-zero:"; echo "$out"; exit 1; }
echo "$out" | grep -q "restoring the last known-good pair" || { echo "Test B FAILED: did not detect the mismatched live pair:"; echo "$out"; exit 1; }
live_pair_matches "$dir" "$dir/new-cert.pem" || { echo "Test B FAILED: final live cert is not the newly renewed cert"; exit 1; }
[ ! -e "$dir/state/hysteria/cert.pem.renew-bak" ] || { echo "Test B FAILED: backup was not cleaned up after successful recovery+renewal"; exit 1; }
echo "Test B (interrupted mid-swap, mismatched pair): PASS"

# --- Test C: genuinely unrecoverable state must fail loudly, not silently
# Both live and backup pairs are mismatched/broken — there is no valid
# pair to fall back to. Must `die` with an actionable message, never
# fabricate success or leave the transaction lock held.
dir="$WORK/c"
setup_env "$dir"
gen_cert_pair "$dir/old-cert.pem" "$dir/old-key.pem" "$PUBLIC_HOST"
gen_cert_pair "$dir/other-key.pem" "$dir/unused.pem" "other.invalid"
cp "$dir/old-cert.pem" "$dir/state/hysteria/cert.pem"
cp "$dir/other-key.pem" "$dir/state/hysteria/key.pem"        # live: mismatched
cp "$dir/old-cert.pem" "$dir/state/hysteria/cert.pem.renew-bak"
cp "$dir/other-key.pem" "$dir/state/hysteria/key.pem.renew-bak"  # backup: also mismatched
gen_cert_pair "$dir/new-cert.pem" "$dir/new-key.pem" "$PUBLIC_HOST"
cp "$dir/new-cert.pem" "$dir/renewed/$PUBLIC_HOST/fullchain.pem"
cp "$dir/new-key.pem" "$dir/renewed/$PUBLIC_HOST/privkey.pem"

if run_hook "$dir" "$dir/renewed/$PUBLIC_HOST" >"$dir/out.log" 2>&1; then
  echo "Test C FAILED: hook must not succeed when no valid pair can be recovered:"
  cat "$dir/out.log"
  exit 1
fi
grep -q "manual intervention required" "$dir/out.log" || { echo "Test C FAILED: missing actionable error message:"; cat "$dir/out.log"; exit 1; }
echo "Test C (genuinely unrecoverable state fails loudly): PASS"

# --- Test D: a normal renewal with no stale backup is unaffected ------
dir="$WORK/d"
setup_env "$dir"
gen_cert_pair "$dir/old-cert.pem" "$dir/old-key.pem" "$PUBLIC_HOST"
cp "$dir/old-cert.pem" "$dir/state/hysteria/cert.pem"
cp "$dir/old-key.pem" "$dir/state/hysteria/key.pem"
gen_cert_pair "$dir/new-cert.pem" "$dir/new-key.pem" "$PUBLIC_HOST"
cp "$dir/new-cert.pem" "$dir/renewed/$PUBLIC_HOST/fullchain.pem"
cp "$dir/new-key.pem" "$dir/renewed/$PUBLIC_HOST/privkey.pem"

out=$(run_hook "$dir" "$dir/renewed/$PUBLIC_HOST") || { echo "Test D FAILED: hook exited non-zero:"; echo "$out"; exit 1; }
echo "$out" | grep -q "leftover certificate transaction backup" && { echo "Test D FAILED: reported a stale backup where none existed:"; echo "$out"; exit 1; }
live_pair_matches "$dir" "$dir/new-cert.pem" || { echo "Test D FAILED: live cert is not the renewed cert"; exit 1; }
[ ! -e "$dir/state/hysteria/cert.pem.renew-bak" ] || { echo "Test D FAILED: backup left behind after a normal successful renewal"; exit 1; }
echo "Test D (ordinary renewal, no stale backup): PASS"

echo "certbot-deploy-hook stale-lock recovery tests: PASS"
