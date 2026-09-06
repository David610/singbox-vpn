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
# What this does: drives install.sh -> acceptance-test.sh (services
# healthy) -> reboot -> health-check.sh -> re-run install.sh (idempotent
# repair) -> a deliberately interrupted install + offline-uninstall
# cleanup (failure-injection hook, see below) -> reinstall -> create a
# test user -> vpn-admin doctor -> vpn-admin doctor --protocol
# --require-protocol (real REALITY handshake proof) ->
# deploy/lib/vpn-benchmark.sh (real Hysteria2 handshake+transfer proof)
# -> SIGKILL sing-box and verify systemd recovers it within a bounded
# time -> exhaust sing-box's StartLimitBurst on purpose and prove
# vpn-service-watchdog recovers it from a parked FAILED state (and that
# `vpn doctor`/`vpn status` actually report FAILED, not just "not
# active") -> prove a deliberate `systemctl stop` still behaves
# normally and is never "recovered" by the watchdog -> re-verify the
# protocol -> user rotate/disable/remove sanity on a scratch user ->
# update.sh (if a second version/fixture is given) -> a deliberately
# failed update + rollback proof -> vpn-admin
# backup -> uninstall.sh (complete) -> reinstall -> vpn-admin restore ->
# verify the restored user/key state works -> doctor/protocol checks
# again -> final uninstall.sh -> SSH/host-state residue checks, over
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
#      if present LOCALLY, and against $SINGBOX_VPN_PRODUCTION_HOST if set) —
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
UPDATE_TO_VERSION=""
VERSION=""
SKIP_REBOOT=0
SSH_PORT=22
DOMAIN=""

usage() {
  cat <<'USAGE'
deploy/almalinux/lifecycle-acceptance.sh --host user@disposable-host --i-understand-this-is-destructive [options]

Required:
  --host USER@HOST                     disposable AlmaLinux 9 x86_64 target, reached via SSH.
                                        Refuses localhost/127.0.0.1/::1 and this repo's own
                                        configured production PUBLIC_HOST.
  --i-understand-this-is-destructive   explicit acknowledgement; this WIPES the target host's
                                        singbox-vpn state (and, on failure-injection stages, more).

Options:
  --ssh-port PORT        SSH port to use for the initial connection AND to pass to install.sh
                         as --ssh-port, so the whole lifecycle is exercised on a non-default
                         port instead of silently assuming 22. Default: 22.
  --update-to-ref REF   after the clean install, also exercise update.sh against REF
                         (branch/tag) as a second version — skipped if not given.
  --update-to-version VERSION
                         exercise the checksum-verified production updater to a
                         second immutable release and its rollback path.
  --version VERSION      exercise the checksum-verified stable release path for VERSION
                         (for example v0.1.2). Without this option the harness uses the
                         explicitly unverified development branch and CANNOT produce a
                         production-acceptance result.
  --skip-reboot          skip the reboot+health stage (e.g. host cannot be rebooted by this
                         SSH session's caller). Every other stage still runs.
  --domain HOST          use this hostname for every install.sh/reinstall this run performs
                         instead of letting install.sh auto-detect a *.sslip.io hostname from
                         the target's public IP. Each fresh install in a single run requests a
                         fresh Let's Encrypt certificate (stages 2, 8, and 21 can each do so);
                         Let's Encrypt's real-world rate limit is 5 certificates per exact
                         hostname per 168h, so repeated full runs against the same auto-detected
                         sslip.io hostname (tied to the target's IP, so it never changes) can
                         exhaust that limit and fail every subsequent fresh install with
                         "too many certificates already issued" until it resets. Point this at a
                         domain you control instead to avoid sharing a rate-limit budget with
                         every other sslip.io run against this same IP.
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
    --update-to-version) UPDATE_TO_VERSION="$2"; shift 2 ;;
    --update-to-version=*) UPDATE_TO_VERSION="${1#*=}"; shift ;;
    --version) VERSION="$2"; shift 2 ;;
    --version=*) VERSION="${1#*=}"; shift ;;
    --skip-reboot) SKIP_REBOOT=1; shift ;;
    --domain) DOMAIN="$2"; shift 2 ;;
    --domain=*) DOMAIN="${1#*=}"; shift ;;
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
if [ -n "$VERSION" ] && [[ ! "$VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
  die "--version must be an immutable vX.Y.Z release tag, got '$VERSION'."
fi
if [ -n "$UPDATE_TO_VERSION" ] && [[ ! "$UPDATE_TO_VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
  die "--update-to-version must be an immutable vX.Y.Z release tag, got '$UPDATE_TO_VERSION'."
fi
[ -z "$UPDATE_TO_REF" ] || [ -z "$UPDATE_TO_VERSION" ] \
  || die "--update-to-ref and --update-to-version are mutually exclusive."

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
if [ -n "${SINGBOX_VPN_PRODUCTION_HOST:-}" ] && [ "$host_part" = "$SINGBOX_VPN_PRODUCTION_HOST" ]; then
  die "refusing to target '$HOST' — it matches SINGBOX_VPN_PRODUCTION_HOST. Use a disposable host."
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BRANCH="$(cd "$REPO_ROOT" && git rev-parse --abbrev-ref HEAD 2>/dev/null || echo main)"
if [ -n "$VERSION" ]; then
  BOOTSTRAP_REF="main"
  INSTALL_SOURCE_ENV="SINGBOX_VPN_VERSION=$VERSION"
  ACCEPTANCE_SCOPE="PRODUCTION RELEASE $VERSION"
else
  BOOTSTRAP_REF="$BRANCH"
  INSTALL_SOURCE_ENV="SINGBOX_VPN_REF=$BRANCH SINGBOX_VPN_CHANNEL=dev SINGBOX_VPN_ALLOW_UNVERIFIED_DEV=1"
  ACCEPTANCE_SCOPE="DEVELOPMENT BRANCH $BRANCH (NOT production acceptance)"
fi

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
if [ -n "$DOMAIN" ]; then
  INSTALL_ARGS="$INSTALL_ARGS --domain $DOMAIN"
fi

failures=0
required_fail=0
unverified=0
pass() { printf "[PASS] %-55s\n" "$1"; }
fail() { printf "[FAIL] %-55s %s\n" "$1" "${2:-}"; failures=$((failures + 1)); }
fail_required() { printf "[FAIL][required] %-45s %s\n" "$1" "${2:-}"; failures=$((failures + 1)); required_fail=1; }
mark_unverified() { printf "[UNVERIFIED] %-49s %s\n" "$1" "${2:-}"; unverified=$((unverified + 1)); }
section() { echo; echo "=== $1 ==="; }

echo "singbox-vpn destructive lifecycle acceptance gate"
echo "target: $HOST"
echo "acceptance scope: $ACCEPTANCE_SCOPE"
echo "THIS WILL WIPE singbox-vpn STATE ON THE TARGET HOST. 5s to Ctrl-C..."
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
  echo "opt_singbox-vpn=$([ -e /opt/singbox-vpn ] && echo 1 || echo 0)"
  echo "etc_vpn=$([ -e /etc/vpn ] && echo 1 || echo 0)"
  echo "var_lib_singbox-vpn=$([ -e /var/lib/singbox-vpn ] && echo 1 || echo 0)"
  echo "user_singbox=$(id sing-box >/dev/null 2>&1 && echo 1 || echo 0)"
  echo "user_vpnsub=$(id vpn-subscription >/dev/null 2>&1 && echo 1 || echo 0)"
  echo "unit_singbox=$([ -e /etc/systemd/system/sing-box.service ] && echo 1 || echo 0)"
  echo "unit_vpnsub=$([ -e /etc/systemd/system/vpn-subscription.service ] && echo 1 || echo 0)"
  echo "nginx_conf=$([ -e /etc/nginx/conf.d/vpn-subscription.conf ] && echo 1 || echo 0)"
  echo "certbot_hook=$([ -e /etc/letsencrypt/renewal-hooks/deploy/singbox-vpn-hysteria.sh ] && echo 1 || echo 0)"
  echo "listeners=$(ss -ltnp 2>/dev/null | grep -Ec "sing-box|vpn-subscription")"
  echo "locks=$(ls /run/lock/singbox-vpn* 2>/dev/null | wc -l)"
' 2>/dev/null || true)"
if [ -n "$BASELINE" ]; then pass "host baseline captured"; else fail "host baseline captured"; fi

section "2. clean install"
if ssh_run "curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/$BOOTSTRAP_REF/install.sh | $INSTALL_SOURCE_ENV REALITY_HANDSHAKE_SERVER=www.google.com SINGBOX_VPN_ALLOW_IP_HOSTNAME=1 bash -s -- $INSTALL_ARGS"; then
  pass "install.sh (clean)"
else
  fail_required "install.sh (clean)"
fi

section "3. SSH after install (new connection, port $SSH_PORT)"
if ssh_reconnect 'systemctl is-active --quiet sshd' 2>/dev/null; then pass "SSH still active post-install"; else fail_required "SSH still active post-install"; fi

section "4. acceptance-test.sh"
if ssh_run 'sudo /opt/singbox-vpn/deploy/almalinux/acceptance-test.sh' 2>/dev/null; then
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
       sudo /opt/singbox-vpn/deploy/almalinux/health-check.sh \
    && systemctl is-active --quiet sing-box \
    && systemctl is-active --quiet vpn-subscription \
    && systemctl is-active --quiet nginx \
    && systemctl is-active --quiet vpn-expiry-reconcile.timer \
    && systemctl is-active --quiet vpn-service-watchdog.timer \
    && ss -ltn 2>/dev/null | grep -q ":443 " \
    && sudo test -s /var/lib/singbox-vpn/install-state.json \
    && sudo vpn-admin doctor --protocol
     ' 2>/dev/null; then
    pass "reboot + independent post-reboot verification (sshd/sing-box/subscription/nginx/timers incl. watchdog/listener/install-state/protocol)"
  else
    fail_required "reboot + independent post-reboot verification"
  fi
else
  section "5. reboot + health (SKIPPED --skip-reboot)"
fi

section "6. repair / idempotent re-run"
if ssh_run "curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/$BOOTSTRAP_REF/install.sh | $INSTALL_SOURCE_ENV REALITY_HANDSHAKE_SERVER=www.google.com SINGBOX_VPN_ALLOW_IP_HOSTNAME=1 bash -s -- $INSTALL_ARGS" \
  && ssh_reconnect 'systemctl is-active --quiet sshd' 2>/dev/null; then
  pass "install.sh (idempotent re-run) + SSH reconnect"
else
  fail_required "install.sh (idempotent re-run) + SSH reconnect"
fi

section "7. failed/interrupted install cleanup (scratch scenario; ends with singbox-vpn fully removed)"
# Failure-injection hook: install.sh honors SINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER
# (a stage-name substring) to deliberately die mid-install, purely for this
# gate — see install.sh's own comment at the check. The env var MUST be set
# for the bash process that actually execs install.sh, not for curl on the
# other side of the pipe (sudo VAR=x curl | bash would silently drop it —
# sudo scopes VAR=x to curl's own exec only, never to bash downstream).
if ssh_run "curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/$BOOTSTRAP_REF/install.sh | sudo SINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER=install_singbox $INSTALL_SOURCE_ENV REALITY_HANDSHAKE_SERVER=www.google.com SINGBOX_VPN_ALLOW_IP_HOSTNAME=1 bash -s -- $INSTALL_ARGS" ; then
  fail_required "interrupted install actually aborted" "(expected non-zero exit, got success)"
else
  pass "interrupted install aborted as expected"
fi
# Prove the abort hook actually fired mid-install (not e.g. a network/SSH
# failure that would also produce a non-zero exit): install.sh must have
# started (left partial state) but never reached acceptance.
if ssh_run '[ -e /opt/singbox-vpn ] || [ -e /etc/vpn ]' 2>/dev/null; then
  pass "abort hook fired mid-install (partial state present, not a pre-flight failure)"
else
  fail_required "abort hook fired mid-install" "(no partial state found — the abort may not have reached the installer process at all)"
fi
if ssh_run 'sudo /opt/singbox-vpn/bin/singbox-vpn-uninstall --yes' 2>/dev/null \
  && ssh_run '[ ! -e /etc/vpn ] && [ ! -e /opt/singbox-vpn ] && [ ! -e /var/lib/singbox-vpn ] \
      && ! systemctl list-unit-files 2>/dev/null | grep -q "^sing-box\.service\|^vpn-subscription\.service" \
      && ! id sing-box >/dev/null 2>&1 && ! id vpn-subscription >/dev/null 2>&1' 2>/dev/null; then
  pass "cleanup after interrupted install (offline singbox-vpn-uninstall)"
else
  fail "cleanup after interrupted install"
fi

section "8. reinstall after interrupted-install cleanup (back to a working baseline for the rest of this run)"
if ssh_run "curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/$BOOTSTRAP_REF/install.sh | $INSTALL_SOURCE_ENV REALITY_HANDSHAKE_SERVER=www.google.com SINGBOX_VPN_ALLOW_IP_HOSTNAME=1 bash -s -- $INSTALL_ARGS" \
  && ssh_reconnect 'systemctl is-active --quiet sshd' 2>/dev/null; then
  pass "install.sh (clean, post-cleanup) + SSH reconnect"
else
  fail_required "install.sh (clean, post-cleanup) + SSH reconnect"
fi

section "9. create test user (persists through the backup/restore verification below)"
TEST_USER_NAME="lifecycle-test-user"
if ssh_run "sudo vpn-admin user create --name $TEST_USER_NAME --json" >/dev/null 2>&1 \
  && ssh_run "sudo vpn-admin user list | grep -q $TEST_USER_NAME" 2>/dev/null; then
  pass "test user created ($TEST_USER_NAME)"
else
  fail_required "test user created ($TEST_USER_NAME)"
fi

section "10. vpn-admin doctor (standard checks, no protocol self-test)"
if ssh_run 'sudo vpn-admin doctor' 2>/dev/null; then
  pass "vpn-admin doctor"
else
  fail_required "vpn-admin doctor"
fi

section "11. REALITY authentication proof (vpn-admin doctor --protocol --require-protocol)"
# --require-protocol turns an inconclusive/unavailable self-test into a
# hard failure instead of a warning — this is the same flag the
# installer itself uses after creating the first user (see
# apps/admin/src/main.rs), so a PASS here means what a real fresh
# install's own acceptance gate means, not a weaker check.
if PROTOCOL_OUT="$(ssh_run 'sudo vpn-admin doctor --protocol --require-protocol' 2>&1)"; then
  protocol_rc=0
else
  protocol_rc=$?
fi
if [ "$protocol_rc" -eq 0 ] && printf '%s' "$PROTOCOL_OUT" | grep -q 'completed a full handshake'; then
  pass "REALITY handshake self-test PASSED (real sing-box client, live public_key/short_id, application bytes end-to-end)"
else
  fail_required "REALITY handshake self-test (doctor --protocol --require-protocol)" "(exit=$protocol_rc; see remote output above)"
fi

section "12. Hysteria2 real handshake+transfer proof (deploy/lib/vpn-benchmark.sh)"
# Reuses the existing benchmark tool's layer-4 tunnel test: a REAL
# sing-box client, on this same host, dials this same host's own live
# Hysteria2 listener through a throwaway benchmark user (created and
# deleted by the tool itself) and transfers real bytes through it. A
# small download size keeps this bounded on a disposable VPS's uplink.
if HY_OUT="$(ssh_run "sudo /opt/singbox-vpn/deploy/lib/vpn-benchmark.sh --runs 1 --download-url 'https://speed.cloudflare.com/__down?bytes=2000000'" 2>&1)"; then
  hy_rc=0
else
  hy_rc=$?
fi
hy_block="$(printf '%s\n' "$HY_OUT" | sed -n '/^Hysteria2 protocol\/server-side overhead/,/^Assessment$/p' || true)"
hy_line="$(printf '%s\n' "$hy_block" | grep -A1 'throughput (Mbps)' | tail -1 || true)"
hy_min_mbps="$(printf '%s' "$hy_line" | grep -oE 'min=[0-9.]+' | cut -d= -f2 || true)"
if [ "$hy_rc" -eq 0 ] && ! printf '%s' "$hy_block" | grep -qE 'SKIPPED|FAILED|unavailable' \
  && [ -n "$hy_min_mbps" ] && awk -v n="$hy_min_mbps" 'BEGIN{exit !(n>0)}'; then
  pass "Hysteria2 real handshake+transfer proof (throughput: $hy_line Mbps)"
else
  fail_required "Hysteria2 real handshake+transfer proof" "(exit=$hy_rc; see benchmark output: $hy_block)"
fi

section "13. kill sing-box (SIGKILL) and verify systemd recovers it within a bounded time"
sb_pid_before="$(ssh_run 'systemctl show -p MainPID --value sing-box' 2>/dev/null || true)"
if [ -n "$sb_pid_before" ] && [ "$sb_pid_before" != "0" ] && ssh_run "sudo kill -9 $sb_pid_before" 2>/dev/null; then
  pass "SIGKILL sent to sing-box (pid $sb_pid_before)"
else
  fail_required "SIGKILL sent to sing-box"
fi
sb_recovered=0
for _ in $(seq 1 15); do
  if ssh_run 'systemctl is-active --quiet sing-box' 2>/dev/null; then sb_recovered=1; break; fi
  sleep 2
done
if [ "$sb_recovered" -eq 1 ]; then
  pass "systemd restarted sing-box within a bounded time (<=30s; unit is Restart=on-failure, RestartSec=2)"
else
  fail_required "systemd restarted sing-box within a bounded time"
fi
sb_pid_after="$(ssh_run 'systemctl show -p MainPID --value sing-box' 2>/dev/null || true)"
if [ -n "$sb_pid_after" ] && [ "$sb_pid_after" != "0" ] && [ "$sb_pid_after" != "$sb_pid_before" ]; then
  pass "sing-box MainPID changed after kill+restart (a real respawn, not a stale unit)"
else
  fail_required "sing-box MainPID changed after kill+restart" "(before=$sb_pid_before after=$sb_pid_after)"
fi

section "13b. exhaust the restart budget (StartLimitBurst) and prove vpn-service-watchdog recovers it"
# sing-box.service's StartLimitBurst=8 within StartLimitIntervalSec=300
# (see that unit file) means a unit crashing more than 8 times in 5
# minutes is marked FAILED and systemd stops retrying it automatically —
# vpn-service-watchdog.timer/.service exist specifically so that this is
# never permanent. Prove the whole chain for real, not just that the
# unit files exist: crash sing-box fast enough to actually exhaust the
# burst; confirm systemd really does give up (`is-failed`); confirm
# `vpn doctor`/`vpn status` report it; then run the watchdog directly
# (a fast, legitimate way to prove its OWN logic works, rather than
# waiting up to 5 real minutes for its timer to fire naturally — the
# timer's cadence itself is already proven separately by the reboot
# stage confirming it comes up armed) and confirm sing-box comes back.
if ssh_run '
  # Poll for MainPID actually changing rather than assuming a fixed
  # sleep lines up with RestartSec=2 — on a slow/loaded host sing-box
  # can take longer than a short fixed gap to rebind and register a new
  # PID, so a kill landing during that dead window is a no-op and
  # silently burns one of the 8 restart attempts we need to exhaust.
  # Poll fast and kill every genuinely new PID for a bounded 40s wall
  # clock instead, which comfortably clears 8 real restarts regardless
  # of how long each individual respawn takes.
  last_killed=""
  end=$(( $(date +%s) + 40 ))
  while [ "$(date +%s)" -lt "$end" ]; do
    pid="$(systemctl show -p MainPID --value sing-box)"
    if [ -n "$pid" ] && [ "$pid" != "0" ] && [ "$pid" != "$last_killed" ]; then
      sudo kill -9 "$pid" 2>/dev/null
      last_killed="$pid"
    fi
    sleep 0.2
  done
  sleep 5
  systemctl is-failed --quiet sing-box
' 2>/dev/null; then
  pass "sing-box.service reached FAILED state after exhausting StartLimitBurst (proves the burst is real, not effectively infinite)"
else
  fail_required "sing-box.service did not reach FAILED state after a fast repeated-crash burst" "(StartLimitBurst may be too generous, or something else recovered it before this check ran)"
fi

if DOCTOR_DURING_FAILURE_OUT="$(ssh_run 'sudo vpn-admin doctor' 2>&1)"; then
  doctor_during_failure_rc=0
else
  doctor_during_failure_rc=$?
fi
if [ "$doctor_during_failure_rc" -ne 0 ] && printf '%s' "$DOCTOR_DURING_FAILURE_OUT" | grep -qi 'sing-box.service is in a FAILED state'; then
  pass "vpn-admin doctor correctly reports sing-box.service as FAILED (distinct from merely 'not active')"
else
  fail_required "vpn-admin doctor did not report the FAILED sing-box.service" "(exit=$doctor_during_failure_rc; see remote output above)"
fi

STATUS_DURING_FAILURE_OUT="$(ssh_run 'sudo vpn-admin status' 2>&1 || true)"
if printf '%s' "$STATUS_DURING_FAILURE_OUT" | grep -qi 'sing-box.*failed'; then
  pass "vpn-admin status correctly reports sing-box as failed"
else
  fail_required "vpn-admin status did not report sing-box as failed" "(see remote output above)"
fi

if ssh_run 'sudo systemctl start vpn-service-watchdog.service' 2>/dev/null; then
  pass "vpn-service-watchdog.service ran without error"
else
  fail_required "vpn-service-watchdog.service ran without error"
fi

watchdog_recovered=0
for _ in $(seq 1 15); do
  if ssh_run 'systemctl is-active --quiet sing-box' 2>/dev/null; then watchdog_recovered=1; break; fi
  sleep 2
done
if [ "$watchdog_recovered" -eq 1 ]; then
  pass "vpn-service-watchdog recovered sing-box.service from its parked FAILED state (a recoverable service is never left permanently down)"
else
  fail_required "vpn-service-watchdog did not recover sing-box.service from its FAILED state"
fi

section "13c. systemctl stop still behaves normally (a deliberate stop is never treated as a failure to auto-recover)"
if ssh_run 'sudo systemctl stop sing-box' 2>/dev/null; then
  pass "systemctl stop sing-box succeeded"
else
  fail_required "systemctl stop sing-box succeeded"
fi
sleep 3
if ssh_run 'systemctl is-active --quiet sing-box' 2>/dev/null; then
  fail_required "sing-box remained active after systemctl stop" "(a deliberate stop must actually stop it)"
else
  pass "sing-box is inactive after systemctl stop (not silently auto-restarted)"
fi
if ssh_run 'systemctl is-failed --quiet sing-box' 2>/dev/null; then
  fail_required "sing-box is reported FAILED after a deliberate stop" "(a clean stop must leave it 'inactive', never 'failed' — 'failed' would wrongly make vpn-service-watchdog try to restart it)"
else
  pass "sing-box is 'inactive', not 'failed', after a deliberate stop"
fi
# Prove the watchdog genuinely leaves a deliberately-stopped unit alone
# — it only acts on units systemd itself reports as FAILED.
ssh_run 'sudo systemctl start vpn-service-watchdog.service' >/dev/null 2>&1 || true
sleep 2
if ssh_run 'systemctl is-active --quiet sing-box' 2>/dev/null; then
  fail_required "vpn-service-watchdog restarted a deliberately-stopped sing-box" "(it must only act on FAILED units, never merely-inactive ones)"
else
  pass "vpn-service-watchdog left the deliberately-stopped sing-box alone"
fi
if ssh_run 'sudo systemctl start sing-box' 2>/dev/null; then
  pass "sing-box restarted normally after the deliberate-stop test (restoring state for the rest of this run)"
else
  fail_required "sing-box restarted normally after the deliberate-stop test"
fi

section "14. protocol works after recovery (re-run doctor --protocol --require-protocol)"
if POST_RECOVERY_PROTOCOL_OUT="$(ssh_run 'sudo vpn-admin doctor --protocol --require-protocol' 2>&1)"; then
  post_recovery_rc=0
else
  post_recovery_rc=$?
fi
if [ "$post_recovery_rc" -eq 0 ] && printf '%s' "$POST_RECOVERY_PROTOCOL_OUT" | grep -q 'completed a full handshake'; then
  pass "REALITY handshake self-test still PASSES after the SIGKILL+recovery cycle"
else
  fail_required "REALITY handshake self-test after recovery" "(exit=$post_recovery_rc; see remote output above)"
fi

section "15. user rotate/disable/remove sanity (scratch user; does not touch the persisted test user above)"
# `user create --name` assigns a separate CSPRNG `id` (see cmd_user_create
# in apps/admin/src/main.rs) — every other `user` subcommand matches on
# that id, never on the name, so it must be captured from --json output
# rather than reusing the name string here.
if ssh_run '
  scratch_id="$(sudo vpn-admin user create --name lifecycle-scratch-user --json \
    | grep -o "\"id\": *\"[^\"]*\"" | head -1 | sed -E "s/.*\"([^\"]+)\"$/\1/")"
  [ -n "$scratch_id" ] \
    && sudo vpn-admin user list | grep -q "$scratch_id" \
    && sudo vpn-admin user rotate-token "$scratch_id" \
    && sudo vpn-admin user rotate-vless "$scratch_id" \
    && sudo vpn-admin user rotate-hysteria "$scratch_id" \
    && sudo vpn-admin user disable "$scratch_id" \
    && sudo vpn-admin user remove "$scratch_id"
' 2>/dev/null; then
  pass "scratch user create/rotate/disable/remove"
else
  fail_required "scratch user create/rotate/disable/remove"
fi

if [ -n "$UPDATE_TO_VERSION" ]; then
  section "16. checksum-verified production update -> $UPDATE_TO_VERSION"
  version_before="$(ssh_run 'sudo cat /var/lib/singbox-vpn/install-state.json 2>/dev/null' 2>/dev/null || true)"
  if ssh_run "sudo /opt/singbox-vpn/deploy/almalinux/update.sh --version $UPDATE_TO_VERSION" \
    && ssh_reconnect 'true' 2>/dev/null; then
    version_after="$(ssh_run 'sudo cat /var/lib/singbox-vpn/install-state.json 2>/dev/null' 2>/dev/null || true)"
    if [ -n "$version_after" ] && [ "$version_before" != "$version_after" ]; then
      pass "production update -> $UPDATE_TO_VERSION (install state changed)"
    else
      fail_required "production update -> $UPDATE_TO_VERSION" "(command succeeded but install state did not change)"
    fi
  else
    fail_required "production update -> $UPDATE_TO_VERSION"
  fi

  section "16b. injected failed production repair -> rollback proof"
  pre_rollback_version="$(ssh_run 'sudo cat /var/lib/singbox-vpn/install-state.json 2>/dev/null' 2>/dev/null || true)"
  if ssh_run "sudo SINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER=after_switch /opt/singbox-vpn/deploy/almalinux/update.sh --repair" 2>/dev/null; then
    fail_required "failed production repair aborted as expected" "(expected non-zero exit, got success)"
  else
    pass "failed production repair aborted as expected"
  fi
  if ssh_reconnect '
       systemctl is-active --quiet sshd \
    && systemctl is-active --quiet sing-box \
    && systemctl is-active --quiet vpn-subscription \
    && sudo vpn-admin doctor --protocol
     ' 2>/dev/null; then
    post_rollback_version="$(ssh_run 'sudo cat /var/lib/singbox-vpn/install-state.json 2>/dev/null' 2>/dev/null || true)"
    if [ "$pre_rollback_version" = "$post_rollback_version" ]; then
      pass "production repair rollback restored the prior working state"
    else
      fail_required "production repair rollback restored prior state" "(install-state.json differs)"
    fi
  else
    fail_required "production repair rollback left services/protocol/SSH healthy"
  fi
elif [ -n "$UPDATE_TO_REF" ]; then
  # No real tagged singbox-vpn release exists yet to exercise update.sh's actual
  # production release-to-release transaction (--version/--latest) —
  # that remains a real-release UNVERIFIED item (see
  # docs/IMPLEMENTATION_STATUS.md). Until one is published, this
  # exercises the explicit --dev-rebuild escape hatch instead: check out
  # $UPDATE_TO_REF's source over /opt/singbox-vpn, then rebuild/redeploy it via
  # update.sh --dev-rebuild (the same transactional
  # backup/switch/verify/rollback machinery the production path uses,
  # minus release-artifact resolution). update.sh no longer reads
  # SINGBOX_VPN_REF at all — passing it as an env var (the previous, silently
  # ignored invocation) never actually changed anything.
  section "16. safe update path -> $UPDATE_TO_REF (--dev-rebuild; this is VERIFIED-TEST for the transactional updater machinery only — a real GitHub release A->B transition remains UNVERIFIED until a tagged release exists, see docs/IMPLEMENTATION_STATUS.md)"
  version_before="$(ssh_run 'sudo /opt/singbox-vpn/bin/vpn-admin --version 2>/dev/null || sudo cat /var/lib/singbox-vpn/install-state.json 2>/dev/null' 2>/dev/null || true)"
  if ssh_run "curl -fsSL --connect-timeout 10 --max-time 60 -o /tmp/singbox-vpn-update-ref.tar.gz https://codeload.github.com/David610/singbox-vpn/tar.gz/refs/heads/$UPDATE_TO_REF \
      && rm -rf /tmp/singbox-vpn-update-ref && mkdir -p /tmp/singbox-vpn-update-ref \
      && tar -xzf /tmp/singbox-vpn-update-ref.tar.gz -C /tmp/singbox-vpn-update-ref --strip-components=1 \
      && sudo rsync -a --delete --exclude target --exclude .git /tmp/singbox-vpn-update-ref/ /opt/singbox-vpn/ \
      && sudo /opt/singbox-vpn/deploy/almalinux/update.sh --dev-rebuild" \
    && ssh_reconnect 'true' 2>/dev/null; then
    version_after="$(ssh_run 'sudo /opt/singbox-vpn/bin/vpn-admin --version 2>/dev/null || sudo cat /var/lib/singbox-vpn/install-state.json 2>/dev/null' 2>/dev/null || true)"
    if [ -n "$version_after" ] && [ "$version_before" != "$version_after" ]; then
      pass "update.sh --dev-rebuild -> $UPDATE_TO_REF (binary/state actually changed, not just exit 0)"
    else
      fail_required "update.sh --dev-rebuild -> $UPDATE_TO_REF" "(command succeeded but before/after version-state did not change — this must not be reported as an update)"
    fi
  else
    fail "update.sh --dev-rebuild -> $UPDATE_TO_REF"
  fi

  section "16b. injected failed update -> rollback proof (failure injected after SWITCH begins, via update.sh's SINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER hook)"
  pre_rollback_version="$(ssh_run 'sudo cat /var/lib/singbox-vpn/install-state.json 2>/dev/null' 2>/dev/null || true)"
  if ssh_run "sudo SINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER=after_switch /opt/singbox-vpn/deploy/almalinux/update.sh --dev-rebuild" 2>/dev/null; then
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
    post_rollback_version="$(ssh_run 'sudo cat /var/lib/singbox-vpn/install-state.json 2>/dev/null' 2>/dev/null || true)"
    if [ "$pre_rollback_version" = "$post_rollback_version" ]; then
      pass "rollback restored the previous working release (prior binary/schema/units/config/services/protocol/SSH)"
    else
      fail_required "rollback restored prior state" "(install-state.json differs from before the failed update)"
    fi
  else
    fail_required "rollback restored prior binary/schema/units/config/services/protocol/SSH"
  fi
else
  section "16. safe update path (SKIPPED: no --update-to-ref given)"
  section "16b. injected failed update -> rollback proof (SKIPPED: no --update-to-ref given)"
fi

section "17. create vpn backup"
BACKUP_PATH="/root/singbox-vpn-lifecycle-backup.tar"
PRE_BACKUP_USERLIST="$(ssh_run 'sudo vpn-admin user list' 2>/dev/null || true)"
if ssh_run "sudo vpn-admin backup --output $BACKUP_PATH" 2>/dev/null \
  && ssh_run "sudo test -s $BACKUP_PATH" 2>/dev/null; then
  pass "vpn-admin backup produced a non-empty archive at $BACKUP_PATH"
else
  fail_required "vpn-admin backup produced a non-empty archive"
fi

section "18. certbot renew --dry-run (while the deployment is still live, before the destructive uninstall below)"
if ssh_run 'sudo certbot renew --dry-run' 2>/dev/null; then
  pass "certbot renew --dry-run"
else
  mark_unverified "certbot renew --dry-run" "(failed or certbot/ACME conditions on this host prevent a dry-run — not faked)"
fi

section "19. uninstall completely (offline singbox-vpn-uninstall)"
# Genuinely offline: the local binary only, no curl/GitHub/DNS/package-repo.
# Best-effort proof of no outbound network use: block egress to github.com
# for the duration of this one command (ignored if iptables/firewalld isn't
# available/permitted — the "local binary only" invocation itself is the
# primary guarantee either way). $BACKUP_PATH lives under /root, which
# singbox-vpn-uninstall never touches, so the backup survives this step.
ssh_run 'sudo iptables -I OUTPUT -d github.com -j REJECT 2>/dev/null; sudo iptables -I OUTPUT -d raw.githubusercontent.com -j REJECT 2>/dev/null' >/dev/null 2>&1 || true
if ssh_run 'sudo /opt/singbox-vpn/bin/singbox-vpn-uninstall --yes'; then
  pass "singbox-vpn-uninstall --yes (offline, local binary only)"
else
  fail_required "singbox-vpn-uninstall --yes (offline, local binary only)"
fi
ssh_run 'sudo iptables -D OUTPUT -d github.com -j REJECT 2>/dev/null; sudo iptables -D OUTPUT -d raw.githubusercontent.com -j REJECT 2>/dev/null' >/dev/null 2>&1 || true

section "20. SSH after uninstall (new connection, port $SSH_PORT)"
if ssh_reconnect 'systemctl is-active --quiet sshd' 2>/dev/null; then pass "SSH still active post-uninstall"; else fail_required "SSH still active post-uninstall"; fi

section "21. reinstall from the normal one-command production path"
if ssh_run "curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/$BOOTSTRAP_REF/install.sh | $INSTALL_SOURCE_ENV REALITY_HANDSHAKE_SERVER=www.google.com SINGBOX_VPN_ALLOW_IP_HOSTNAME=1 bash -s -- $INSTALL_ARGS" \
  && ssh_reconnect 'systemctl is-active --quiet sshd' 2>/dev/null; then
  pass "reinstall after uninstall + SSH reconnect"
else
  fail_required "reinstall after uninstall + SSH reconnect"
fi

section "22. restore backup"
if ssh_run "sudo test -s $BACKUP_PATH" 2>/dev/null && ssh_run "sudo vpn-admin restore $BACKUP_PATH" 2>/dev/null; then
  pass "vpn-admin restore applied the backup archive"
else
  fail_required "vpn-admin restore applied the backup archive"
fi

section "23. verify restored user/key state works"
POST_RESTORE_USERLIST="$(ssh_run 'sudo vpn-admin user list' 2>/dev/null || true)"
if [ -n "$PRE_BACKUP_USERLIST" ] && [ "$PRE_BACKUP_USERLIST" = "$POST_RESTORE_USERLIST" ]; then
  pass "restored user list matches the pre-backup snapshot exactly (ids/names/enabled state, including $TEST_USER_NAME)"
else
  fail_required "restored user list matches the pre-backup snapshot" "(pre-backup: $PRE_BACKUP_USERLIST | post-restore: $POST_RESTORE_USERLIST)"
fi
if ssh_run "sudo vpn-admin user list | grep -q $TEST_USER_NAME" 2>/dev/null; then
  pass "the persisted test user ($TEST_USER_NAME) survived uninstall/reinstall/restore"
else
  fail_required "the persisted test user ($TEST_USER_NAME) survived uninstall/reinstall/restore"
fi

section "24. doctor/protocol checks again (post-restore)"
if POST_RESTORE_PROTOCOL_OUT="$(ssh_run 'sudo vpn-admin doctor --protocol --require-protocol' 2>&1)"; then
  post_restore_rc=0
else
  post_restore_rc=$?
fi
if [ "$post_restore_rc" -eq 0 ] && printf '%s' "$POST_RESTORE_PROTOCOL_OUT" | grep -q 'completed a full handshake'; then
  pass "REALITY handshake self-test PASSES against the restored key material"
else
  fail_required "REALITY handshake self-test against restored key material" "(exit=$post_restore_rc; see remote output above)"
fi

section "25. final uninstall (offline singbox-vpn-uninstall)"
ssh_run "sudo rm -f $BACKUP_PATH" >/dev/null 2>&1 || true
if ssh_run 'sudo /opt/singbox-vpn/bin/singbox-vpn-uninstall --yes'; then
  pass "final singbox-vpn-uninstall --yes (offline, local binary only)"
else
  fail_required "final singbox-vpn-uninstall --yes (offline, local binary only)"
fi

section "26. SSH after final uninstall (new connection, port $SSH_PORT)"
if ssh_reconnect 'systemctl is-active --quiet sshd' 2>/dev/null; then pass "SSH still active post-uninstall"; else fail_required "SSH still active post-uninstall"; fi

section "27. final uninstall residue audit (vs. host baseline from stage 1b) — no singbox-vpn-owned services/binaries/state/firewall rules/sysctl or other residue"
RESIDUE="$(ssh_run '
  echo "opt_singbox-vpn=$([ -e /opt/singbox-vpn ] && echo 1 || echo 0)"
  echo "etc_vpn=$([ -e /etc/vpn ] && echo 1 || echo 0)"
  echo "var_lib_singbox-vpn=$([ -e /var/lib/singbox-vpn ] && echo 1 || echo 0)"
  echo "user_singbox=$(id sing-box >/dev/null 2>&1 && echo 1 || echo 0)"
  echo "user_vpnsub=$(id vpn-subscription >/dev/null 2>&1 && echo 1 || echo 0)"
  echo "unit_singbox=$([ -e /etc/systemd/system/sing-box.service ] && echo 1 || echo 0)"
  echo "unit_vpnsub=$([ -e /etc/systemd/system/vpn-subscription.service ] && echo 1 || echo 0)"
  echo "nginx_conf=$([ -e /etc/nginx/conf.d/vpn-subscription.conf ] && echo 1 || echo 0)"
  echo "certbot_hook=$([ -e /etc/letsencrypt/renewal-hooks/deploy/singbox-vpn-hysteria.sh ] && echo 1 || echo 0)"
  echo "listeners=$(ss -ltnp 2>/dev/null | grep -Ec "sing-box|vpn-subscription")"
  echo "locks=$(ls /run/lock/singbox-vpn* 2>/dev/null | wc -l)"
' 2>/dev/null || true)"
if [ "$RESIDUE" = "$BASELINE" ]; then
  pass "no singbox-vpn residue vs. pre-install host baseline"
else
  fail_required "no singbox-vpn residue vs. pre-install host baseline" "(baseline: $BASELINE | after uninstall: $RESIDUE)"
fi

section "manual-only / out-of-scope gates (cannot be automated here — UNVERIFIED, not PASS)"
mark_unverified "public/internet reachability from outside the target's network" "(no independent external controller in this harness — see checkpoint scope)"
mark_unverified "real Hiddify iOS/Android/MagicOS import + connect + sustained traffic" "(client/device properties — out of scope for this host-lifecycle gate)"
if [ -z "$UPDATE_TO_VERSION" ]; then
  mark_unverified "real GitHub release A->B update transition" "(no --update-to-version given)"
fi
mark_unverified "reboot-triggered client reconnect from a real device (Hiddify/other) after a server-side reboot" "(this harness proves the SERVER recovers on reboot — see stage 5 — and that the protocol self-test passes afterward; whether a REAL CLIENT DEVICE reconnects on its own after a server reboot needs a second physical device and is a manual release-candidate requirement, not automatable here — see docs/DEVICE_ACCEPTANCE_TESTS.md)"

section "summary"
if [ -z "$VERSION" ]; then
  echo "ACCEPTANCE CLASSIFICATION: DEVELOPMENT LIFECYCLE ONLY — NOT PRODUCTION ACCEPTANCE"
fi
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
