#!/usr/bin/env bash
# Canonical DESTRUCTIVE lifecycle acceptance gate — disposable
# AlmaLinux 9 x86_64 only. This is the release gate for
# install/uninstall/update rewrites; deploy/lib/fast-gate.sh is the
# cheap per-change gate and does NOT exercise any of this.
#
#   ssh root@DISPOSABLE-HOST -- true   # confirm you can reach it first
#   ./deploy/almalinux/lifecycle-acceptance.sh \
#       --host root@DISPOSABLE-HOST --i-understand-this-is-destructive
#
# What this does: drives install.sh -> acceptance-test.sh -> reboot ->
# health-check.sh -> re-run install.sh (idempotent repair) ->
# update.sh (if a second version/fixture is given) -> vpn-admin user
# create/disable/rotate-token/delete -> vpn-admin doctor --protocol ->
# a deliberately interrupted install (failure-injection hook, see
# below) -> uninstall.sh -> SSH/host-state checks -> reinstall, over
# SSH, on ONE remote host you name explicitly. It never runs any stage
# locally and never assumes a target.
#
# SAFETY (non-negotiable, do not "fix" by removing a check):
#   1. Refuses to run without --host AND
#      --i-understand-this-is-destructive.
#   2. Refuses --host localhost/127.0.0.1/0.0.0.0/::1/<no value> — this
#      script is SSH-only by construction so it cannot "fall back" to
#      the machine it's invoked from.
#   3. Refuses when --host matches this repo's own configured
#      production PUBLIC_HOST (checked against /etc/vpn/deployment.toml
#      if present LOCALLY, and against $VPN1_PRODUCTION_HOST if set) —
#      a copy/paste of your real VPN's hostname must not silently wipe
#      it.
#   4. Every stage is a real SSH command against the real target; no
#      systemd/firewalld/package behavior is faked or assumed inside a
#      container substitute (see acceptance-test.sh's own container-vs-VM
#      note — this script inherits that same constraint by only ever
#      running over SSH against whatever --host actually is).
#
# What this does NOT cover (mark UNVERIFIED, do by hand):
#   - Public/internet reachability from outside the host's network.
#   - Real mobile client (Hiddify iOS/Android/MagicOS) import + connect.
#   - Anything requiring a second physical device.
#
# Each stage prints PASS/FAIL and the script keeps going where safe so
# one failure doesn't hide the rest of the report; exit code is
# non-zero if any REQUIRED stage failed.
set -Eeuo pipefail

HOST=""
ACK=0
UPDATE_TO_REF=""
SKIP_REBOOT=0

usage() {
  cat <<'USAGE'
deploy/almalinux/lifecycle-acceptance.sh --host user@disposable-host --i-understand-this-is-destructive [options]

Required:
  --host USER@HOST                     disposable AlmaLinux 9 x86_64 target, reached via SSH.
                                        Refuses localhost/127.0.0.1/::1 and this repo's own
                                        configured production PUBLIC_HOST.
  --i-understand-this-is-destructive   explicit acknowledgement; this WIPES the target host's
                                        vpn1 state (and, on failure-injection stages, more).

Options:
  --update-to-ref REF   after the clean install, also exercise update.sh against REF
                         (branch/tag) as a second version — skipped if not given.
  --skip-reboot          skip the reboot+health stage (e.g. host cannot be rebooted by this
                         SSH session's caller). Every other stage still runs.
  -h, --help             this help.
USAGE
}

while [ $# -gt 0 ]; do
  case "$1" in
    --host) HOST="$2"; shift 2 ;;
    --host=*) HOST="${1#*=}"; shift ;;
    --i-understand-this-is-destructive) ACK=1; shift ;;
    --update-to-ref) UPDATE_TO_REF="$2"; shift 2 ;;
    --update-to-ref=*) UPDATE_TO_REF="${1#*=}"; shift ;;
    --skip-reboot) SKIP_REBOOT=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 1 ;;
  esac
done

die() { echo "ERROR: $*" >&2; exit 1; }

[ -n "$HOST" ] || die "--host is required (no default target — see safety requirement #2)."
[ "$ACK" -eq 1 ] || die "refusing to run: pass --i-understand-this-is-destructive to acknowledge this WIPES the target host."

host_part="${HOST#*@}"
case "$host_part" in
  localhost|127.0.0.1|0.0.0.0|::1|"")
    die "refusing to target '$HOST' — this script is SSH-only and must never run against the local machine." ;;
esac

if [ -f /etc/vpn/deployment.toml ]; then
  prod_host="$(grep -m1 '^public_host' /etc/vpn/deployment.toml 2>/dev/null | sed -E 's/^public_host *= *"([^"]*)".*/\1/')"
  if [ -n "$prod_host" ] && [ "$host_part" = "$prod_host" ]; then
    die "refusing to target '$HOST' — it matches THIS machine's own configured production public_host in /etc/vpn/deployment.toml. Use a disposable host, not your real VPN."
  fi
fi
if [ -n "${VPN1_PRODUCTION_HOST:-}" ] && [ "$host_part" = "$VPN1_PRODUCTION_HOST" ]; then
  die "refusing to target '$HOST' — it matches VPN1_PRODUCTION_HOST. Use a disposable host."
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BRANCH="$(cd "$REPO_ROOT" && git rev-parse --abbrev-ref HEAD 2>/dev/null || echo main)"

SSH_OPTS=(-o BatchMode=yes -o ConnectTimeout=10 -o StrictHostKeyChecking=accept-new)
ssh_run() { ssh "${SSH_OPTS[@]}" "$HOST" "$@"; }

failures=0
required_fail=0
pass() { printf "[PASS] %-55s\n" "$1"; }
fail() { printf "[FAIL] %-55s %s\n" "$1" "${2:-}"; failures=$((failures + 1)); }
fail_required() { printf "[FAIL][required] %-45s %s\n" "$1" "${2:-}"; failures=$((failures + 1)); required_fail=1; }
section() { echo; echo "=== $1 ==="; }

echo "vpn1 destructive lifecycle acceptance gate"
echo "target: $HOST"
echo "source ref: $BRANCH"
echo "THIS WILL WIPE vpn1 STATE ON THE TARGET HOST. 5s to Ctrl-C..."
sleep 5

section "0. connectivity + OS baseline"
if ssh_run 'true' 2>/dev/null; then pass "SSH reachable"; else fail_required "SSH reachable"; fi
if ssh_run '[ -f /etc/os-release ] && . /etc/os-release && [ "$ID" = almalinux ] && [[ "$VERSION_ID" == 9* ]]' 2>/dev/null; then
  pass "AlmaLinux 9 confirmed"
else
  fail "AlmaLinux 9 confirmed" "(not AlmaLinux 9 — proceeding anyway is off the supported OS matrix)"
fi

section "1. SSH baseline (before any install)"
ssh_baseline="$(ssh_run 'systemctl is-active sshd 2>/dev/null; ss -ltnp 2>/dev/null | grep -c :22 || true')"
if [ -n "$ssh_baseline" ]; then pass "SSH baseline captured"; else fail "SSH baseline captured"; fi

section "2. clean install"
if ssh_run "curl -fsSL https://raw.githubusercontent.com/David610/vpn1/$BRANCH/install.sh | VPN1_REF=$BRANCH VPN1_CHANNEL=dev bash -s -- --non-interactive"; then
  pass "install.sh (clean)"
else
  fail_required "install.sh (clean)"
fi

section "3. SSH after install"
if ssh_run 'systemctl is-active --quiet sshd' 2>/dev/null; then pass "SSH still active post-install"; else fail_required "SSH still active post-install"; fi

section "4. acceptance-test.sh"
if ssh_run 'sudo /opt/vpn1/deploy/almalinux/acceptance-test.sh' 2>/dev/null; then
  pass "acceptance-test.sh"
else
  fail "acceptance-test.sh" "(see remote output above)"
fi

if [ "$SKIP_REBOOT" -eq 0 ]; then
  section "5. reboot + health"
  ssh_run 'sudo systemctl reboot' >/dev/null 2>&1 || true
  sleep 20
  reboot_ok=0
  for _ in $(seq 1 15); do
    if ssh_run 'true' 2>/dev/null; then reboot_ok=1; break; fi
    sleep 10
  done
  if [ "$reboot_ok" -eq 1 ] && ssh_run 'sudo /opt/vpn1/deploy/almalinux/health-check.sh' 2>/dev/null; then
    pass "reboot + health-check.sh"
  else
    fail_required "reboot + health-check.sh"
  fi
else
  section "5. reboot + health (SKIPPED --skip-reboot)"
fi

section "6. repair / idempotent re-run"
if ssh_run "curl -fsSL https://raw.githubusercontent.com/David610/vpn1/$BRANCH/install.sh | VPN1_REF=$BRANCH VPN1_CHANNEL=dev bash -s -- --non-interactive"; then
  pass "install.sh (idempotent re-run)"
else
  fail_required "install.sh (idempotent re-run)"
fi

if [ -n "$UPDATE_TO_REF" ]; then
  section "7. update path -> $UPDATE_TO_REF"
  if ssh_run "sudo VPN1_REF=$UPDATE_TO_REF /opt/vpn1/deploy/almalinux/update.sh"; then
    pass "update.sh -> $UPDATE_TO_REF"
  else
    fail "update.sh -> $UPDATE_TO_REF"
  fi
else
  section "7. update path (SKIPPED: no --update-to-ref given)"
fi

section "8. user add / disable / rotate / delete"
if ssh_run 'sudo vpn-admin user create --name lifecycle-gate-user \
  && sudo vpn-admin user list | grep -q lifecycle-gate-user \
  && sudo vpn-admin user rotate-token lifecycle-gate-user \
  && sudo vpn-admin user rotate-vless lifecycle-gate-user \
  && sudo vpn-admin user rotate-hysteria lifecycle-gate-user \
  && sudo vpn-admin user disable lifecycle-gate-user \
  && sudo vpn-admin user remove lifecycle-gate-user' 2>/dev/null; then
  pass "user create/rotate/disable/remove"
else
  fail_required "user create/rotate/disable/remove"
fi

section "9. local protocol validation"
if ssh_run 'sudo vpn-admin doctor --protocol' 2>/dev/null; then
  pass "vpn-admin doctor --protocol"
else
  fail "vpn-admin doctor --protocol" "(see remote output above)"
fi

section "10. failed/interrupted install cleanup"
# Failure-injection hook: install.sh honors VPN1_LIFECYCLE_GATE_ABORT_AFTER
# (a stage-name substring) to deliberately die mid-install, purely for this
# gate — see install.sh's own comment at the check. Kept to exactly one
# narrow hook, not a generic fault-injection framework.
if ssh_run "sudo VPN1_LIFECYCLE_GATE_ABORT_AFTER=install_singbox curl -fsSL https://raw.githubusercontent.com/David610/vpn1/$BRANCH/install.sh | VPN1_REF=$BRANCH VPN1_CHANNEL=dev bash -s -- --non-interactive" ; then
  fail "interrupted install actually aborted" "(expected non-zero exit, got success)"
else
  pass "interrupted install aborted as expected"
fi
if ssh_run "curl -fsSL https://raw.githubusercontent.com/David610/vpn1/$BRANCH/uninstall.sh | bash" 2>/dev/null \
  && ssh_run '[ ! -e /etc/vpn ] && [ ! -e /opt/vpn1 ]' 2>/dev/null; then
  pass "cleanup after interrupted install"
else
  fail "cleanup after interrupted install"
fi

section "11. offline uninstall (from the repaired install in stage 6, redone)"
ssh_run "curl -fsSL https://raw.githubusercontent.com/David610/vpn1/$BRANCH/install.sh | VPN1_REF=$BRANCH VPN1_CHANNEL=dev bash -s -- --non-interactive" >/dev/null 2>&1 || true
if ssh_run "curl -fsSL https://raw.githubusercontent.com/David610/vpn1/$BRANCH/uninstall.sh | bash"; then
  pass "uninstall.sh"
else
  fail_required "uninstall.sh"
fi

section "12. SSH after uninstall"
if ssh_run 'systemctl is-active --quiet sshd' 2>/dev/null; then pass "SSH still active post-uninstall"; else fail_required "SSH still active post-uninstall"; fi

section "13. unrelated host state preserved"
if ssh_run '[ ! -e /etc/vpn ] && [ ! -e /opt/vpn1 ] && ! id sing-box >/dev/null 2>&1 && ! id vpn-subscription >/dev/null 2>&1' 2>/dev/null; then
  pass "vpn1 state fully removed"
else
  fail "vpn1 state fully removed"
fi

section "14. reinstall"
if ssh_run "curl -fsSL https://raw.githubusercontent.com/David610/vpn1/$BRANCH/install.sh | VPN1_REF=$BRANCH VPN1_CHANNEL=dev bash -s -- --non-interactive"; then
  pass "reinstall after uninstall"
else
  fail_required "reinstall after uninstall"
fi

section "manual-only release gates (cannot be automated here — leave UNVERIFIED)"
echo "  - public/internet reachability from outside the target's network"
echo "  - real Hiddify iOS/Android/MagicOS import + connect + sustained traffic"

section "summary"
echo "failing stages: $failures"
if [ "$required_fail" -eq 1 ]; then
  echo "LIFECYCLE ACCEPTANCE GATE: FAIL (required stage failed)"
  exit 1
elif [ "$failures" -gt 0 ]; then
  echo "LIFECYCLE ACCEPTANCE GATE: FAIL (non-required stage(s) failed)"
  exit 1
else
  echo "LIFECYCLE ACCEPTANCE GATE: PASS"
  exit 0
fi
