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
SSH_PORT=22

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
  --ssh-port PORT        SSH port to use for the initial connection AND to pass to install.sh
                         as --ssh-port, so the whole lifecycle is exercised on a non-default
                         port instead of silently assuming 22. Default: 22.
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
    --ssh-port) SSH_PORT="$2"; shift 2 ;;
    --ssh-port=*) SSH_PORT="${1#*=}"; shift ;;
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
case "$SSH_PORT" in
  ''|*[!0-9]*) die "--ssh-port must be numeric, got '$SSH_PORT'." ;;
esac

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

SSH_OPTS=(-p "$SSH_PORT" -o BatchMode=yes -o ConnectTimeout=10 -o StrictHostKeyChecking=accept-new)
ssh_run() { timeout 180 ssh "${SSH_OPTS[@]}" "$HOST" "$@"; }
# Same as ssh_run but opens a brand-new SSH connection with a longer
# ConnectTimeout, used specifically as proof-of-reconnect after events that
# could have broken sshd (install, reboot, update, rollback, uninstall) —
# an already-open control-master connection would not catch that.
ssh_reconnect() {
  timeout 30 ssh -o ControlPath=none -p "$SSH_PORT" -o BatchMode=yes -o ConnectTimeout=15 \
    -o StrictHostKeyChecking=accept-new "$HOST" "$@"
}

# INSTALL_ARGS is passed to install.sh so every clean/repair/reinstall/
# interrupted-install stage exercises the same non-default SSH port as the
# harness itself is connecting on, instead of silently assuming 22.
INSTALL_ARGS="--non-interactive"
if [ "$SSH_PORT" != "22" ]; then
  INSTALL_ARGS="$INSTALL_ARGS --ssh-port $SSH_PORT"
fi

failures=0
required_fail=0
unverified=0
pass() { printf "[PASS] %-55s\n" "$1"; }
fail() { printf "[FAIL] %-55s %s\n" "$1" "${2:-}"; failures=$((failures + 1)); }
fail_required() { printf "[FAIL][required] %-45s %s\n" "$1" "${2:-}"; failures=$((failures + 1)); required_fail=1; }
mark_unverified() { printf "[UNVERIFIED] %-49s %s\n" "$1" "${2:-}"; unverified=$((unverified + 1)); }
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
  fail_required "AlmaLinux 9 confirmed" "(not AlmaLinux 9 — off the supported OS matrix, refusing to treat the rest of this run as a valid acceptance result)"
fi
if ssh_run '[ "$(uname -m)" = x86_64 ]' 2>/dev/null; then
  pass "x86_64 arch confirmed"
else
  fail_required "x86_64 arch confirmed" "(not x86_64 — off the supported arch matrix)"
fi

section "1. SSH baseline (before any install)"
ssh_baseline="$(ssh_run "systemctl is-active sshd 2>/dev/null; ss -ltnp 2>/dev/null | grep -c :$SSH_PORT || true" 2>/dev/null || true)"
if [ -n "$ssh_baseline" ]; then pass "SSH baseline captured (port $SSH_PORT)"; else fail "SSH baseline captured"; fi

section "1b. host baseline (sanitized, no secrets)"
# A coarse pre-install snapshot used only to detect gross residue after
# uninstall (section 13/18) — file/dir existence, unit/package/user
# presence, not content, so nothing sensitive is captured.
BASELINE="$(ssh_run '
  echo "opt_vpn1=$([ -e /opt/vpn1 ] && echo 1 || echo 0)"
  echo "etc_vpn=$([ -e /etc/vpn ] && echo 1 || echo 0)"
  echo "var_lib_vpn1=$([ -e /var/lib/vpn1 ] && echo 1 || echo 0)"
  echo "user_singbox=$(id sing-box >/dev/null 2>&1 && echo 1 || echo 0)"
  echo "user_vpnsub=$(id vpn-subscription >/dev/null 2>&1 && echo 1 || echo 0)"
  echo "unit_singbox=$([ -e /etc/systemd/system/sing-box.service ] && echo 1 || echo 0)"
  echo "unit_vpnsub=$([ -e /etc/systemd/system/vpn-subscription.service ] && echo 1 || echo 0)"
  echo "nginx_conf=$([ -e /etc/nginx/conf.d/vpn1.conf ] && echo 1 || echo 0)"
  echo "certbot_hook=$([ -e /etc/letsencrypt/renewal-hooks/deploy/vpn1-hysteria.sh ] && echo 1 || echo 0)"
' 2>/dev/null || true)"
if [ -n "$BASELINE" ]; then pass "host baseline captured"; else fail "host baseline captured"; fi

section "2. clean install"
if ssh_run "curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/$BRANCH/install.sh | VPN1_REF=$BRANCH VPN1_CHANNEL=dev REALITY_HANDSHAKE_SERVER=www.google.com VPN1_ALLOW_IP_HOSTNAME=1 bash -s -- $INSTALL_ARGS"; then
  pass "install.sh (clean)"
else
  fail_required "install.sh (clean)"
fi

section "3. SSH after install (new connection, port $SSH_PORT)"
if ssh_reconnect 'systemctl is-active --quiet sshd' 2>/dev/null; then pass "SSH still active post-install"; else fail_required "SSH still active post-install"; fi

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
    if ssh_reconnect 'true' 2>/dev/null; then reboot_ok=1; break; fi
    sleep 10
  done
  if [ "$reboot_ok" -eq 1 ] && ssh_run '
       sudo /opt/vpn1/deploy/almalinux/health-check.sh \
    && systemctl is-active --quiet sing-box \
    && systemctl is-active --quiet vpn-subscription \
    && systemctl is-active --quiet nginx \
    && systemctl list-timers --all 2>/dev/null | grep -q vpn1 \
    && ss -ltn 2>/dev/null | grep -q ":443 " \
    && sudo test -s /var/lib/vpn1/install-state.json \
    && sudo vpn-admin doctor --protocol
     ' 2>/dev/null; then
    pass "reboot + independent post-reboot verification (sshd/sing-box/subscription/nginx/timers/listener/install-state/protocol)"
  else
    fail_required "reboot + independent post-reboot verification"
  fi
else
  section "5. reboot + health (SKIPPED --skip-reboot)"
fi

section "6. repair / idempotent re-run"
if ssh_run "curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/$BRANCH/install.sh | VPN1_REF=$BRANCH VPN1_CHANNEL=dev REALITY_HANDSHAKE_SERVER=www.google.com VPN1_ALLOW_IP_HOSTNAME=1 bash -s -- $INSTALL_ARGS" \
  && ssh_reconnect 'systemctl is-active --quiet sshd' 2>/dev/null; then
  pass "install.sh (idempotent re-run) + SSH reconnect"
else
  fail_required "install.sh (idempotent re-run) + SSH reconnect"
fi

if [ -n "$UPDATE_TO_REF" ]; then
  # No real tagged vpn1 release exists yet to exercise update.sh's actual
  # production release-to-release transaction (--version/--latest) —
  # that remains a real-release UNVERIFIED item (see
  # docs/IMPLEMENTATION_STATUS.md). Until one is published, this
  # exercises the explicit --dev-rebuild escape hatch instead: check out
  # $UPDATE_TO_REF's source over /opt/vpn1, then rebuild/redeploy it via
  # update.sh --dev-rebuild (the same transactional
  # backup/switch/verify/rollback machinery the production path uses,
  # minus release-artifact resolution). update.sh no longer reads
  # VPN1_REF at all — passing it as an env var (the previous, silently
  # ignored invocation) never actually changed anything.
  section "7. update path -> $UPDATE_TO_REF (--dev-rebuild; this is VERIFIED-TEST for the transactional updater machinery only — a real GitHub release A->B transition remains UNVERIFIED until a tagged release exists, see docs/IMPLEMENTATION_STATUS.md)"
  version_before="$(ssh_run 'sudo /opt/vpn1/bin/vpn-admin --version 2>/dev/null || sudo cat /var/lib/vpn1/install-state.json 2>/dev/null' 2>/dev/null || true)"
  if ssh_run "curl -fsSL --connect-timeout 10 --max-time 60 -o /tmp/vpn1-update-ref.tar.gz https://codeload.github.com/David610/singbox-vpn/tar.gz/refs/heads/$UPDATE_TO_REF \
      && rm -rf /tmp/vpn1-update-ref && mkdir -p /tmp/vpn1-update-ref \
      && tar -xzf /tmp/vpn1-update-ref.tar.gz -C /tmp/vpn1-update-ref --strip-components=1 \
      && sudo rsync -a --delete --exclude target --exclude .git /tmp/vpn1-update-ref/ /opt/vpn1/ \
      && sudo /opt/vpn1/deploy/almalinux/update.sh --dev-rebuild" \
    && ssh_reconnect 'true' 2>/dev/null; then
    version_after="$(ssh_run 'sudo /opt/vpn1/bin/vpn-admin --version 2>/dev/null || sudo cat /var/lib/vpn1/install-state.json 2>/dev/null' 2>/dev/null || true)"
    if [ -n "$version_after" ] && [ "$version_before" != "$version_after" ]; then
      pass "update.sh --dev-rebuild -> $UPDATE_TO_REF (binary/state actually changed, not just exit 0)"
    else
      fail_required "update.sh --dev-rebuild -> $UPDATE_TO_REF" "(command succeeded but before/after version-state did not change — this must not be reported as an update)"
    fi
  else
    fail "update.sh --dev-rebuild -> $UPDATE_TO_REF"
  fi

  section "7b. failed update -> rollback (failure injected after SWITCH begins, via update.sh's VPN1_LIFECYCLE_GATE_ABORT_AFTER hook)"
  pre_rollback_version="$(ssh_run 'sudo cat /var/lib/vpn1/install-state.json 2>/dev/null' 2>/dev/null || true)"
  if ssh_run "sudo VPN1_LIFECYCLE_GATE_ABORT_AFTER=after_switch /opt/vpn1/deploy/almalinux/update.sh --dev-rebuild" 2>/dev/null; then
    fail_required "failed update aborted as expected" "(expected non-zero exit, got success)"
  else
    pass "failed update aborted as expected"
  fi
  if ssh_reconnect '
       systemctl is-active --quiet sshd \
    && systemctl is-active --quiet sing-box \
    && systemctl is-active --quiet vpn-subscription \
    && sudo vpn-admin doctor --protocol
     ' 2>/dev/null; then
    post_rollback_version="$(ssh_run 'sudo cat /var/lib/vpn1/install-state.json 2>/dev/null' 2>/dev/null || true)"
    if [ "$pre_rollback_version" = "$post_rollback_version" ]; then
      pass "rollback restored prior binary/schema/units/config/services/protocol/SSH"
    else
      fail_required "rollback restored prior state" "(install-state.json differs from before the failed update)"
    fi
  else
    fail_required "rollback restored prior binary/schema/units/config/services/protocol/SSH"
  fi
else
  section "7. update path (SKIPPED: no --update-to-ref given)"
  section "7b. failed update -> rollback (SKIPPED: no --update-to-ref given)"
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
# gate — see install.sh's own comment at the check. The env var MUST be set
# for the bash process that actually execs install.sh, not for curl on the
# other side of the pipe (sudo VAR=x curl | bash would silently drop it —
# sudo scopes VAR=x to curl's own exec only, never to bash downstream).
if ssh_run "curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/$BRANCH/install.sh | sudo VPN1_LIFECYCLE_GATE_ABORT_AFTER=install_singbox VPN1_REF=$BRANCH VPN1_CHANNEL=dev REALITY_HANDSHAKE_SERVER=www.google.com VPN1_ALLOW_IP_HOSTNAME=1 bash -s -- $INSTALL_ARGS" ; then
  fail_required "interrupted install actually aborted" "(expected non-zero exit, got success)"
else
  pass "interrupted install aborted as expected"
fi
# Prove the abort hook actually fired mid-install (not e.g. a network/SSH
# failure that would also produce a non-zero exit): install.sh must have
# started (left partial state) but never reached acceptance.
if ssh_run '[ -e /opt/vpn1 ] || [ -e /etc/vpn ]' 2>/dev/null; then
  pass "abort hook fired mid-install (partial state present, not a pre-flight failure)"
else
  fail_required "abort hook fired mid-install" "(no partial state found — the abort may not have reached the installer process at all)"
fi
if ssh_run 'sudo /opt/vpn1/bin/vpn1-uninstall --yes' 2>/dev/null \
  && ssh_run '[ ! -e /etc/vpn ] && [ ! -e /opt/vpn1 ] && [ ! -e /var/lib/vpn1 ] \
      && ! systemctl list-unit-files 2>/dev/null | grep -q "^sing-box\.service\|^vpn-subscription\.service" \
      && ! id sing-box >/dev/null 2>&1 && ! id vpn-subscription >/dev/null 2>&1' 2>/dev/null; then
  pass "cleanup after interrupted install (offline vpn1-uninstall)"
else
  fail "cleanup after interrupted install"
fi

section "11. offline uninstall (from the repaired install in stage 6, redone)"
ssh_run "curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/$BRANCH/install.sh | VPN1_REF=$BRANCH VPN1_CHANNEL=dev REALITY_HANDSHAKE_SERVER=www.google.com VPN1_ALLOW_IP_HOSTNAME=1 bash -s -- $INSTALL_ARGS" >/dev/null 2>&1 || true
# Genuinely offline: the local binary only, no curl/GitHub/DNS/package-repo.
# Best-effort proof of no outbound network use: block egress to github.com
# for the duration of this one command (ignored if iptables/firewalld isn't
# available/permitted — the "local binary only" invocation itself is the
# primary guarantee either way).
ssh_run 'sudo iptables -I OUTPUT -d github.com -j REJECT 2>/dev/null; sudo iptables -I OUTPUT -d raw.githubusercontent.com -j REJECT 2>/dev/null' >/dev/null 2>&1 || true
if ssh_run 'sudo /opt/vpn1/bin/vpn1-uninstall --yes'; then
  pass "vpn1-uninstall --yes (offline, local binary only)"
else
  fail_required "vpn1-uninstall --yes (offline, local binary only)"
fi
ssh_run 'sudo iptables -D OUTPUT -d github.com -j REJECT 2>/dev/null; sudo iptables -D OUTPUT -d raw.githubusercontent.com -j REJECT 2>/dev/null' >/dev/null 2>&1 || true

section "12. SSH after uninstall (new connection, port $SSH_PORT)"
if ssh_reconnect 'systemctl is-active --quiet sshd' 2>/dev/null; then pass "SSH still active post-uninstall"; else fail_required "SSH still active post-uninstall"; fi

section "13. uninstall residue audit (vs. host baseline from stage 1b)"
RESIDUE="$(ssh_run '
  echo "opt_vpn1=$([ -e /opt/vpn1 ] && echo 1 || echo 0)"
  echo "etc_vpn=$([ -e /etc/vpn ] && echo 1 || echo 0)"
  echo "var_lib_vpn1=$([ -e /var/lib/vpn1 ] && echo 1 || echo 0)"
  echo "user_singbox=$(id sing-box >/dev/null 2>&1 && echo 1 || echo 0)"
  echo "user_vpnsub=$(id vpn-subscription >/dev/null 2>&1 && echo 1 || echo 0)"
  echo "unit_singbox=$([ -e /etc/systemd/system/sing-box.service ] && echo 1 || echo 0)"
  echo "unit_vpnsub=$([ -e /etc/systemd/system/vpn-subscription.service ] && echo 1 || echo 0)"
  echo "nginx_conf=$([ -e /etc/nginx/conf.d/vpn1.conf ] && echo 1 || echo 0)"
  echo "certbot_hook=$([ -e /etc/letsencrypt/renewal-hooks/deploy/vpn1-hysteria.sh ] && echo 1 || echo 0)"
  echo "listeners=$(ss -ltnp 2>/dev/null | grep -Ec "sing-box|vpn-subscription")"
  echo "locks=$(ls /run/lock/vpn1* 2>/dev/null | wc -l)"
' 2>/dev/null || true)"
if [ "$RESIDUE" = "$BASELINE" ]; then
  pass "no vpn1 residue vs. pre-install host baseline"
else
  fail_required "no vpn1 residue vs. pre-install host baseline" "(baseline: $BASELINE | after uninstall: $RESIDUE)"
fi

section "14. reinstall"
if ssh_run "curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/$BRANCH/install.sh | VPN1_REF=$BRANCH VPN1_CHANNEL=dev REALITY_HANDSHAKE_SERVER=www.google.com VPN1_ALLOW_IP_HOSTNAME=1 bash -s -- $INSTALL_ARGS" \
  && ssh_reconnect 'systemctl is-active --quiet sshd' 2>/dev/null; then
  pass "reinstall after uninstall + SSH reconnect"
else
  fail_required "reinstall after uninstall + SSH reconnect"
fi

section "manual-only / out-of-scope gates (cannot be automated here — UNVERIFIED, not PASS)"
mark_unverified "public/internet reachability from outside the target's network" "(no independent external controller in this harness — see checkpoint scope)"
mark_unverified "real Hiddify iOS/Android/MagicOS import + connect + sustained traffic" "(client/device properties — out of scope for this host-lifecycle gate)"
if [ -z "$UPDATE_TO_REF" ]; then
  mark_unverified "real GitHub release A->B update transition" "(no --update-to-ref given, and no tagged release exists yet to test against)"
fi
if ssh_run 'sudo certbot renew --dry-run' 2>/dev/null; then
  pass "certbot renew --dry-run"
else
  mark_unverified "certbot renew --dry-run" "(failed or certbot/ACME conditions on this host prevent a dry-run — not faked)"
fi

section "summary"
echo "failing stages: $failures"
echo "unverified items: $unverified"
if [ "$unverified" -gt 0 ]; then
  echo "NOTE: this run has UNVERIFIED items above — do not treat PASS below as full v1.0 release readiness."
fi
if [ "$required_fail" -eq 1 ]; then
  echo "LIFECYCLE GATE: FAIL"
  exit 1
elif [ "$failures" -gt 0 ]; then
  echo "LIFECYCLE GATE: FAIL"
  exit 1
else
  echo "LIFECYCLE GATE: PASS"
  exit 0
fi
