#!/usr/bin/env bash
# Canonical DESTRUCTIVE lifecycle acceptance gate — disposable
# AlmaLinux 9 x86_64 only. This is the release gate for
# install/uninstall/update rewrites; deploy/lib/fast-gate.sh is the
# cheap per-change gate and does NOT exercise any of this.
#
#   ssh root@DISPOSABLE-HOST -- true
#   ./deploy/almalinux/lifecycle-acceptance.sh \
#       --host root@DISPOSABLE-HOST --i-understand-this-is-destructive
#
# The gate deliberately exercises install, repair, interrupted-install
# cleanup, reinstall, protocol proofs, service recovery, user lifecycle,
# backup/restore, update/rollback when requested, certificate renewal,
# uninstall and final residue checks on one disposable remote host.
#
# Important test-isolation rule: a successful first install may issue one
# real Let's Encrypt certificate. Before the destructive reinstall scenarios,
# this harness snapshots that exact lineage outside singbox-vpn-owned state,
# lets uninstall prove that it removes its owned lineage, then restores the
# same valid lineage for later fresh installs. This prevents a lifecycle test
# from consuming several production ACME issuances for one hostname and
# hitting Let's Encrypt's exact-identifier rate limit.
#
# SAFETY (non-negotiable):
#   1. Requires --host and --i-understand-this-is-destructive.
#   2. Refuses localhost/127.0.0.1/0.0.0.0/::1.
#   3. Refuses this machine's configured production host and
#      SINGBOX_VPN_PRODUCTION_HOST.
#   4. Runs destructive behavior only over SSH against the explicit target.
#
# What this does NOT cover (reported UNVERIFIED): public reachability from a
# second network, a real Hiddify device, DNS/IPv6 leak behavior, or censorship
# resistance on a restrictive network.
set -Eeuo pipefail

HOST=""
ACK=0
ALLOW_DESTROY_EXISTING=0
UPDATE_TO_REF=""
UPDATE_TO_VERSION=""
VERSION=""
SKIP_REBOOT=0
SSH_PORT=22
DOMAIN=""
CERT_SNAPSHOT_REMOTE="/root/.singbox-vpn-lifecycle-cert.tar.gz"
CERT_REUSE_READY=0
CERT_HOST=""
CERT_CREATED_BY_GATE=0
INITIAL_BASELINE_READY=0
WORKING_BASELINE_READY=0
REINSTALL_READY=0
BACKUP_READY=0
STAGE7_CLEANED=0

usage() {
  cat <<'USAGE'
deploy/almalinux/lifecycle-acceptance.sh --host user@disposable-host --i-understand-this-is-destructive [options]

Required:
  --host USER@HOST                     disposable AlmaLinux 9 x86_64 target, reached via SSH.
  --i-understand-this-is-destructive   explicit acknowledgement; this WIPES singbox-vpn state.

  --allow-destroy-existing-singbox-vpn-install
                         SECOND, separate acknowledgement — required in addition to
                         --i-understand-this-is-destructive whenever the target already has
                         a singbox-vpn installation on it (detected via /etc/vpn/deployment.toml,
                         /var/lib/singbox-vpn/install-state.json, /var/lib/singbox-vpn/ownership.env,
                         or /opt/singbox-vpn). --i-understand-this-is-destructive only means "I know
                         this harness is destructive"; this flag means "I knowingly authorize
                         destroying an EXISTING singbox-vpn installation on this specific target."
                         Without it, the harness refuses to touch a target that already looks
                         provisioned, even if its hostname doesn't match any known production host.

Options:
  --ssh-port PORT        SSH port for the controller and installer. Default: 22.
  --update-to-ref REF    exercise the development updater against REF.
  --update-to-version VERSION
                         exercise the checksum-verified production updater.
  --version VERSION      exercise the immutable stable-release install path.
                         Without it, this is development-branch lifecycle testing only.
  --skip-reboot          skip the reboot+health stage.
  --domain HOST          use this hostname for every install/reinstall. The first
                         successful install may issue one real Let's Encrypt certificate;
                         the harness snapshots and reuses that exact lineage for later
                         destructive reinstall stages instead of requesting a new
                         production certificate each time.
  -h, --help             this help.
USAGE
}

while [ $# -gt 0 ]; do
  case "$1" in
    --host) HOST="$2"; shift 2 ;;
    --host=*) HOST="${1#*=}"; shift ;;
    --i-understand-this-is-destructive) ACK=1; shift ;;
    --allow-destroy-existing-singbox-vpn-install) ALLOW_DESTROY_EXISTING=1; shift ;;
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
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/preflight.sh"

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
# --update-to-ref crosses into a remote-executed URL/command string below
# (section 16, codeload.github.com/.../refs/heads/$UPDATE_TO_REF) — validate
# it as a real git ref/branch name before it ever reaches that string.
# Reject anything containing shell metacharacters, path traversal, or a
# leading '-' (which could be misread as a flag by a downstream command).
if [ -n "$UPDATE_TO_REF" ]; then
  case "$UPDATE_TO_REF" in
    -*) die "--update-to-ref must not start with '-', got '$UPDATE_TO_REF'." ;;
  esac
  if [[ ! "$UPDATE_TO_REF" =~ ^[A-Za-z0-9][A-Za-z0-9._/-]*$ ]] || [[ "$UPDATE_TO_REF" == *..* ]] || [[ "$UPDATE_TO_REF" == */ ]]; then
    die "--update-to-ref '$UPDATE_TO_REF' is not a syntactically valid git ref/branch name — refusing to use it."
  fi
fi
# --domain crosses into the same remote-executed command strings (the
# installer's own --domain argument, quoted below) — reuse the installer's
# own hostname grammar (preflight_validate_hostname) rather than a
# second, subtly different one.
if [ -n "$DOMAIN" ]; then
  preflight_validate_hostname "$DOMAIN" "--domain" || die "refusing to use the --domain value above for a destructive remote run."
fi

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
ssh_reconnect() {
  timeout 30 ssh -o ControlPath=none -p "$SSH_PORT" -o BatchMode=yes -o ConnectTimeout=15 \
    -o StrictHostKeyChecking=accept-new "$HOST" "$@"
}

# Built as an array, never as a raw interpolated string: every element
# here crosses an SSH shell boundary (embedded into a single command
# string executed by the remote shell in run_install() below). SSH_PORT
# is already numeric-only (validated above) and DOMAIN is already
# validated against the installer's own hostname grammar
# (preflight_validate_hostname, above) — but quoting each element with
# `printf '%q'` at the point of use (install_args_quoted(), below) is the
# actual safety boundary, not the validation alone: it guarantees that
# whatever ends up in these values can only ever be interpreted as a
# single literal argument to install.sh on the remote end, never as
# additional shell syntax (';', '$(...)', backticks, etc.).
INSTALL_ARGS=(--non-interactive)
if [ "$SSH_PORT" != "22" ]; then
  INSTALL_ARGS+=(--ssh-port "$SSH_PORT")
fi
if [ -n "$DOMAIN" ]; then
  INSTALL_ARGS+=(--domain "$DOMAIN")
fi
install_args_quoted() {
  local out="" a
  for a in "${INSTALL_ARGS[@]}"; do
    out="$out $(printf '%q' "$a")"
  done
  printf '%s' "$out"
}
# The lifecycle controller itself fetches the bootstrap over the target VPS's
# network before install.sh's own retry policy can take effect. A transient
# ECONNREFUSED here was observed in a real gate run, so make this outermost
# fetch independently resilient. `set -o pipefail` in run_install* below is
# equally important: without it, curl can fail while an empty downstream bash
# exits 0, producing a false install PASS.
REMOTE_BOOTSTRAP_CURL_FLAGS="--connect-timeout 10 --max-time 300 --retry 5 --retry-delay 2 --retry-connrefused"

failures=0
required_fail=0
unverified=0
blocked=0
pass() { printf "[PASS] %-55s\n" "$1"; }
fail() { printf "[FAIL] %-55s %s\n" "$1" "${2:-}"; failures=$((failures + 1)); }
fail_required() { printf "[FAIL][required] %-45s %s\n" "$1" "${2:-}"; failures=$((failures + 1)); required_fail=1; }
block() { printf "[BLOCKED] %-51s %s\n" "$1" "${2:-}"; blocked=$((blocked + 1)); }
mark_unverified() { printf "[UNVERIFIED] %-49s %s\n" "$1" "${2:-}"; unverified=$((unverified + 1)); }
section() { echo; echo "=== $1 ==="; }

# Snapshot the real certificate lineage after the first successful install.
# The snapshot lives under /root, outside every path the singbox-vpn
# uninstaller owns. The remote command exits non-zero if the deployment has
# no valid hostname/lineage, so a real run never silently claims protection
# against ACME rate limiting when no snapshot exists. Mock fixture SSH may
# return success with no metadata; that is harmless because metadata is only
# needed for final ownership-aware cleanup.
capture_cert_for_reuse() {
  [ "$CERT_REUSE_READY" -eq 0 ] || return 0
  local meta=""
  if meta="$(ssh_run '
    host="$(sed -nE "s/^public_host[[:space:]]*=[[:space:]]*\"([^\"]+)\".*/\1/p" /etc/vpn/deployment.toml 2>/dev/null | head -1)"
    case "$host" in ""|*[!A-Za-z0-9.-]*) exit 2 ;; esac
    [ -s "/etc/letsencrypt/live/$host/fullchain.pem" ] || exit 3
    [ -s "/etc/letsencrypt/live/$host/privkey.pem" ] || exit 3
    paths="etc/letsencrypt/live/$host"
    [ -e "/etc/letsencrypt/archive/$host" ] && paths="$paths etc/letsencrypt/archive/$host"
    [ -e "/etc/letsencrypt/renewal/$host.conf" ] && paths="$paths etc/letsencrypt/renewal/$host.conf"
    sudo tar -C / -czf /root/.singbox-vpn-lifecycle-cert.tar.gz $paths || exit 4
    owned=0
    owned_list="$(awk -F= '\''$1=="CERT_LINEAGES_CREATED_BY_SINGBOX_VPN" {v=$2; gsub(/^\"|\"$/, "", v); print v; exit}'\'' /var/lib/singbox-vpn/ownership.env 2>/dev/null)"
    case " $owned_list " in *" $host "*) owned=1 ;; esac
    printf "%s|%s" "$host" "$owned"
  ' 2>/dev/null)"; then
    CERT_REUSE_READY=1
    if [[ "$meta" =~ ^([A-Za-z0-9.-]+)\|([01])$ ]]; then
      CERT_HOST="${BASH_REMATCH[1]}"
      CERT_CREATED_BY_GATE="${BASH_REMATCH[2]}"
    fi
    pass "certificate lineage snapshotted for destructive reinstall reuse"
    return 0
  fi
  fail_required "certificate lineage snapshot for lifecycle reuse" "(refusing to spend repeated production ACME issuances in later fresh-install stages)"
  return 1
}

restore_cert_for_reuse() {
  [ "$CERT_REUSE_READY" -eq 1 ] || return 1
  if ssh_run "sudo test -s $CERT_SNAPSHOT_REMOTE && sudo tar -C / -xzf $CERT_SNAPSHOT_REMOTE" 2>/dev/null; then
    pass "reused the original certificate lineage (no new ACME issuance)"
    return 0
  fi
  fail_required "restore snapshotted certificate lineage" "(later fresh install would otherwise request another production certificate)"
  return 1
}

cleanup_cert_snapshot() {
  if [ "$CERT_CREATED_BY_GATE" -eq 1 ] && [[ "$CERT_HOST" =~ ^[A-Za-z0-9.-]+$ ]]; then
    ssh_run "sudo rm -rf '/etc/letsencrypt/live/$CERT_HOST' '/etc/letsencrypt/archive/$CERT_HOST'; sudo rm -f '/etc/letsencrypt/renewal/$CERT_HOST.conf'" >/dev/null 2>&1 || true
  fi
  ssh_run "sudo rm -f $CERT_SNAPSHOT_REMOTE" >/dev/null 2>&1 || true
}

run_install() {
  ssh_run "set -o pipefail; curl -fsSL $REMOTE_BOOTSTRAP_CURL_FLAGS https://raw.githubusercontent.com/David610/singbox-vpn/$BOOTSTRAP_REF/install.sh | $INSTALL_SOURCE_ENV REALITY_HANDSHAKE_SERVER=www.google.com SINGBOX_VPN_ALLOW_IP_HOSTNAME=1 bash -s -- $(install_args_quoted)"
}

run_install_abort_after_singbox() {
  ssh_run "set -o pipefail; curl -fsSL $REMOTE_BOOTSTRAP_CURL_FLAGS https://raw.githubusercontent.com/David610/singbox-vpn/$BOOTSTRAP_REF/install.sh | sudo SINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER=install_singbox $INSTALL_SOURCE_ENV REALITY_HANDSHAKE_SERVER=www.google.com SINGBOX_VPN_ALLOW_IP_HOSTNAME=1 bash -s -- $(install_args_quoted)"
}

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
  fail_required "AlmaLinux 9 confirmed" "(not AlmaLinux 9 — off the supported OS matrix)"
fi
if ssh_run '[ "$(uname -m)" = x86_64 ]' 2>/dev/null; then
  pass "x86_64 arch confirmed"
else
  fail_required "x86_64 arch confirmed" "(not x86_64 — off the supported arch matrix)"
fi

section "0a. existing singbox-vpn installation guard"
# Hostname-only production protection (above) is not enough: the SAME
# production machine can also be reached via a different hostname/alias
# or its bare IP, none of which would ever match /etc/vpn/deployment.toml
# on THIS controller or SINGBOX_VPN_PRODUCTION_HOST. Positively inspect
# the TARGET itself for signs of an existing singbox-vpn installation
# before any destructive stage runs. These four paths are fixed literals
# (never remotely-sourced values), so there is no injection surface here
# — this is a plain existence check, not a place where malformed remote
# data could ever influence what gets destroyed.
existing_install_markers=""
for marker in /etc/vpn/deployment.toml /var/lib/singbox-vpn/install-state.json /var/lib/singbox-vpn/ownership.env /opt/singbox-vpn; do
  if ssh_run "[ -e '$marker' ]" 2>/dev/null; then
    existing_install_markers="$existing_install_markers $marker"
  fi
done
if [ -n "$existing_install_markers" ]; then
  if [ "$ALLOW_DESTROY_EXISTING" -eq 1 ]; then
    pass "existing singbox-vpn installation detected on target ($existing_install_markers) — destruction explicitly authorized via --allow-destroy-existing-singbox-vpn-install"
  else
    fail_required "existing singbox-vpn installation guard" "target already has singbox-vpn state:$existing_install_markers — refusing to destroy it. If this is genuinely disposable test state, re-run with --allow-destroy-existing-singbox-vpn-install ALSO given (in addition to --i-understand-this-is-destructive)."
    die "refusing to proceed against '$HOST': it already has an existing singbox-vpn installation and --allow-destroy-existing-singbox-vpn-install was not given. Nothing on this host has been touched."
  fi
else
  pass "no existing singbox-vpn installation detected on target — safe to provision fresh"
fi

section "0b. bootstrap prerequisites (bash, curl, tar)"
# The controller's own bootstrap fetch (run_install(), below) is a
# `curl ... | bash` pipeline against the TARGET host, not this
# controller. If the target is missing curl (observed on a real host
# once: "bash: line 1: curl: command not found"), that pipeline fails —
# `set -o pipefail` (above) already turns that into a real non-zero exit
# rather than a false PASS, but the resulting error is just a generic
# pipeline failure buried in remote stderr. Check explicitly here instead,
# so a missing prerequisite is reported as exactly that in one line, the
# clean-install stage is never attempted against a host that cannot
# possibly run it, and everything depending on a working install reports
# BLOCKED (via INITIAL_BASELINE_READY staying 0) rather than a pile of
# unrelated-looking failures.
BOOTSTRAP_READY=0
missing_tools=""
for tool in bash curl tar; do
  ssh_run "command -v $tool" >/dev/null 2>&1 || missing_tools="$missing_tools $tool"
done
if [ -z "$missing_tools" ]; then
  pass "bootstrap prerequisites present (bash, curl, tar)"
  BOOTSTRAP_READY=1
else
  fail_required "bootstrap prerequisites" "missing:$missing_tools"
fi

section "1. SSH baseline (before any install)"
ssh_baseline="$(ssh_run "systemctl is-active sshd 2>/dev/null; ss -ltnp 2>/dev/null | grep -c :$SSH_PORT || true" 2>/dev/null || true)"
if [ -n "$ssh_baseline" ]; then pass "SSH baseline captured (port $SSH_PORT)"; else fail "SSH baseline captured"; fi

section "1b. host baseline (sanitized, no secrets)"
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
if [ "$BOOTSTRAP_READY" -ne 1 ]; then
  block "install.sh (clean)" "(bootstrap prerequisites missing on target — see stage 0b; the curl|bash pipeline cannot possibly run)"
elif run_install; then
  pass "install.sh (clean)"
  INITIAL_BASELINE_READY=1
else
  fail_required "install.sh (clean)"
fi

section "3. SSH after install (new connection, port $SSH_PORT)"
if ssh_reconnect 'systemctl is-active --quiet sshd' 2>/dev/null; then pass "SSH still active post-install"; else fail_required "SSH still active post-install"; fi

if [ "$INITIAL_BASELINE_READY" -eq 1 ]; then
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
else
  section "4-5. acceptance/reboot checks"
  block "acceptance and reboot checks" "(clean install never established a working baseline)"
fi

section "6. repair / idempotent re-run"
if [ "$BOOTSTRAP_READY" -ne 1 ]; then
  block "install.sh (idempotent re-run) + SSH reconnect" "(bootstrap prerequisites missing on target — see stage 0b)"
elif run_install && ssh_reconnect 'systemctl is-active --quiet sshd' 2>/dev/null; then
  pass "install.sh (idempotent re-run) + SSH reconnect"
  INITIAL_BASELINE_READY=1
else
  fail_required "install.sh (idempotent re-run) + SSH reconnect"
fi

if [ "$INITIAL_BASELINE_READY" -eq 1 ]; then
  capture_cert_for_reuse || true
fi

section "7. failed/interrupted install cleanup (scratch scenario; ends with singbox-vpn fully removed)"
if [ "$INITIAL_BASELINE_READY" -ne 1 ]; then
  block "interrupted-install cleanup scenario" "(no working installation to exercise as a repair)"
elif [ "$CERT_REUSE_READY" -ne 1 ]; then
  block "interrupted-install cleanup scenario" "(certificate snapshot unavailable; refusing to consume another production ACME issuance later)"
else
  if run_install_abort_after_singbox; then
    fail_required "interrupted install actually aborted" "(expected non-zero exit, got success)"
  else
    pass "interrupted install aborted as expected"
  fi
  if ssh_run '[ -e /opt/singbox-vpn ] || [ -e /etc/vpn ]' 2>/dev/null; then
    pass "abort hook fired mid-install (partial state present, not a pre-flight failure)"
  else
    fail_required "abort hook fired mid-install" "(no partial state found — abort may not have reached installer)"
  fi
  if ssh_run 'sudo /opt/singbox-vpn/bin/singbox-vpn-uninstall --yes' 2>/dev/null \
    && ssh_run '[ ! -e /etc/vpn ] && [ ! -e /opt/singbox-vpn ] && [ ! -e /var/lib/singbox-vpn ] \
        && ! systemctl list-unit-files 2>/dev/null | grep -q "^sing-box\.service\|^vpn-subscription\.service" \
        && ! id sing-box >/dev/null 2>&1 && ! id vpn-subscription >/dev/null 2>&1' 2>/dev/null; then
    pass "cleanup after interrupted install (offline singbox-vpn-uninstall)"
    STAGE7_CLEANED=1
  else
    fail "cleanup after interrupted install"
  fi
fi

section "8. reinstall after interrupted-install cleanup (back to a working baseline for the rest of this run)"
if [ "$STAGE7_CLEANED" -eq 1 ]; then
  if ! restore_cert_for_reuse; then
    block "post-cleanup reinstall" "(certificate reuse failed; refusing a fresh production ACME request)"
  elif run_install && ssh_reconnect 'systemctl is-active --quiet sshd' 2>/dev/null; then
    pass "install.sh (clean, post-cleanup) + SSH reconnect"
    WORKING_BASELINE_READY=1
  else
    fail_required "install.sh (clean, post-cleanup) + SSH reconnect"
  fi
elif [ "$BOOTSTRAP_READY" -ne 1 ]; then
  block "install.sh (baseline repair/retry) + SSH reconnect" "(bootstrap prerequisites missing on target — see stage 0b)"
else
  # No destructive cleanup happened. A normal re-run is still useful: it can
  # recover from an earlier transient install failure and establish a valid
  # baseline without spending another certificate if the current install is
  # intact.
  if run_install && ssh_reconnect 'systemctl is-active --quiet sshd' 2>/dev/null; then
    pass "install.sh (baseline repair/retry) + SSH reconnect"
    WORKING_BASELINE_READY=1
    capture_cert_for_reuse || true
  else
    fail_required "install.sh (baseline repair/retry) + SSH reconnect"
  fi
fi

BACKUP_PATH="/root/singbox-vpn-lifecycle-backup.tar"
PRE_BACKUP_USERLIST=""
TEST_USER_NAME="lifecycle-test-user"

if [ "$WORKING_BASELINE_READY" -eq 1 ]; then
  section "9. create test user (persists through the backup/restore verification below)"
  if ssh_run "sudo vpn-admin user create --name $TEST_USER_NAME --json" >/dev/null 2>&1 \
    && ssh_run "sudo vpn-admin user list | grep -q $TEST_USER_NAME" 2>/dev/null; then
    pass "test user created ($TEST_USER_NAME)"
  else
    fail_required "test user created ($TEST_USER_NAME)"
  fi

  section "10. vpn-admin doctor (standard checks, no protocol self-test)"
  if ssh_run 'sudo vpn-admin doctor' 2>/dev/null; then pass "vpn-admin doctor"; else fail_required "vpn-admin doctor"; fi

  section "11. REALITY authentication proof (vpn-admin doctor --protocol --require-protocol)"
  if PROTOCOL_OUT="$(ssh_run 'sudo vpn-admin doctor --protocol --require-protocol' 2>&1)"; then protocol_rc=0; else protocol_rc=$?; fi
  if [ "$protocol_rc" -eq 0 ] && printf '%s' "$PROTOCOL_OUT" | grep -q 'completed a full handshake'; then
    pass "REALITY handshake self-test PASSED (real sing-box client, live public_key/short_id, application bytes end-to-end)"
  else
    fail_required "REALITY handshake self-test (doctor --protocol --require-protocol)" "(exit=$protocol_rc; see remote output above)"
  fi

  section "12. Hysteria2 real handshake+transfer proof (deploy/lib/vpn-benchmark.sh)"
  if HY_OUT="$(ssh_run "sudo /opt/singbox-vpn/deploy/lib/vpn-benchmark.sh --runs 1 --download-url 'https://speed.cloudflare.com/__down?bytes=2000000'" 2>&1)"; then hy_rc=0; else hy_rc=$?; fi
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
  if [ "$sb_recovered" -eq 1 ]; then pass "systemd restarted sing-box within a bounded time (<=30s; unit is Restart=on-failure, RestartSec=2)"; else fail_required "systemd restarted sing-box within a bounded time"; fi
  sb_pid_after="$(ssh_run 'systemctl show -p MainPID --value sing-box' 2>/dev/null || true)"
  if [ -n "$sb_pid_after" ] && [ "$sb_pid_after" != "0" ] && [ "$sb_pid_after" != "$sb_pid_before" ]; then
    pass "sing-box MainPID changed after kill+restart (a real respawn, not a stale unit)"
  else
    fail_required "sing-box MainPID changed after kill+restart" "(before=$sb_pid_before after=$sb_pid_after)"
  fi

  section "13b. exhaust the restart budget (StartLimitBurst) and prove vpn-service-watchdog recovers it"
  # Suspend the timer while deliberately creating FAILED. Otherwise its
  # legitimate periodic recovery can race this observation and turn a real
  # FAILED state back to active before the harness checks it. Reboot stage 5
  # already proves the timer is armed; here we test watchdog logic directly.
  watchdog_timer_suspended=0
  if ssh_run 'sudo systemctl stop vpn-service-watchdog.timer && systemctl is-inactive --quiet vpn-service-watchdog.timer' 2>/dev/null; then
    pass "vpn-service-watchdog.timer suspended for deterministic crash-loop test"
    watchdog_timer_suspended=1
  else
    fail_required "suspend vpn-service-watchdog.timer for crash-loop test"
  fi

  failed_state_ready=0
  if [ "$watchdog_timer_suspended" -eq 1 ]; then
    if ssh_run '
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
      failed_state_ready=1
    else
      fail_required "sing-box.service did not reach FAILED state after a fast repeated-crash burst" "(restart budget was not exhausted deterministically)"
    fi
  else
    block "crash-loop FAILED-state creation" "(watchdog timer could not be suspended, so the observation would be racy)"
  fi

  if [ "$failed_state_ready" -eq 1 ]; then
    if DOCTOR_DURING_FAILURE_OUT="$(ssh_run 'sudo vpn-admin doctor' 2>&1)"; then doctor_during_failure_rc=0; else doctor_during_failure_rc=$?; fi
    if [ "$doctor_during_failure_rc" -ne 0 ] && printf '%s' "$DOCTOR_DURING_FAILURE_OUT" | grep -qi 'sing-box.service is in a FAILED state'; then
      pass "vpn-admin doctor correctly reports sing-box.service as FAILED (distinct from merely 'not active')"
    else
      fail_required "vpn-admin doctor did not report the FAILED sing-box.service" "(exit=$doctor_during_failure_rc; see remote output above)"
    fi
    STATUS_DURING_FAILURE_OUT="$(ssh_run 'sudo vpn-admin status' 2>&1 || true)"
    if printf '%s' "$STATUS_DURING_FAILURE_OUT" | grep -qi 'sing-box.*failed'; then pass "vpn-admin status correctly reports sing-box as failed"; else fail_required "vpn-admin status did not report sing-box as failed"; fi

    if ssh_run 'sudo systemctl start vpn-service-watchdog.service' 2>/dev/null; then pass "vpn-service-watchdog.service ran without error"; else fail_required "vpn-service-watchdog.service ran without error"; fi
    watchdog_recovered=0
    for _ in $(seq 1 15); do
      if ssh_run 'systemctl is-active --quiet sing-box' 2>/dev/null; then watchdog_recovered=1; break; fi
      sleep 2
    done
    if [ "$watchdog_recovered" -eq 1 ]; then pass "vpn-service-watchdog recovered sing-box.service from its parked FAILED state (a recoverable service is never left permanently down)"; else fail_required "vpn-service-watchdog did not recover sing-box.service from its FAILED state"; fi
  else
    block "FAILED-state doctor/status/watchdog recovery assertions" "(the prerequisite FAILED state was not established; dependent failures are not counted separately)"
  fi

  if [ "$watchdog_timer_suspended" -eq 1 ]; then
    if ssh_run 'sudo systemctl start vpn-service-watchdog.timer && systemctl is-active --quiet vpn-service-watchdog.timer' 2>/dev/null; then
      pass "vpn-service-watchdog.timer re-armed after deterministic crash-loop test"
    else
      fail_required "re-arm vpn-service-watchdog.timer after crash-loop test"
    fi
  fi

  section "13c. systemctl stop still behaves normally (a deliberate stop is never treated as a failure to auto-recover)"
  if ssh_run 'sudo systemctl stop sing-box' 2>/dev/null; then pass "systemctl stop sing-box succeeded"; else fail_required "systemctl stop sing-box succeeded"; fi
  sleep 3
  if ssh_run 'systemctl is-active --quiet sing-box' 2>/dev/null; then fail_required "sing-box remained active after systemctl stop" "(a deliberate stop must actually stop it)"; else pass "sing-box is inactive after systemctl stop (not silently auto-restarted)"; fi
  if ssh_run 'systemctl is-failed --quiet sing-box' 2>/dev/null; then fail_required "sing-box is reported FAILED after a deliberate stop"; else pass "sing-box is 'inactive', not 'failed', after a deliberate stop"; fi
  ssh_run 'sudo systemctl start vpn-service-watchdog.service' >/dev/null 2>&1 || true
  sleep 2
  if ssh_run 'systemctl is-active --quiet sing-box' 2>/dev/null; then fail_required "vpn-service-watchdog restarted a deliberately-stopped sing-box"; else pass "vpn-service-watchdog left the deliberately-stopped sing-box alone"; fi
  if ssh_run 'sudo systemctl start sing-box' 2>/dev/null; then pass "sing-box restarted normally after the deliberate-stop test (restoring state for the rest of this run)"; else fail_required "sing-box restarted normally after the deliberate-stop test"; fi

  section "14. protocol works after recovery (re-run doctor --protocol --require-protocol)"
  if POST_RECOVERY_PROTOCOL_OUT="$(ssh_run 'sudo vpn-admin doctor --protocol --require-protocol' 2>&1)"; then post_recovery_rc=0; else post_recovery_rc=$?; fi
  if [ "$post_recovery_rc" -eq 0 ] && printf '%s' "$POST_RECOVERY_PROTOCOL_OUT" | grep -q 'completed a full handshake'; then pass "REALITY handshake self-test still PASSES after the SIGKILL+recovery cycle"; else fail_required "REALITY handshake self-test after recovery" "(exit=$post_recovery_rc; see remote output above)"; fi

  section "15. user rotate/disable/remove sanity (scratch user; does not touch the persisted test user above)"
  # Report only the failing step name. Never echo rotate-token output because
  # that contains a credential.
  if SCRATCH_RESULT="$(ssh_run '
    scratch_id=""
    cleanup_scratch() { [ -z "$scratch_id" ] || sudo vpn-admin user remove "$scratch_id" >/dev/null 2>&1 || true; }
    trap cleanup_scratch EXIT
    scratch_id="$(sudo vpn-admin user create --name lifecycle-scratch-user --json | grep -o "\"id\": *\"[^\"]*\"" | head -1 | sed -E "s/.*\"([^\"]+)\"$/\1/")" || { echo create; exit 1; }
    [ -n "$scratch_id" ] || { echo parse-id; exit 1; }
    sudo vpn-admin user list | grep -q "$scratch_id" || { echo list; exit 1; }
    sudo vpn-admin user rotate-token "$scratch_id" >/dev/null || { echo rotate-token; exit 1; }
    sudo vpn-admin user rotate-vless "$scratch_id" >/dev/null || { echo rotate-vless; exit 1; }
    sudo vpn-admin user rotate-hysteria "$scratch_id" >/dev/null || { echo rotate-hysteria; exit 1; }
    sudo vpn-admin user disable "$scratch_id" >/dev/null || { echo disable; exit 1; }
    sudo vpn-admin user remove "$scratch_id" >/dev/null || { echo remove; exit 1; }
    scratch_id=""
    trap - EXIT
  ' 2>/dev/null)"; then
    pass "scratch user create/rotate/disable/remove"
  else
    fail_required "scratch user create/rotate/disable/remove" "(failed sub-step: ${SCRATCH_RESULT:-unknown}; secret-bearing command output intentionally suppressed)"
  fi

  if [ -n "$UPDATE_TO_VERSION" ]; then
    section "16. checksum-verified production update -> $UPDATE_TO_VERSION"
    version_before="$(ssh_run 'sudo cat /var/lib/singbox-vpn/install-state.json 2>/dev/null' 2>/dev/null || true)"
    if ssh_run "sudo /opt/singbox-vpn/deploy/almalinux/update.sh --version $(printf '%q' "$UPDATE_TO_VERSION")" && ssh_reconnect 'true' 2>/dev/null; then
      version_after="$(ssh_run 'sudo cat /var/lib/singbox-vpn/install-state.json 2>/dev/null' 2>/dev/null || true)"
      if [ -n "$version_after" ] && [ "$version_before" != "$version_after" ]; then pass "production update -> $UPDATE_TO_VERSION (install state changed)"; else fail_required "production update -> $UPDATE_TO_VERSION" "(command succeeded but install state did not change)"; fi
    else
      fail_required "production update -> $UPDATE_TO_VERSION"
    fi

    section "16b. injected failed production repair -> rollback proof"
    pre_rollback_version="$(ssh_run 'sudo cat /var/lib/singbox-vpn/install-state.json 2>/dev/null' 2>/dev/null || true)"
    if ssh_run "sudo SINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER=after_switch /opt/singbox-vpn/deploy/almalinux/update.sh --repair" 2>/dev/null; then fail_required "failed production repair aborted as expected" "(expected non-zero exit, got success)"; else pass "failed production repair aborted as expected"; fi
    if ssh_reconnect 'systemctl is-active --quiet sshd && systemctl is-active --quiet sing-box && systemctl is-active --quiet vpn-subscription && sudo vpn-admin doctor --protocol' 2>/dev/null; then
      post_rollback_version="$(ssh_run 'sudo cat /var/lib/singbox-vpn/install-state.json 2>/dev/null' 2>/dev/null || true)"
      if [ "$pre_rollback_version" = "$post_rollback_version" ]; then pass "production repair rollback restored the prior working state"; else fail_required "production repair rollback restored prior state" "(install-state.json differs)"; fi
    else
      fail_required "production repair rollback left services/protocol/SSH healthy"
    fi
  elif [ -n "$UPDATE_TO_REF" ]; then
    section "16. safe update path -> $UPDATE_TO_REF (--dev-rebuild; transactional updater machinery only)"
    version_before="$(ssh_run 'sudo /opt/singbox-vpn/bin/vpn-admin --version 2>/dev/null || sudo cat /var/lib/singbox-vpn/install-state.json 2>/dev/null' 2>/dev/null || true)"
    if ssh_run "curl -fsSL --connect-timeout 10 --max-time 120 --retry 5 --retry-delay 2 --retry-connrefused -o /tmp/singbox-vpn-update-ref.tar.gz https://codeload.github.com/David610/singbox-vpn/tar.gz/refs/heads/$(printf '%q' "$UPDATE_TO_REF") \
        && rm -rf /tmp/singbox-vpn-update-ref && mkdir -p /tmp/singbox-vpn-update-ref \
        && tar -xzf /tmp/singbox-vpn-update-ref.tar.gz -C /tmp/singbox-vpn-update-ref --strip-components=1 \
        && sudo rsync -a --delete --exclude target --exclude .git /tmp/singbox-vpn-update-ref/ /opt/singbox-vpn/ \
        && sudo /opt/singbox-vpn/deploy/almalinux/update.sh --dev-rebuild" && ssh_reconnect 'true' 2>/dev/null; then
      version_after="$(ssh_run 'sudo /opt/singbox-vpn/bin/vpn-admin --version 2>/dev/null || sudo cat /var/lib/singbox-vpn/install-state.json 2>/dev/null' 2>/dev/null || true)"
      if [ -n "$version_after" ] && [ "$version_before" != "$version_after" ]; then pass "update.sh --dev-rebuild -> $UPDATE_TO_REF (binary/state actually changed, not just exit 0)"; else fail_required "update.sh --dev-rebuild -> $UPDATE_TO_REF" "(command succeeded but version-state did not change)"; fi
    else
      fail "update.sh --dev-rebuild -> $UPDATE_TO_REF"
    fi

    section "16b. injected failed update -> rollback proof (failure injected after SWITCH begins)"
    pre_rollback_version="$(ssh_run 'sudo cat /var/lib/singbox-vpn/install-state.json 2>/dev/null' 2>/dev/null || true)"
    if ssh_run "sudo SINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER=after_switch /opt/singbox-vpn/deploy/almalinux/update.sh --dev-rebuild" 2>/dev/null; then fail_required "failed update aborted as expected" "(expected non-zero exit, got success)"; else pass "failed update aborted as expected"; fi
    if ssh_reconnect 'systemctl is-active --quiet sshd && systemctl is-active --quiet sing-box && systemctl is-active --quiet vpn-subscription && sudo vpn-admin doctor --protocol' 2>/dev/null; then
      post_rollback_version="$(ssh_run 'sudo cat /var/lib/singbox-vpn/install-state.json 2>/dev/null' 2>/dev/null || true)"
      if [ "$pre_rollback_version" = "$post_rollback_version" ]; then pass "rollback restored the previous working release (prior binary/schema/units/config/services/protocol/SSH)"; else fail_required "rollback restored prior state" "(install-state.json differs)"; fi
    else
      fail_required "rollback restored prior binary/schema/units/config/services/protocol/SSH"
    fi
  else
    section "16. safe update path (SKIPPED: no --update-to-ref given)"
    section "16b. injected failed update -> rollback proof (SKIPPED: no --update-to-ref given)"
  fi

  section "17. create vpn backup"
  PRE_BACKUP_USERLIST="$(ssh_run 'sudo vpn-admin user list' 2>/dev/null || true)"
  if ssh_run "sudo vpn-admin backup --output $BACKUP_PATH" 2>/dev/null && ssh_run "sudo test -s $BACKUP_PATH" 2>/dev/null; then
    pass "vpn-admin backup produced a non-empty archive at $BACKUP_PATH"
    BACKUP_READY=1
  else
    fail_required "vpn-admin backup produced a non-empty archive"
  fi

  section "18. certbot renew --dry-run (while the deployment is still live, before the destructive uninstall below)"
  if CERTBOT_DRY_OUT="$(ssh_run 'sudo certbot renew --dry-run' 2>&1)"; then certbot_dry_rc=0; else certbot_dry_rc=$?; fi
  if [ "$certbot_dry_rc" -eq 0 ] && ! printf '%s' "$CERTBOT_DRY_OUT" | grep -qF 'No simulated renewals were attempted.'; then
    pass "certbot renew --dry-run (at least one renewal was eligible for simulation)"
  elif printf '%s' "$CERTBOT_DRY_OUT" | grep -qF 'No simulated renewals were attempted.'; then
    fail_required "certbot renew --dry-run" "(certbot exited 0 but tested zero lineages — this is not renewal proof)"
  else
    fail_required "certbot renew --dry-run" "(exit=$certbot_dry_rc; renewal must work before a stable release)"
  fi
else
  section "9-18. runtime/protocol/user/update/backup/renewal checks"
  block "stages 9-18" "(stage 8 did not establish a working baseline; dependent failures are intentionally not counted as separate bugs)"
fi

section "19. uninstall completely (offline singbox-vpn-uninstall)"
ssh_run 'sudo iptables -I OUTPUT -d github.com -j REJECT 2>/dev/null; sudo iptables -I OUTPUT -d raw.githubusercontent.com -j REJECT 2>/dev/null' >/dev/null 2>&1 || true
if ssh_run 'test -x /opt/singbox-vpn/bin/singbox-vpn-uninstall' 2>/dev/null; then
  if ssh_run 'sudo /opt/singbox-vpn/bin/singbox-vpn-uninstall --yes'; then pass "singbox-vpn-uninstall --yes (offline, local binary only)"; else fail_required "singbox-vpn-uninstall --yes (offline, local binary only)"; fi
elif ssh_run '[ ! -e /etc/vpn ] && [ ! -e /opt/singbox-vpn ] && [ ! -e /var/lib/singbox-vpn ]' 2>/dev/null; then
  pass "target already clean (no installed uninstaller needed)"
else
  fail_required "offline uninstall available for partial state" "(partial singbox-vpn state exists but local uninstaller is missing)"
fi
ssh_run 'sudo iptables -D OUTPUT -d github.com -j REJECT 2>/dev/null; sudo iptables -D OUTPUT -d raw.githubusercontent.com -j REJECT 2>/dev/null' >/dev/null 2>&1 || true

section "20. SSH after uninstall (new connection, port $SSH_PORT)"
if ssh_reconnect 'systemctl is-active --quiet sshd' 2>/dev/null; then pass "SSH still active post-uninstall"; else fail_required "SSH still active post-uninstall"; fi

section "21. reinstall from the normal one-command production path"
if [ "$CERT_REUSE_READY" -ne 1 ]; then
  block "reinstall after uninstall" "(no reusable certificate snapshot; refusing another production ACME issuance)"
elif ! restore_cert_for_reuse; then
  block "reinstall after uninstall" "(certificate restore failed)"
elif run_install && ssh_reconnect 'systemctl is-active --quiet sshd' 2>/dev/null; then
  pass "reinstall after uninstall + SSH reconnect"
  REINSTALL_READY=1
else
  fail_required "reinstall after uninstall + SSH reconnect"
fi

if [ "$REINSTALL_READY" -eq 1 ] && [ "$BACKUP_READY" -eq 1 ]; then
  section "22. restore backup"
  if ssh_run "sudo test -s $BACKUP_PATH" 2>/dev/null && ssh_run "sudo vpn-admin restore $BACKUP_PATH" 2>/dev/null; then pass "vpn-admin restore applied the backup archive"; else fail_required "vpn-admin restore applied the backup archive"; fi

  section "23. verify restored user/key state works"
  POST_RESTORE_USERLIST="$(ssh_run 'sudo vpn-admin user list' 2>/dev/null || true)"
  if [ -n "$PRE_BACKUP_USERLIST" ] && [ "$PRE_BACKUP_USERLIST" = "$POST_RESTORE_USERLIST" ]; then pass "restored user list matches the pre-backup snapshot exactly (ids/names/enabled state, including $TEST_USER_NAME)"; else fail_required "restored user list matches the pre-backup snapshot" "(pre-backup: $PRE_BACKUP_USERLIST | post-restore: $POST_RESTORE_USERLIST)"; fi
  if ssh_run "sudo vpn-admin user list | grep -q $TEST_USER_NAME" 2>/dev/null; then pass "the persisted test user ($TEST_USER_NAME) survived uninstall/reinstall/restore"; else fail_required "the persisted test user ($TEST_USER_NAME) survived uninstall/reinstall/restore"; fi

  section "24. doctor/protocol checks again (post-restore)"
  if POST_RESTORE_PROTOCOL_OUT="$(ssh_run 'sudo vpn-admin doctor --protocol --require-protocol' 2>&1)"; then post_restore_rc=0; else post_restore_rc=$?; fi
  if [ "$post_restore_rc" -eq 0 ] && printf '%s' "$POST_RESTORE_PROTOCOL_OUT" | grep -q 'completed a full handshake'; then pass "REALITY handshake self-test PASSES against the restored key material"; else fail_required "REALITY handshake self-test against restored key material" "(exit=$post_restore_rc; see remote output above)"; fi
else
  section "22-24. backup restore/state/protocol checks"
  block "stages 22-24" "(requires both a successful backup and stage-21 reinstall; dependent failures are not counted separately)"
fi

section "25. final uninstall (offline singbox-vpn-uninstall)"
ssh_run "sudo rm -f $BACKUP_PATH" >/dev/null 2>&1 || true
if ssh_run 'test -x /opt/singbox-vpn/bin/singbox-vpn-uninstall' 2>/dev/null; then
  if ssh_run 'sudo /opt/singbox-vpn/bin/singbox-vpn-uninstall --yes'; then pass "final singbox-vpn-uninstall --yes (offline, local binary only)"; else fail_required "final singbox-vpn-uninstall --yes (offline, local binary only)"; fi
elif ssh_run '[ ! -e /etc/vpn ] && [ ! -e /opt/singbox-vpn ] && [ ! -e /var/lib/singbox-vpn ]' 2>/dev/null; then
  pass "target already clean at final uninstall"
else
  fail_required "final offline uninstall available for partial state"
fi
cleanup_cert_snapshot

section "26. SSH after final uninstall (new connection, port $SSH_PORT)"
if ssh_reconnect 'systemctl is-active --quiet sshd' 2>/dev/null; then pass "SSH still active post-uninstall"; else fail_required "SSH still active post-uninstall"; fi

section "27. final uninstall residue audit (vs. host baseline from stage 1b) — singbox-vpn-owned service/config/state/firewall/sysctl residue"
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
  pass "no singbox-vpn-owned runtime/config/state residue vs. pre-install host baseline"
else
  fail_required "no singbox-vpn-owned residue vs. pre-install host baseline" "(baseline: $BASELINE | after uninstall: $RESIDUE)"
fi

section "manual-only / out-of-scope gates (cannot be automated here — UNVERIFIED, not PASS)"
mark_unverified "public/internet reachability from outside the target's network" "(no independent external controller in this harness)"
mark_unverified "real Hiddify iOS/Android/MagicOS import + connect + sustained traffic" "(client/device property — out of scope for this host lifecycle gate)"
if [ -z "$UPDATE_TO_VERSION" ]; then mark_unverified "real GitHub release A->B update transition" "(no --update-to-version given)"; fi
mark_unverified "reboot-triggered client reconnect from a real device (Hiddify/other) after a server-side reboot" "(requires a second physical client; server recovery is tested above)"

section "summary"
if [ -z "$VERSION" ]; then echo "ACCEPTANCE CLASSIFICATION: DEVELOPMENT LIFECYCLE ONLY — NOT PRODUCTION ACCEPTANCE"; fi
echo "failing stages: $failures"
echo "blocked dependent stages/groups: $blocked"
echo "unverified items: $unverified"
if [ "$unverified" -gt 0 ]; then echo "NOTE: this run has UNVERIFIED items above — do not treat PASS below as full v1.0 release readiness."; fi
if [ "$required_fail" -eq 1 ] || [ "$failures" -gt 0 ]; then
  echo "LIFECYCLE GATE: FAIL"
  exit 1
fi
echo "LIFECYCLE GATE: PASS"
exit 0
