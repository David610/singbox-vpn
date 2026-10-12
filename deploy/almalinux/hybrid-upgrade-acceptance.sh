#!/usr/bin/env bash
# Disposable-host acceptance of the supported v1.2.0 -> candidate TRANSITION,
# including the "hybrid" host the owner actually has: a v1.2.0 installation whose
# vpn/vpn-admin were later replaced by hand with a newer build, optionally with
# an [egress] policy already applied.
#
# This is NOT lifecycle-acceptance.sh. That harness starts from a fresh install
# and ends in an uninstall; a host built that way can never be a hybrid. This
# script starts from an EXISTING baseline, creates the scenario's state in
# place, runs the supported updater from the baseline's own installed tree
# (the v1.2.0 updater — the only updater such a host has before it is updated),
# and checks the result. It never uninstalls, reinstalls or "repairs toward the
# old release" before the transition is tested.
#
#   --scenario normal         plain v1.2.0 host -> candidate (policy stays the default `allow`)
#   --scenario hybrid         v1.2.0 + candidate executables copied over by hand
#   --scenario hybrid-policy  hybrid + `egress set reject_google` already applied
#
# Prepare the baseline once per scenario (an empty disposable host):
#   ... --prepare-baseline --domain DISPOSABLE_DOMAIN   (installs the baseline release)
# or bring your own already-installed disposable baseline host.
#
# SAFETY (same rules as lifecycle-acceptance.sh):
#   1. Requires --host, --candidate and --i-understand-this-modifies-the-host.
#   2. Refuses localhost/loopback and this machine's own production host.
#   3. Needs a verified SSH host key (--known-hosts-file) unless
#      --unverified-host-key is given for a throwaway development run.
#   4. --prepare-baseline refuses a host that already has an installation.
#   5. Everything runs over SSH against the explicit target only; no secret is
#      printed (credentials are compared only as SHA-256 fingerprints).
set -Eeuo pipefail

HOST=""; ACK=0; CANDIDATE=""; BASELINE="v1.2.0"; SCENARIO=""; PREPARE=0; DOMAIN=""
SSH_PORT=22; KNOWN_HOSTS_FILE=""; UNVERIFIED_KEY=0

usage() {
  cat <<'USAGE'
Usage: hybrid-upgrade-acceptance.sh --host USER@HOST --candidate vX.Y.Z[-rc.N] \
         --scenario normal|hybrid|hybrid-policy --i-understand-this-modifies-the-host \
         (--known-hosts-file FILE | --unverified-host-key) [--baseline v1.2.0] \
         [--prepare-baseline --domain NAME] [--ssh-port N]
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --host) HOST="$2"; shift 2 ;;
    --candidate) CANDIDATE="$2"; shift 2 ;;
    --baseline) BASELINE="$2"; shift 2 ;;
    --scenario) SCENARIO="$2"; shift 2 ;;
    --domain) DOMAIN="$2"; shift 2 ;;
    --ssh-port) SSH_PORT="$2"; shift 2 ;;
    --known-hosts-file) KNOWN_HOSTS_FILE="$2"; shift 2 ;;
    --unverified-host-key) UNVERIFIED_KEY=1; shift ;;
    --prepare-baseline) PREPARE=1; shift ;;
    --i-understand-this-modifies-the-host) ACK=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 1 ;;
  esac
done

die() { echo "ERROR: $*" >&2; exit 1; }
TAG_RE='^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$'
[ -n "$HOST" ] || die "--host is required."
[ "$ACK" -eq 1 ] || die "refusing to run: pass --i-understand-this-modifies-the-host (this updates and reconfigures the target)."
[[ "$CANDIDATE" =~ $TAG_RE ]] || die "--candidate must be an immutable release tag (vX.Y.Z or vX.Y.Z-rc.N), got '$CANDIDATE'."
[[ "$BASELINE" =~ $TAG_RE ]] || die "--baseline must be an immutable release tag, got '$BASELINE'."
case "$SCENARIO" in normal|hybrid|hybrid-policy) ;; *) die "--scenario must be normal, hybrid or hybrid-policy." ;; esac
case "$SSH_PORT" in ''|*[!0-9]*) die "--ssh-port must be numeric." ;; esac
if [ -n "$DOMAIN" ] && [[ ! "$DOMAIN" =~ ^[A-Za-z0-9]([A-Za-z0-9.-]*[A-Za-z0-9])?$ ]]; then die "--domain is not a valid hostname."; fi
if [ "$PREPARE" -eq 1 ] && [ -z "$DOMAIN" ]; then die "--prepare-baseline needs --domain."; fi

host_part="${HOST#*@}"
case "$host_part" in
  localhost|127.0.0.1|0.0.0.0|::1|"") die "refusing to target '$HOST' (loopback)." ;;
esac
if [ -n "${SINGBOX_VPN_PRODUCTION_HOST:-}" ] && [ "$host_part" = "$SINGBOX_VPN_PRODUCTION_HOST" ]; then
  die "refusing to target '$HOST': it matches SINGBOX_VPN_PRODUCTION_HOST."
fi
if [ -f /etc/vpn/deployment.toml ]; then
  prod_host="$(grep -m1 '^public_host' /etc/vpn/deployment.toml 2>/dev/null | sed -E 's/^public_host *= *"([^"]*)".*/\1/')"
  [ -z "$prod_host" ] || [ "$host_part" != "$prod_host" ] || die "refusing to target '$HOST': it is this machine's own production host."
fi
if [ -n "$KNOWN_HOSTS_FILE" ]; then
  [ -s "$KNOWN_HOSTS_FILE" ] || die "--known-hosts-file '$KNOWN_HOSTS_FILE' is not a non-empty file."
  HOSTKEY_OPTS=(-o StrictHostKeyChecking=yes -o "UserKnownHostsFile=$KNOWN_HOSTS_FILE" -o GlobalKnownHostsFile=/dev/null)
elif [ "$UNVERIFIED_KEY" -eq 1 ]; then
  HOSTKEY_OPTS=(-o StrictHostKeyChecking=accept-new)
else
  die "a verified host key is required: pass --known-hosts-file FILE (or --unverified-host-key for a throwaway development run)."
fi

SSH_OPTS=(-p "$SSH_PORT" -o BatchMode=yes -o ConnectTimeout=10 "${HOSTKEY_OPTS[@]}")
ssh_run() { timeout 180 ssh "${SSH_OPTS[@]}" "$HOST" "$@"; }
ssh_run_long() { timeout 1500 ssh "${SSH_OPTS[@]}" "$HOST" "$@"; }

failures=0
pass() { printf '[PASS] %s\n' "$1"; }
fail() { printf '[FAIL][required] %s %s\n' "$1" "${2:-}"; failures=$((failures + 1)); }
section() { echo; echo "=== $1 ==="; }
check() { # check "description" remote-command  (PASS when the command exits 0)
  if ssh_run "$2" >/dev/null 2>&1; then pass "$1"; else fail "$1"; fi
}

MANIFEST=/var/lib/singbox-vpn/install-state.json
USERS=/etc/vpn/compat/users/users.json
CAND_CORE="${CANDIDATE#v}"; CAND_CORE="${CAND_CORE%%-*}"
BASE_CORE="${BASELINE#v}"; BASE_CORE="${BASE_CORE%%-*}"
TARGET_TRIPLE=x86_64-unknown-linux-gnu
FP_USERS="sudo sha256sum $USERS | cut -d' ' -f1"

echo "hybrid-upgrade acceptance: scenario=$SCENARIO baseline=$BASELINE candidate=$CANDIDATE target=$HOST"
echo "(no uninstall, reinstall or repair is run before the transition is tested)"

section "0. connectivity and platform"
if ssh_run true 2>/dev/null; then pass "SSH reachable (host key checked)"; else fail "SSH reachable (host key checked)"; fi
check "x86_64" '[ "$(uname -m)" = x86_64 ]'
check "AlmaLinux 9 or Ubuntu 24.04" '. /etc/os-release; { [ "$ID" = almalinux ] && [[ "$VERSION_ID" == 9* ]]; } || { [ "$ID" = ubuntu ] && [ "$VERSION_ID" = 24.04 ]; }'

section "1. baseline $BASELINE is installed and the host is NOT already updated"
if ! ssh_run "[ -e $MANIFEST ]" 2>/dev/null; then
  if [ "$PREPARE" -eq 1 ]; then
    if ssh_run '[ ! -e /etc/vpn ] && [ ! -e /opt/singbox-vpn ]' 2>/dev/null; then
      if ssh_run_long "set -o pipefail; curl -fsSL --retry 5 https://raw.githubusercontent.com/David610/singbox-vpn/main/install.sh | SINGBOX_VPN_SUPPRESS_ONBOARDING_SECRETS=1 SINGBOX_VPN_VERSION=$BASELINE REALITY_HANDSHAKE_SERVER=www.google.com bash -s -- --domain $DOMAIN" >/dev/null 2>&1; then
        pass "baseline $BASELINE installed on the empty disposable host"
      else
        fail "baseline $BASELINE installed on the empty disposable host"
      fi
    else
      fail "--prepare-baseline refused" "(the host already has an installation; it must be empty)"
    fi
  else
    fail "baseline present" "(no install-state manifest; pass --prepare-baseline on an empty host)"
  fi
fi
if ssh_run "grep -q '\"singbox_vpn_version\": \"$BASELINE\"' $MANIFEST" 2>/dev/null; then
  pass "install-state records the baseline release $BASELINE"
else
  fail "install-state records the baseline release $BASELINE" "(the host is not at the baseline; this scenario needs a host that has not been updated)"
fi
check "baseline vpn-admin reports $BASE_CORE" "[ \"\$(/usr/local/bin/vpn-admin --version | awk '{print \$2}')\" = '$BASE_CORE' ]"

section "2. scenario state ($SCENARIO), created in place"
check "a non-reserved test user exists" "/usr/local/bin/vpn-admin user list | grep -q hybrid-test || /usr/local/bin/vpn-admin user create --name hybrid-test >/dev/null 2>&1"
if [ "$SCENARIO" != normal ]; then
  # The "development executable": the candidate's own vpn-admin, fetched with its
  # checksum, copied over the installed binaries by hand — exactly the manual swap.
  if ssh_run_long "set -e; d=\$(mktemp -d); cd \$d; base=https://github.com/David610/singbox-vpn/releases/download/$CANDIDATE; a=singbox-vpn-$TARGET_TRIPLE.tar.gz; curl -fsSL --retry 5 -o \$a \$base/\$a; curl -fsSL --retry 5 -o SHA256SUMS \$base/SHA256SUMS; sha256sum --ignore-missing -c SHA256SUMS; tar -xzf \$a; sudo install -m 0755 singbox-vpn-$TARGET_TRIPLE/vpn-admin /usr/local/bin/vpn-admin; sudo install -m 0755 singbox-vpn-$TARGET_TRIPLE/vpn-admin /usr/local/bin/vpn; cp singbox-vpn-$TARGET_TRIPLE/vpn-admin /root/.hybrid-candidate-vpn-admin; rm -rf \$d" >/dev/null 2>&1; then
    pass "candidate executables (checksum-verified) copied over $BASELINE by hand — hybrid state created"
  else
    fail "hybrid state created"
  fi
  check "the hybrid host now reports the candidate version $CAND_CORE" "[ \"\$(/usr/local/bin/vpn-admin --version | awk '{print \$2}')\" = '$CAND_CORE' ]"
  check "install-state still names $BASELINE (the manual copy did not update it)" "grep -q '\"singbox_vpn_version\": \"$BASELINE\"' $MANIFEST"
fi
if [ "$SCENARIO" = hybrid-policy ]; then
  check "reject_google applied with the hybrid vpn-admin" "sudo /usr/local/bin/vpn-admin egress set reject_google >/dev/null 2>&1 && sudo /usr/local/bin/vpn-admin egress show | grep -q '^ipv6_literal: reject_google'"
fi
USERS_BEFORE="$(ssh_run "$FP_USERS" 2>/dev/null || true)"
[ -n "$USERS_BEFORE" ] && pass "credential fingerprint recorded (SHA-256 only, nothing printed)" || fail "credential fingerprint recorded"

section "3. the supported transition, run by the BASELINE's own installed updater"
if ssh_run_long "sudo /opt/singbox-vpn/deploy/almalinux/update.sh --version $CANDIDATE" >/dev/null 2>&1; then
  pass "update.sh --version $CANDIDATE succeeded"
else
  fail "update.sh --version $CANDIDATE succeeded" "(the host was left to the updater's own rollback)"
fi

section "4. the result"
check "install-state records $CANDIDATE" "grep -q '\"singbox_vpn_version\": \"$CANDIDATE\"' $MANIFEST"
check "vpn-admin reports $CAND_CORE" "[ \"\$(/usr/local/bin/vpn-admin --version | awk '{print \$2}')\" = '$CAND_CORE' ]"
check "installed vpn-admin equals the published candidate executable (independent of the manifest)" "d=\$(mktemp -d); cd \$d; base=https://github.com/David610/singbox-vpn/releases/download/$CANDIDATE; a=singbox-vpn-$TARGET_TRIPLE.tar.gz; curl -fsSL --retry 5 -o \$a \$base/\$a && curl -fsSL --retry 5 -o SHA256SUMS \$base/SHA256SUMS && sha256sum --ignore-missing -c SHA256SUMS && tar -xzf \$a && [ \"\$(sha256sum singbox-vpn-$TARGET_TRIPLE/vpn-admin | cut -d' ' -f1)\" = \"\$(sha256sum /usr/local/bin/vpn-admin | cut -d' ' -f1)\" ]; rc=\$?; rm -rf \$d; exit \$rc"
USERS_AFTER="$(ssh_run "$FP_USERS" 2>/dev/null || true)"
[ -n "$USERS_AFTER" ] && [ "$USERS_AFTER" = "$USERS_BEFORE" ] && pass "users and credentials unchanged by the update" || fail "users and credentials unchanged by the update"
if [ "$SCENARIO" = hybrid-policy ]; then
  check "the reject_google policy survived the update" "sudo /usr/local/bin/vpn-admin egress show | grep -q '^ipv6_literal: reject_google'"
  check "the running sing-box config carries the policy rules (sniff + reject)" "[ \"\$(sudo python3 -c \"import json;print(sum(1 for r in json.load(open('/etc/vpn/compat/sing-box/config.json'))['route']['rules'] if r.get('ip_version')==6))\")\" = 2 ]"
else
  check "the egress policy is still the default for this scenario (allow)" "sudo /usr/local/bin/vpn-admin egress show | grep -q 'allow (plain exit default)'"
fi
check "doctor --protocol --require-protocol passes (real handshake)" "sudo /usr/local/bin/vpn-admin doctor --protocol --require-protocol >/dev/null 2>&1"
check "the old updater could not record executable hashes: identity is UNKNOWN, not a false match" "sudo /usr/local/bin/vpn-admin doctor 2>&1 | grep -q 'release identity UNKNOWN'"

section "5. record identity with the NEW tree's updater, without losing anything"
if ssh_run_long "sudo /opt/singbox-vpn/deploy/almalinux/update.sh --repair" >/dev/null 2>&1; then pass "update.sh --repair (new tree, recorded release is the candidate) succeeded"; else fail "update.sh --repair (new tree) succeeded"; fi
check "release identity now matches the recorded candidate" "sudo /usr/local/bin/vpn-admin doctor 2>&1 | grep -q 'release identity: recorded'"
USERS_REPAIR="$(ssh_run "$FP_USERS" 2>/dev/null || true)"
[ "$USERS_REPAIR" = "$USERS_BEFORE" ] && pass "credentials unchanged by repair" || fail "credentials unchanged by repair"
if [ "$SCENARIO" = hybrid-policy ]; then
  check "the policy survived repair" "sudo /usr/local/bin/vpn-admin egress show | grep -q '^ipv6_literal: reject_google'"

  section "6. a downgrade toward the baseline is refused BEFORE anything changes"
  BIN_BEFORE="$(ssh_run "sha256sum /usr/local/bin/vpn-admin | cut -d' ' -f1" 2>/dev/null || true)"
  if ssh_run_long "sudo /opt/singbox-vpn/deploy/almalinux/update.sh --version $BASELINE" >/dev/null 2>&1; then
    fail "update.sh --version $BASELINE must be refused while [egress] is configured" "(it succeeded: the policy would have been silently dropped)"
  else
    pass "update.sh --version $BASELINE refused"
  fi
  BIN_AFTER="$(ssh_run "sha256sum /usr/local/bin/vpn-admin | cut -d' ' -f1" 2>/dev/null || true)"
  [ -n "$BIN_BEFORE" ] && [ "$BIN_BEFORE" = "$BIN_AFTER" ] && pass "the refusal left the installed executable untouched" || fail "the refusal left the installed executable untouched"
  check "the policy is still applied after the refused downgrade" "sudo /usr/local/bin/vpn-admin egress show | grep -q '^ipv6_literal: reject_google'"
fi

section "summary"
echo "scenario: $SCENARIO; failing stages: $failures"
if [ "$failures" -ne 0 ]; then echo "HYBRID UPGRADE GATE: FAIL"; exit 1; fi
echo "HYBRID UPGRADE GATE: PASS ($SCENARIO)"
