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
# Set to 1 for as long as stage 19's offline-uninstall network block (see
# OFFLINE_BLOCK_REMOTE_FILE below) might still be active on the target, so
# the EXIT/INT/TERM trap knows whether it needs to remove it as a safety
# net against this script being interrupted mid-stage.
OFFLINE_BLOCK_ACTIVE=0
# Remote file recording the exact IPs stage 19 blocked, so the matching
# unblock (whether from the normal code path or the interruption safety
# net above) always removes precisely what was inserted. See the stage 19
# comment for why re-resolving DNS for the delete is not safe here.
OFFLINE_BLOCK_REMOTE_FILE="/tmp/singbox-vpn-lifecycle-offline-block-ips"
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
# For calls that can legitimately run a from-source Rust build on the
# target (installing an unpinned dev-channel ref with no prebuilt release
# available means install.sh/update.sh --dev-rebuild compile the whole
# workspace, plus a rustup toolchain install on a fresh host) — 180s is
# nowhere near enough on a small (~2GB RAM) disposable VPS. Observed
# directly: a real run's stage 2 install hit the 180s ssh_run timeout
# mid-compile and was reported [FAIL], while the build kept running on
# the remote side after the local ssh client was killed and had already
# finished by the very next stage (its idempotent re-run showed "Finished
# ... in 0.09s" — a from-cache no-op — instead of recompiling). That is a
# false FAIL from an impatient controller, not a real installer failure.
ssh_run_long() { timeout 1200 ssh "${SSH_OPTS[@]}" "$HOST" "$@"; }
ssh_reconnect() {
  timeout 30 ssh -o ControlPath=none -p "$SSH_PORT" -o BatchMode=yes -o ConnectTimeout=15 \
    -o StrictHostKeyChecking=accept-new "$HOST" "$@"
}
# Certbot is neither a short probe (ssh_run's 180s) nor a from-source Rust
# build (ssh_run_long's 1200s) — it needs its OWN bounded timeout class.
# Real-VPS evidence: a full `certbot renew --dry-run` (standalone HTTP-01,
# nginx suspend/restore, firewalld temp-open/close) completes in well under
# a minute; 360s locally / 300s remotely leaves generous headroom without
# ever approaching ssh_run_long's 20-minute budget, which is what made a
# genuinely finished renewal look permanently frozen.
# -n: this is a fully noninteractive remote command (see the sentinel-based
# script this wraps) — it never needs to read the controller's own stdin.
# ServerAliveInterval/CountMax: detect a stalled TCP session (not just a
# hung remote command) within ~60s instead of waiting out the full local
# timeout. `-k 10s`: guarantees a SIGKILL 10s after SIGTERM if ssh itself
# (or anything it's waiting on) ignores the terminate signal, so this can
# never hang past 370s regardless of what the remote end does.
ssh_run_certbot() {
  timeout -k 10s 360s ssh "${SSH_OPTS[@]}" -n \
    -o ServerAliveInterval=15 -o ServerAliveCountMax=4 \
    "$HOST" "$@"
}

# Uninstall performs systemd/Certbot/nginx/firewall/SELinux/package/Rust
# cleanup — plausibly more than certbot's quick dry-run but nowhere near a
# from-source Rust build, so it gets its own bounded timeout class rather
# than reusing either ssh_run's 180s (too tight — a previous run
# misclassified an uninstall this short timeout cut off mid-run as a
# false FAIL) or ssh_run_long's 1200s (too loose to ever notice a genuine
# hang promptly). No real-VPS timing measurement for this exact stage was
# available when this was written (this change was made without SSH
# access to a live host) — 480s remote / 540s outer is a documented
# starting point inside the same evidence-based-headroom shape as
# ssh_run_certbot above, not a blind guess; re-tune from a real run's
# actual elapsed time if it proves too tight or unnecessarily loose.
ssh_run_uninstall() {
  timeout -k 10s 540s ssh "${SSH_OPTS[@]}" -n \
    -o ServerAliveInterval=15 -o ServerAliveCountMax=4 \
    "$HOST" "$@"
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

# Local-controller-side temp state this script itself creates (currently:
# stage 18's certbot streaming log — see ssh_run_certbot()/section 18
# below). Cleaned up on every exit path, not just the happy one, so an
# interrupted or failing run never leaves a stray file behind. Stage 18
# also removes its own log immediately after use as defense in depth; this
# trap exists for every other exit (Ctrl-C, an earlier stage's failure
# under `set -e`, etc).
CERTBOT_LOG_FILE=""
cleanup_lifecycle_tmp() {
  [ -n "$CERTBOT_LOG_FILE" ] && rm -f "$CERTBOT_LOG_FILE" 2>/dev/null
  # Safety net: if this script is interrupted between stage 19 applying
  # the offline-uninstall network block and it removing that same block,
  # don't leave the target permanently unable to reach GitHub. Reuses the
  # exact recorded IPs, not a fresh DNS lookup (see stage 19).
  if [ "$OFFLINE_BLOCK_ACTIVE" = "1" ]; then
    ssh_run "while read -r ip; do [ -n \"\$ip\" ] && sudo iptables -D OUTPUT -d \"\$ip\" -j REJECT 2>/dev/null; done < $OFFLINE_BLOCK_REMOTE_FILE 2>/dev/null; rm -f $OFFLINE_BLOCK_REMOTE_FILE" >/dev/null 2>&1 || true
  fi
  return 0
}
trap cleanup_lifecycle_tmp EXIT INT TERM

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

# Shared by both offline-uninstall stages (19 and the later "25. final
# uninstall") so each gets the same real classification instead of one
# opaque FAIL — a real VPS run previously reported bare
# "[FAIL][required] singbox-vpn-uninstall --yes (offline, local binary
# only)" with almost no other diagnostic, right after stage 18's certbot
# gate had just been forcibly timed out; there was no way from that
# output alone to tell a genuine uninstaller defect apart from
# after-effects of the timeout immediately before it (a stuck lock, a
# still-running certbot/hook process, etc.) — see ssh_run_uninstall()
# above and the classification below for how this now tells those apart.
# $1: the exact PASS/FAIL step label used for this call site (matches the
# pre-existing text for the two original call sites, so no existing
# assertion tied to that text needs to change).
run_uninstall_classified() {
  local step_label="$1"
  local sentinel='__SINGBOX_VPN_UNINSTALL_DONE__'
  # Same shape as stage 18's remote_certbot_cmd: `\$?`/`\$rc` stay escaped
  # so they are evaluated on the REMOTE shell after `timeout` returns,
  # never by this controller.
  local remote_cmd="set +e
timeout -k 10s 480s sudo /opt/singbox-vpn/bin/singbox-vpn-uninstall --yes
rc=\$?
printf '\n${sentinel} rc=%d\n' \"\$rc\"
exit \"\$rc\""
  local log_file rc out sentinel_line sentinel_present=0 sentinel_rc="" uclass=""
  log_file="$(mktemp)"
  chmod 600 "$log_file" 2>/dev/null || true
  # set +e / PIPESTATUS / set -e: same reasoning as stage 18 — a
  # genuinely nonzero/timed-out uninstall must reach the classification
  # below, not kill the whole harness under this script's own
  # `set -Eeuo pipefail` right at this line. tee here keeps the
  # uninstaller's own output streaming live to the operator (it is
  # destructive and release-critical) while still capturing it.
  set +e
  ssh_run_uninstall "$remote_cmd" 2>&1 | tee "$log_file"
  rc=${PIPESTATUS[0]}
  set -e
  out="$(cat "$log_file" 2>/dev/null || true)"
  rm -f "$log_file"

  sentinel_line="$(printf '%s\n' "$out" | grep -F "$sentinel" | tail -1 || true)"
  if [[ "$sentinel_line" =~ rc=([0-9]+) ]]; then
    sentinel_present=1
    sentinel_rc="${BASH_REMATCH[1]}"
  fi

  if [ "$sentinel_present" -eq 0 ]; then
    if [ "$rc" -eq 124 ] || [ "$rc" -eq 137 ]; then
      uclass="UNINSTALL_SSH_TIMEOUT"
    else
      uclass="UNINSTALL_SSH_SESSION_ENDED_UNEXPECTEDLY"
    fi
  elif [ "$sentinel_rc" = "124" ] || [ "$sentinel_rc" = "137" ]; then
    uclass="UNINSTALL_REMOTE_TIMEOUT"
  elif [ "$sentinel_rc" != "0" ]; then
    uclass="UNINSTALL_EXIT_NONZERO"
    # Distinguish lock contention from a generic failure without having
    # to guess/match uninstall.sh's own error text: check directly
    # whether a singbox-vpn-owned lock file is still held by another
    # process (flock -n fails immediately if so; a stale, unheld lock
    # file is not contention and is left alone here either way).
    if ! ssh_run '
      held=0
      for l in /run/lock/singbox-vpn-installer.lock /run/lock/singbox-vpn.lock; do
        [ -e "$l" ] || continue
        flock -n -w 0 "$l" true 2>/dev/null || held=1
      done
      [ "$held" -eq 0 ]
    ' 2>/dev/null; then
      uclass="UNINSTALL_LOCK_CONTENTION"
    fi
  fi

  if [ -z "$uclass" ]; then
    pass "$step_label"
    return 0
  fi

  # Diagnostics only, and only when the transport itself is known to be
  # alive (a genuine SSH timeout means the remote state is unknown from
  # here — a second remote call right after would just time out again).
  # Never the full uninstall log a second time, never anything from
  # /etc/vpn or the environment: lock/process/package-manager/nginx state
  # only, matching stage 18's diagnostics scope.
  local diag=""
  if [ "$uclass" != "UNINSTALL_SSH_TIMEOUT" ]; then
    diag="$(ssh_run '
      echo "--- lock files ---"
      ls -la /run/lock/singbox-vpn* 2>/dev/null || echo none
      echo "--- dnf/rpm ---"
      pgrep -x dnf >/dev/null 2>&1 && echo dnf-running || echo dnf-not-running
      [ -e /var/lib/rpm/.rpm.lock ] && echo rpm-lock-present || echo rpm-lock-absent
      echo "--- certbot ---"
      pgrep -f certbot >/dev/null 2>&1 && echo certbot-running || echo certbot-not-running
      echo "--- nginx ---"
      systemctl is-active nginx 2>&1 || true
      echo "--- related processes ---"
      ps -eo pid,ppid,stat,cmd 2>/dev/null | grep -E "singbox-vpn-uninstall|certbot|dnf|rpm" | grep -v grep || echo none
    ' 2>&1 || true)"
  fi
  local concise
  concise="$(printf '%s\n' "$out" | tail -20)"
  fail_required "$step_label [$uclass]" "(exit=$rc; sentinel=${sentinel_present}/${sentinel_rc:-none}; output: ${concise:-none}; diagnostics: ${diag:-none})"
  return 1
}

# Shared by stage 19 (offline uninstall, while a reinstall is still
# planned) and stage 27 (the true final uninstall) — the same
# singbox-vpn-owned-residue-vs-baseline field diff, called with each
# stage's own PASS/FAIL wording so neither call site's existing assertion
# text has to change. See stage 27's original comment (preserved at its
# call site below) for why this is a field-by-field "nothing NEW beyond
# baseline" comparison, never exact-string equality.
# $1: PASS message. $2: FAIL message (diagnostic detail is appended).
check_residue_vs_baseline() {
  local pass_msg="$1" fail_msg="$2"
  local residue new_residue field after_val baseline_val
  residue="$(ssh_run '
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
  new_residue=""
  while IFS='=' read -r field after_val; do
    [ -n "$field" ] || continue
    baseline_val="$(printf '%s\n' "$BASELINE" | awk -F= -v k="$field" '$1==k{print $2; exit}')"
    baseline_val="${baseline_val:-0}"
    if [[ "$after_val" =~ ^[0-9]+$ ]] && [[ "$baseline_val" =~ ^[0-9]+$ ]] && [ "$after_val" -gt "$baseline_val" ]; then
      new_residue="$new_residue $field(baseline=$baseline_val,after=$after_val)"
    fi
  done <<RESIDUE_FIELDS
$residue
RESIDUE_FIELDS
  if [ -z "$new_residue" ]; then
    pass "$pass_msg"
    return 0
  fi
  fail_required "$fail_msg" "($new_residue | full baseline: $BASELINE | full after: $residue)"
  return 1
}

# SINGBOX_VPN_SUPPRESS_ONBOARDING_SECRETS=1: this harness's own transcript
# (this stdout/stderr) is exactly the kind of thing that ends up in a CI
# log or release-evidence bundle, and install.sh's normal onboarding
# output includes the real subscription URL/QR — the app's own text
# calls that URL "the credential — treat it like a password". Automated
# runs must never print it (see install.sh's ensure_first_user()/
# print_status() and apps/admin/src/main.rs's suppress_onboarding_secrets()
# for how it's suppressed at the source rather than filtered after the
# fact); a human running install.sh directly is unaffected.
run_install() {
  ssh_run_long "set -o pipefail; curl -fsSL $REMOTE_BOOTSTRAP_CURL_FLAGS https://raw.githubusercontent.com/David610/singbox-vpn/$BOOTSTRAP_REF/install.sh | SINGBOX_VPN_SUPPRESS_ONBOARDING_SECRETS=1 $INSTALL_SOURCE_ENV REALITY_HANDSHAKE_SERVER=www.google.com SINGBOX_VPN_ALLOW_IP_HOSTNAME=1 bash -s -- $(install_args_quoted)"
}

run_install_abort_after_singbox() {
  ssh_run_long "set -o pipefail; curl -fsSL $REMOTE_BOOTSTRAP_CURL_FLAGS https://raw.githubusercontent.com/David610/singbox-vpn/$BOOTSTRAP_REF/install.sh | sudo SINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER=install_singbox SINGBOX_VPN_SUPPRESS_ONBOARDING_SECRETS=1 $INSTALL_SOURCE_ENV REALITY_HANDSHAKE_SERVER=www.google.com SINGBOX_VPN_ALLOW_IP_HOSTNAME=1 bash -s -- $(install_args_quoted)"
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
    # Run every check (not a short-circuiting `&&` chain) and report which
    # ones actually failed. The previous all-or-nothing `&&` chain with a
    # blanket `2>/dev/null` gave zero diagnostic value on failure — a real
    # run reported [FAIL][required] here with nothing to indicate whether
    # sing-box, vpn-subscription, nginx, either timer, the :443 listener,
    # install-state.json, or the protocol self-test was the actual cause.
    POST_REBOOT_OUT=""
    if [ "$reboot_ok" -eq 1 ]; then
      # `|| true`: this is a plain assignment under `set -e` — without it,
      # a real (expected, reportable) remote failure here would make
      # ssh_run itself exit non-zero and kill this ENTIRE script right at
      # this line, silently, before ever reaching the fail_required below
      # that's supposed to report it. Reproduced directly: a simulated
      # nginx-down-after-reboot case with this guard missing vanished the
      # whole run instead of producing the intended [FAIL][required] line.
      POST_REBOOT_OUT="$(ssh_run '
        fails=""
        sudo /opt/singbox-vpn/deploy/almalinux/health-check.sh || fails="$fails health-check.sh"
        systemctl is-active --quiet sing-box || fails="$fails sing-box"
        systemctl is-active --quiet vpn-subscription || fails="$fails vpn-subscription"
        systemctl is-active --quiet nginx || fails="$fails nginx"
        systemctl is-active --quiet vpn-expiry-reconcile.timer || fails="$fails vpn-expiry-reconcile.timer"
        systemctl is-active --quiet vpn-service-watchdog.timer || fails="$fails vpn-service-watchdog.timer"
        ss -ltn 2>/dev/null | grep -q ":443 " || fails="$fails :443-listener"
        sudo test -s /var/lib/singbox-vpn/install-state.json || fails="$fails install-state.json"
        sudo /usr/local/bin/vpn-admin doctor --protocol || fails="$fails doctor--protocol"
        if [ -n "$fails" ]; then echo "POST_REBOOT_FAILED_CHECKS:$fails"; exit 1; fi
        echo "POST_REBOOT_ALL_OK"
      ' 2>&1)" || true
    fi
    if [ "$reboot_ok" -eq 1 ] && grep -q 'POST_REBOOT_ALL_OK' <<< "$POST_REBOOT_OUT"; then
      pass "reboot + independent post-reboot verification (sshd/sing-box/subscription/nginx/timers incl. watchdog/listener/install-state/protocol)"
    elif [ "$reboot_ok" -ne 1 ]; then
      fail_required "reboot + independent post-reboot verification" "(host never reconnected within the post-reboot polling budget)"
    else
      failed_checks="$(grep -o 'POST_REBOOT_FAILED_CHECKS:.*' <<< "$POST_REBOOT_OUT")"
      fail_required "reboot + independent post-reboot verification" "(${failed_checks:-no POST_REBOOT_ALL_OK/POST_REBOOT_FAILED_CHECKS marker in output — see remote output above})"
      [ -n "$POST_REBOOT_OUT" ] && printf '%s\n' "$POST_REBOOT_OUT"
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
  if ssh_run "sudo /usr/local/bin/vpn-admin user create --name $TEST_USER_NAME --json" >/dev/null 2>&1 \
    && ssh_run "sudo /usr/local/bin/vpn-admin user list | grep -q $TEST_USER_NAME" 2>/dev/null; then
    pass "test user created ($TEST_USER_NAME)"
  else
    fail_required "test user created ($TEST_USER_NAME)"
  fi

  section "10. vpn-admin doctor (standard checks, no protocol self-test)"
  if ssh_run 'sudo /usr/local/bin/vpn-admin doctor' 2>/dev/null; then pass "vpn-admin doctor"; else fail_required "vpn-admin doctor"; fi

  section "11. REALITY authentication proof (vpn-admin doctor --protocol --require-protocol)"
  if PROTOCOL_OUT="$(ssh_run 'sudo /usr/local/bin/vpn-admin doctor --protocol --require-protocol' 2>&1)"; then protocol_rc=0; else protocol_rc=$?; fi
  if [ "$protocol_rc" -eq 0 ] && printf '%s' "$PROTOCOL_OUT" | grep -q 'completed a full handshake'; then
    pass "REALITY handshake self-test PASSED (real sing-box client, live public_key/short_id, application bytes end-to-end)"
  else
    fail_required "REALITY handshake self-test (doctor --protocol --require-protocol)" "(exit=$protocol_rc; see remote output above)"
  fi

  section "12. Hysteria2 real handshake+transfer proof (deploy/lib/vpn-benchmark.sh)"
  if HY_OUT="$(ssh_run "sudo /opt/singbox-vpn/deploy/lib/vpn-benchmark.sh --runs 1 --download-url 'https://speed.cloudflare.com/__down?bytes=2000000'" 2>&1)"; then hy_rc=0; else hy_rc=$?; fi
  hy_block="$(printf '%s\n' "$HY_OUT" | sed -n '/^Hysteria2 protocol\/server-side overhead/,/^Assessment$/p' || true)"
  # vpn-benchmark.sh's `kv "throughput (Mbps), N run(s)" "min=... median=... max=..."`
  # prints key and value on the SAME line — a real run showed
  # "throughput (Mbps), 1 run(s): min=139.13 median=139.13 max=139.13
  # (n=1)" as one line. `grep -A1 | tail -1` (the old form here) grabbed
  # the line AFTER the match instead of the match itself, so hy_min_mbps
  # below was always empty and this stage FAILed on every real,
  # successful benchmark run — reproduced directly against real
  # benchmark output, not a guess.
  hy_line="$(printf '%s\n' "$hy_block" | grep -m1 'throughput (Mbps)' || true)"
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
  #
  # Record whatever state the timer was ACTUALLY in beforehand so recovery
  # below restores that, never unconditionally "turns it on" — this harness
  # must never change host state as a side effect of a test's own plumbing.
  if ssh_run 'systemctl is-active --quiet vpn-service-watchdog.timer' 2>/dev/null; then
    watchdog_timer_was_active_before=1
  else
    watchdog_timer_was_active_before=0
  fi

  watchdog_timer_suspended=0
  # `systemctl is-inactive` is NOT a real systemctl verb — confirmed
  # directly (systemd rejects it: "Unknown command verb 'is-inactive', did
  # you mean 'is-active'?", exit 1). The previous check here used it, so
  # this compound command failed on every real run regardless of whether
  # `systemctl stop` itself actually worked — the true root cause of this
  # stage's failure on both real-VPS runs, not a systemd race. Use an
  # explicit ActiveState read instead: it is `is-active`-equivalent and
  # gives a real value to report on failure too.
  if WATCHDOG_SUSPEND_OUT="$(ssh_run '
      sudo systemctl stop vpn-service-watchdog.timer
      state="$(systemctl show -p ActiveState --value vpn-service-watchdog.timer)"
      echo "ActiveState=$state"
      [ "$state" = "inactive" ]
    ' 2>&1)"; then
    pass "vpn-service-watchdog.timer suspended for deterministic crash-loop test"
    watchdog_timer_suspended=1
  else
    WATCHDOG_TIMER_STATE="$(ssh_run 'systemctl show -p ActiveState,SubState,Result --value vpn-service-watchdog.timer' 2>/dev/null || true)"
    fail_required "suspend vpn-service-watchdog.timer for crash-loop test" "(command output: ${WATCHDOG_SUSPEND_OUT:-none}; timer ActiveState/SubState/Result: ${WATCHDOG_TIMER_STATE:-unknown})"
  fi

  failed_state_ready=0
  if [ "$watchdog_timer_suspended" -eq 1 ]; then
    # Clear any start-limit accounting sing-box already picked up from
    # stage 13's single SIGKILL+restart just above, so this loop's 40s
    # budget only has to exhaust StartLimitBurst=8 from a known-zero
    # baseline (`reset-failed` is systemd's documented mechanism for this
    # — see sing-box.service's StartLimitIntervalSec=300/StartLimitBurst=8
    # comment) rather than depending on how many restarts already
    # happened earlier in this run.
    ssh_run 'sudo systemctl reset-failed sing-box' >/dev/null 2>&1 || true
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
      FAILED_STATE_DIAG="$(ssh_run 'systemctl show sing-box -p ActiveState -p SubState -p Result -p NRestarts -p MainPID' 2>/dev/null || true)"
      fail_required "sing-box.service did not reach FAILED state after a fast repeated-crash burst" "(restart budget was not exhausted deterministically; state: ${FAILED_STATE_DIAG:-unknown})"
    fi
  else
    block "crash-loop FAILED-state creation" "(watchdog timer could not be suspended, so the observation would be racy)"
  fi

  if [ "$failed_state_ready" -eq 1 ]; then
    if DOCTOR_DURING_FAILURE_OUT="$(ssh_run 'sudo /usr/local/bin/vpn-admin doctor' 2>&1)"; then doctor_during_failure_rc=0; else doctor_during_failure_rc=$?; fi
    if [ "$doctor_during_failure_rc" -ne 0 ] && printf '%s' "$DOCTOR_DURING_FAILURE_OUT" | grep -qi 'sing-box.service is in a FAILED state'; then
      pass "vpn-admin doctor correctly reports sing-box.service as FAILED (distinct from merely 'not active')"
    else
      doctor_during_failure_excerpt="$(printf '%s\n' "$DOCTOR_DURING_FAILURE_OUT" | tail -20)"
      fail_required "vpn-admin doctor did not report the FAILED sing-box.service" "(exit=$doctor_during_failure_rc; output: ${doctor_during_failure_excerpt:-none})"
    fi
    STATUS_DURING_FAILURE_OUT="$(ssh_run 'sudo /usr/local/bin/vpn-admin status' 2>&1 || true)"
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
    if [ "$watchdog_timer_was_active_before" -eq 1 ]; then
      if ssh_run 'sudo systemctl start vpn-service-watchdog.timer && systemctl is-active --quiet vpn-service-watchdog.timer' 2>/dev/null; then
        pass "vpn-service-watchdog.timer re-armed after deterministic crash-loop test"
      else
        fail_required "re-arm vpn-service-watchdog.timer after crash-loop test"
      fi
    else
      # It was NOT active before this stage touched it (unusual — a
      # normal install enables it — but this harness must only ever
      # restore prior state, never activate a timer that was
      # intentionally left inactive for some other reason).
      pass "vpn-service-watchdog.timer left inactive after the crash-loop test (restoring its pre-test state, not activating it)"
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
  # Clear any stale start-limit-hit state before the deliberate restart
  # below: this test has already SIGKILLed sing-box once (stage 13) and
  # will start it again here, all inside a short window — reset-failed on
  # a unit that is NOT in `failed` state is a documented no-op, so this
  # is safe regardless of whether StartLimitBurst is actually involved.
  ssh_run 'sudo systemctl reset-failed sing-box' >/dev/null 2>&1 || true
  singbox_restored_after_stop=0
  if START_SINGBOX_OUT="$(ssh_run 'sudo systemctl start sing-box' 2>&1)"; then
    pass "sing-box restarted normally after the deliberate-stop test (restoring state for the rest of this run)"
    singbox_restored_after_stop=1
  else
    # Reproduced twice on real VPS runs with zero diagnostic output (the
    # old check discarded both stdout and stderr). None of this leaks
    # secrets: systemctl show/status/journalctl report unit/process state,
    # not sing-box's config contents, and `sing-box check` only validates
    # the config file (pass/fail), it never prints it.
    START_SINGBOX_DIAG="$(ssh_run '
      echo "--- systemctl show ---"
      systemctl show sing-box -p ActiveState -p SubState -p Result -p ExecMainStatus -p ExecMainCode -p NRestarts -p MainPID
      echo "--- systemctl status (last 50 lines) ---"
      systemctl status sing-box --no-pager -l --lines=50
      echo "--- journalctl (last 80 lines) ---"
      journalctl -u sing-box --no-pager -n 80
      echo "--- config validation (contents never printed) ---"
      sudo /usr/local/bin/sing-box check -c /etc/vpn/compat/sing-box/config.json
    ' 2>&1 || true)"
    fail_required "sing-box restarted normally after the deliberate-stop test" "(start command output: ${START_SINGBOX_OUT:-none}; diagnostics: ${START_SINGBOX_DIAG:-unknown})"
  fi

  section "14. protocol works after recovery (re-run doctor --protocol --require-protocol)"
  if [ "$singbox_restored_after_stop" -eq 1 ]; then
    if POST_RECOVERY_PROTOCOL_OUT="$(ssh_run 'sudo /usr/local/bin/vpn-admin doctor --protocol --require-protocol' 2>&1)"; then post_recovery_rc=0; else post_recovery_rc=$?; fi
    if [ "$post_recovery_rc" -eq 0 ] && printf '%s' "$POST_RECOVERY_PROTOCOL_OUT" | grep -q 'completed a full handshake'; then
      pass "REALITY handshake self-test still PASSES after the SIGKILL+recovery cycle"
    else
      # doctor --protocol's output is descriptive prose about pass/fail
      # conditions (verified against apps/admin/src/main.rs's
      # check_l5_l6_protocol_selftest) — it never prints raw key/token
      # material, so a bounded excerpt is safe to include here instead of
      # claiming output appears "above" when it was only ever captured
      # into a variable and never actually printed.
      post_recovery_excerpt="$(printf '%s\n' "$POST_RECOVERY_PROTOCOL_OUT" | tail -20)"
      fail_required "REALITY handshake self-test after recovery" "(exit=$post_recovery_rc; output: ${post_recovery_excerpt:-none})"
    fi
  else
    block "REALITY handshake self-test after recovery" "(stage 13c could not restart sing-box; this test's prerequisite service is down, so a failure here would not be an independent protocol defect)"
  fi

  section "15. user rotate/disable/remove sanity (scratch user; does not touch the persisted test user above)"
  # Report only the failing step name. Never echo rotate-token/create output
  # because both contain credentials (the create response includes the
  # subscription URL/token).
  if [ "$singbox_restored_after_stop" -ne 1 ]; then
    # Verified directly (apps/admin/src/main.rs: cmd_user_create ->
    # apply_users_and_save -> render_and_apply_singbox_config) that `user
    # create`/rotate-* all reload sing-box as part of applying the change
    # and `bail!` if that reload fails — so none of this is meaningfully
    # testable while sing-box is down; a failure here would just be the
    # same 13c prerequisite failure again, not a separate user-lifecycle
    # defect.
    block "scratch user create/rotate/disable/remove" "(stage 13c could not restart sing-box; user create/rotate cannot succeed while it's down)"
  elif SCRATCH_RESULT="$(ssh_run '
    scratch_id=""
    cleanup_scratch() { [ -z "$scratch_id" ] || sudo /usr/local/bin/vpn-admin user remove "$scratch_id" >/dev/null 2>&1 || true; }
    trap cleanup_scratch EXIT
    # Capture the create command exit status BEFORE piping its
    # output through any parser: piping directly into `grep | head | sed`
    # (the old form here) loses that exit status entirely, because `sed`
    # exits 0 even on empty input — a real `vpn-admin user create`
    # failure (e.g. sing-box reload rejected) was silently misreported as
    # "parse-id" instead of "create". `--json` output is not pure JSON —
    # it also emits human-readable status lines first (verified in
    # cmd_user_create/render_and_apply_singbox_config), so the id is
    # still extracted with grep -o against the whole captured text.
    create_output="$(sudo /usr/local/bin/vpn-admin user create --name lifecycle-scratch-user --json)" || { echo create; exit 1; }
    scratch_id="$(printf "%s\n" "$create_output" | grep -o "\"id\": *\"[^\"]*\"" | head -1 | sed -E "s/.*\"([^\"]+)\"$/\1/")"
    [ -n "$scratch_id" ] || { echo parse-id; exit 1; }
    sudo /usr/local/bin/vpn-admin user list | grep -q "$scratch_id" || { echo list; exit 1; }
    sudo /usr/local/bin/vpn-admin user rotate-token "$scratch_id" >/dev/null || { echo rotate-token; exit 1; }
    sudo /usr/local/bin/vpn-admin user rotate-vless "$scratch_id" >/dev/null || { echo rotate-vless; exit 1; }
    sudo /usr/local/bin/vpn-admin user rotate-hysteria "$scratch_id" >/dev/null || { echo rotate-hysteria; exit 1; }
    sudo /usr/local/bin/vpn-admin user disable "$scratch_id" >/dev/null || { echo disable; exit 1; }
    sudo /usr/local/bin/vpn-admin user remove "$scratch_id" >/dev/null || { echo remove; exit 1; }
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
    if ssh_reconnect 'systemctl is-active --quiet sshd && systemctl is-active --quiet sing-box && systemctl is-active --quiet vpn-subscription && sudo /usr/local/bin/vpn-admin doctor --protocol' 2>/dev/null; then
      post_rollback_version="$(ssh_run 'sudo cat /var/lib/singbox-vpn/install-state.json 2>/dev/null' 2>/dev/null || true)"
      if [ "$pre_rollback_version" = "$post_rollback_version" ]; then pass "production repair rollback restored the prior working state"; else fail_required "production repair rollback restored prior state" "(install-state.json differs)"; fi
    else
      fail_required "production repair rollback left services/protocol/SSH healthy"
    fi
  elif [ -n "$UPDATE_TO_REF" ]; then
    section "16. safe update path -> $UPDATE_TO_REF (--dev-rebuild; transactional updater machinery only)"
    version_before="$(ssh_run 'sudo /opt/singbox-vpn/bin/vpn-admin --version 2>/dev/null || sudo cat /var/lib/singbox-vpn/install-state.json 2>/dev/null' 2>/dev/null || true)"
    if ssh_run_long "curl -fsSL --connect-timeout 10 --max-time 120 --retry 5 --retry-delay 2 --retry-connrefused -o /tmp/singbox-vpn-update-ref.tar.gz https://codeload.github.com/David610/singbox-vpn/tar.gz/refs/heads/$(printf '%q' "$UPDATE_TO_REF") \
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
    if ssh_run_long "sudo SINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER=after_switch /opt/singbox-vpn/deploy/almalinux/update.sh --dev-rebuild" 2>/dev/null; then fail_required "failed update aborted as expected" "(expected non-zero exit, got success)"; else pass "failed update aborted as expected"; fi
    if ssh_reconnect 'systemctl is-active --quiet sshd && systemctl is-active --quiet sing-box && systemctl is-active --quiet vpn-subscription && sudo /usr/local/bin/vpn-admin doctor --protocol' 2>/dev/null; then
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
  PRE_BACKUP_USERLIST="$(ssh_run 'sudo /usr/local/bin/vpn-admin user list' 2>/dev/null || true)"
  if ssh_run "sudo /usr/local/bin/vpn-admin backup --output $BACKUP_PATH" 2>/dev/null && ssh_run "sudo test -s $BACKUP_PATH" 2>/dev/null; then
    pass "vpn-admin backup produced a non-empty archive at $BACKUP_PATH"
    BACKUP_READY=1
  else
    fail_required "vpn-admin backup produced a non-empty archive"
  fi

  section "18. certbot renew --dry-run (while the deployment is still live, before the destructive uninstall below)"
  # Previously this stage hid the ENTIRE certbot run inside a plain
  # `VAR="$(ssh_run_long ...)"` command substitution: zero output reached
  # the operator until the whole thing finished, and it inherited
  # ssh_run_long's 1200s timeout — sized for a from-source Rust build, not
  # a certificate renewal. A real VPS run showed this looking frozen for
  # a long time even though the renewal itself, and the pre/post-hook
  # nginx/firewall dance around it, had already completed successfully
  # (confirmed independently on that same host: no leftover certbot/hook
  # processes, nginx active, TCP/80 closed again, zero renewal failures).
  # This stage now streams certbot's own output live (via `tee`) while
  # still capturing it for the assertions below, bounds both the remote
  # certbot invocation and the local SSH session on ssh_run_certbot's own
  # certbot-sized timeout (not ssh_run_long's), and appends a
  # project-specific completion sentinel + real exit code so a genuine
  # hang can be told apart from a renewal that finished but whose SSH
  # session failed to tear down promptly.
  CERTBOT_SENTINEL='__SINGBOX_VPN_CERTBOT_DONE__'

  # A bare `certbot renew --dry-run` selects EVERY lineage certbot knows
  # about on the host, not just this deployment's — a real VPS run failed
  # this stage because an unrelated, unrenewable lineage
  # (vpn.sustechnologies.eu, since gone NXDOMAIN) happened to also live on
  # that host, even though THIS deployment's own certificate renewed
  # cleanly. Scope every remote invocation to this deployment's own
  # lineage via `--cert-name`, determined from authoritative remote state
  # (deployment.toml's public_host — the same value install.sh's own
  # `certbot certonly -d "$host"` used to create the lineage in the first
  # place), never from this controller's own --domain input (may be
  # empty/stale) and never by interpolating a hostname string directly
  # into a remote shell. The remote side extracts, allowlist-validates
  # (rejects anything containing a shell metacharacter), and looks up the
  # real certbot lineage itself — the same pattern already used by
  # capture_cert_for_reuse() above — and this controller re-validates the
  # returned name against the same allowlist before ever using it again.
  cert_lookup_cmd='
host="$(sed -nE "s/^public_host[[:space:]]*=[[:space:]]*\"([^\"]+)\".*/\1/p" /etc/vpn/deployment.toml 2>/dev/null | head -1)"
case "$host" in
  "") echo "CERT_LOOKUP:NO_DEPLOYMENT_HOST"; exit 2 ;;
  *[!A-Za-z0-9.-]*) echo "CERT_LOOKUP:INVALID_HOST"; exit 2 ;;
esac
info="$(sudo certbot certificates --cert-name "$host" 2>/dev/null)"
if ! printf "%s\n" "$info" | grep -qE "^[[:space:]]*Certificate Name:[[:space:]]*${host}\$"; then
  echo "CERT_LOOKUP:NO_MATCHING_LINEAGE:$host"
  exit 3
fi
if ! printf "%s\n" "$info" | grep -E "^[[:space:]]*Domains:" | grep -qE "(^|[[:space:]])${host}([[:space:]]|\$)"; then
  echo "CERT_LOOKUP:DOMAIN_MISMATCH:$host"
  exit 4
fi
echo "CERT_LOOKUP:OK:$host"
'
  cert_lookup_out="$(ssh_run "$cert_lookup_cmd" 2>&1 || true)"
  cert_name=""
  if [[ "$cert_lookup_out" =~ CERT_LOOKUP:OK:([A-Za-z0-9.-]+) ]]; then
    cert_name="${BASH_REMATCH[1]}"
  fi
  # Defense in depth: re-validate locally against the identical allowlist
  # even though the remote side already enforced it — this value is about
  # to be interpolated into a second remote command string below, and
  # this stage must never trust a value crossing that boundary on the
  # strength of a single check alone.
  if [[ -n "$cert_name" ]] && [[ ! "$cert_name" =~ ^[A-Za-z0-9.-]+$ ]]; then
    cert_name=""
  fi

  if [ -n "$cert_name" ]; then
  # certbot added --no-random-sleep-on-renew in 1.25.0 (Nov 2022) to make
  # `renew` deterministic for exactly this kind of scripted acceptance
  # run; older installations don't recognize it. This cannot be assumed
  # for an arbitrary AlmaLinux 9 host without checking what that host
  # actually has installed — probed live via `certbot --help renew`
  # rather than hardcoded from a version guess, so a real run's log shows
  # which case actually applied instead of silently assuming one.
  no_random_sleep_flag=""
  if ssh_run 'sudo certbot --help renew 2>&1 | grep -q -- "--no-random-sleep-on-renew"' 2>/dev/null; then
    no_random_sleep_flag=" --no-random-sleep-on-renew"
  fi
  cert_name_q="$(printf '%q' "$cert_name")"
  # Single-quoted local variable for the parts with no local
  # interpolation, then re-opened as a double-quoted string only for the
  # two values that must actually be substituted here (cert_name_q,
  # no_random_sleep_flag) — `\$?`/`\$rc` stay escaped so they are
  # evaluated on the REMOTE shell, after the inner `timeout` returns,
  # never by this controller. The inner timeout (300s, 60s inside
  # ssh_run_certbot's own 360s/-k 10s outer bound) is what actually stops
  # a wedged certbot/hook; the sentinel line is only ever printed AFTER
  # that inner command has already exited, so its presence proves the
  # remote command genuinely finished (see the classification logic
  # below — the sentinel is evidence, never a substitute for checking the
  # real exit code carried right next to it).
  remote_certbot_cmd="set +e
timeout -k 10s 300s sudo certbot renew --dry-run --cert-name ${cert_name_q}${no_random_sleep_flag}
rc=\$?
printf '\n__SINGBOX_VPN_CERTBOT_DONE__ rc=%d\n' \"\$rc\"
exit \"\$rc\""

  # Baseline TCP/80 firewalld state BEFORE this attempt, so the
  # post-renewal check below can tell singbox-vpn's own temporary hook
  # rule (real residue if still open afterward) apart from an operator's
  # own pre-existing TCP/80 allow rule (not this run's to close) — see
  # certbot-firewall-pre-hook.sh's ownership-tracking rationale. Best
  # effort: a failure here just means the post-check below cannot assert
  # the firewall half of the cleanup contract, not that the whole stage
  # is blocked.
  port80_before="$(ssh_run '
    if command -v firewall-cmd >/dev/null 2>&1 && systemctl is-active --quiet firewalld 2>/dev/null; then
      firewall-cmd --query-port=80/tcp >/dev/null 2>&1 && echo open || echo closed
    else
      echo n/a
    fi
  ' 2>/dev/null || echo unknown)"

  # set +e / PIPESTATUS / set -e: required here — under this script's own
  # `set -Eeuo pipefail`, a pipeline that exits non-zero (a real, expected
  # outcome: a genuine certbot failure or timeout) would otherwise kill
  # the ENTIRE script right at this line, before the classification logic
  # below ever runs. PIPESTATUS[0] (not `$?`, which pipefail would set
  # from the LAST failing stage of the pipe) is what actually carries
  # ssh's/timeout's real exit code — `tee`'s own exit status is
  # irrelevant here and must never override it.
  CERTBOT_LOG_FILE="$(mktemp)"
  chmod 600 "$CERTBOT_LOG_FILE" 2>/dev/null || true
  set +e
  ssh_run_certbot "$remote_certbot_cmd" 2>&1 | tee "$CERTBOT_LOG_FILE"
  certbot_dry_rc=${PIPESTATUS[0]}
  set -e
  CERTBOT_DRY_OUT="$(cat "$CERTBOT_LOG_FILE" 2>/dev/null || true)"
  rm -f "$CERTBOT_LOG_FILE"
  CERTBOT_LOG_FILE=""

  sentinel_line="$(printf '%s\n' "$CERTBOT_DRY_OUT" | grep -F "$CERTBOT_SENTINEL" | tail -1 || true)"
  sentinel_present=0
  sentinel_rc=""
  if [[ "$sentinel_line" =~ rc=([0-9]+) ]]; then
    sentinel_present=1
    sentinel_rc="${BASH_REMATCH[1]}"
  fi
  zero_renewals=0
  printf '%s' "$CERTBOT_DRY_OUT" | grep -qF 'No simulated renewals were attempted.' && zero_renewals=1

  # Failure classes (never collapsed into one generic "certbot failed" —
  # each names a different root cause and a different fix):
  #   CERTBOT_REMOTE_TIMEOUT        the remote 300s inner timeout fired;
  #                                 the SSH session itself completed
  #                                 cleanly and reported it.
  #   SSH_TRANSPORT_TIMEOUT         ssh_run_certbot's own local/outer
  #                                 bound fired — no sentinel ever came
  #                                 back, so the remote side's true state
  #                                 is unknown from here.
  #   SSH_SESSION_ENDED_UNEXPECTEDLY  the SSH session ended (any other
  #                                 non-timeout exit) without the
  #                                 sentinel ever appearing — suspicious
  #                                 transport/controller behavior, not
  #                                 proof of anything remote.
  #   NO_RENEWAL_ATTEMPTED          certbot ran and exited, but tested
  #                                 zero lineages — exit 0 here is not
  #                                 renewal proof.
  #   CERTBOT_EXIT_NONZERO          certbot genuinely failed; the
  #                                 sentinel's own rc says so.
  certbot_class=""
  if [ "$sentinel_present" -eq 0 ]; then
    if [ "$certbot_dry_rc" -eq 124 ] || [ "$certbot_dry_rc" -eq 137 ]; then
      certbot_class="SSH_TRANSPORT_TIMEOUT"
    else
      certbot_class="SSH_SESSION_ENDED_UNEXPECTEDLY"
    fi
  elif [ "$sentinel_rc" = "124" ] || [ "$sentinel_rc" = "137" ]; then
    certbot_class="CERTBOT_REMOTE_TIMEOUT"
  elif [ "$zero_renewals" -eq 1 ]; then
    certbot_class="NO_RENEWAL_ATTEMPTED"
  elif [ "$sentinel_rc" != "0" ]; then
    certbot_class="CERTBOT_EXIT_NONZERO"
  fi

  # The sentinel is diagnostic evidence only — never a pass condition by
  # itself (task requirement: "do not simply grep for DONE and mark
  # PASS"). PASS requires ALL of: the sentinel actually present, its own
  # carried rc=0, and at least one lineage genuinely tested.
  if [ -z "$certbot_class" ] && [ "$sentinel_present" -eq 1 ] && [ "$sentinel_rc" = "0" ]; then
    pass "certbot renew --dry-run (at least one renewal was eligible for simulation; remote exit=0, completion sentinel observed)"

    # Post-renewal cleanup-contract check: certbot succeeding is not
    # enough on its own — the pre/post hooks must have actually restored
    # nginx, cleaned up their own /run markers, and left TCP/80 exactly
    # as they found it (never newly open because of THIS run).
    if POST_CERTBOT_STATE="$(ssh_run '
        fails=""
        systemctl is-active --quiet nginx || fails="$fails nginx-not-active"
        compgen -G "/run/singbox-vpn-certbot-*" >/dev/null 2>&1 && fails="$fails stale-hook-markers"
        port80_after="n/a"
        if command -v firewall-cmd >/dev/null 2>&1 && systemctl is-active --quiet firewalld 2>/dev/null; then
          if firewall-cmd --query-port=80/tcp >/dev/null 2>&1; then port80_after=open; else port80_after=closed; fi
        fi
        echo "PORT80_AFTER=$port80_after"
        if [ -n "$fails" ]; then echo "POST_CERTBOT_FAILED:$fails"; exit 1; fi
        echo "POST_CERTBOT_OK"
      ' 2>&1)"; then
      post_certbot_rc=0
    else
      post_certbot_rc=$?
    fi
    # `|| true`: under this script's `set -Eeuo pipefail`, a plain
    # assignment whose RHS pipeline ends without a match (grep -o finds
    # nothing) is NOT exempt from errexit the way an `if`/`&&`-guarded
    # pipeline is — an unexpected/truncated remote response here would
    # otherwise silently kill the entire lifecycle run right at this
    # line, with no [FAIL] ever printed. Reproduced directly while
    # developing this stage. A parse miss just leaves port80_after empty
    # (never mistaken for "open" below), it never suppresses the real
    # pass/fail signal, which comes from $post_certbot_rc (the actual
    # remote exit code), not from this string extraction.
    port80_after="$(printf '%s\n' "$POST_CERTBOT_STATE" | grep -o 'PORT80_AFTER=.*' | cut -d= -f2 || true)"
    port80_regression=0
    if [ "$port80_before" != "open" ] && [ "$port80_after" = "open" ]; then
      port80_regression=1
    fi
    if [ "$post_certbot_rc" -eq 0 ] && [ "$port80_regression" -eq 0 ]; then
      pass "nginx restored, certbot hook markers cleaned, TCP/80 firewall state restored after renewal"
    else
      failed_bits="$(printf '%s\n' "$POST_CERTBOT_STATE" | grep -o 'POST_CERTBOT_FAILED:.*' || true)"
      [ "$port80_regression" -eq 1 ] && failed_bits="$failed_bits tcp80-left-open(before=$port80_before,after=$port80_after)"
      fail_required "post-renewal cleanup contract (nginx/markers/firewall) [POST_RENEWAL_STATE_INVALID]" "(${failed_bits:-see remote state below}; before=$port80_before after=$port80_after; remote state: ${POST_CERTBOT_STATE:-none})"
    fi
  else
    # A killed/timed-out certbot cannot run its OWN
    # renewal-hooks/post/singbox-vpn-firewall.sh (that hook only fires
    # when certbot itself reaches that point normally) — best-effort,
    # defensively re-invoke it directly so a temporarily-stopped nginx or
    # a temporarily-opened TCP/80 firewall rule from this attempt cannot
    # linger indefinitely just because Stage 18 timed out. Bounded by
    # ssh_run's own timeout; safe even when the post-hook already ran
    # normally (test-certbot-firewall-hooks.sh's "Test I" proves a second
    # invocation is a no-op — its own marker files make it idempotent).
    if [ "$certbot_class" = "CERTBOT_REMOTE_TIMEOUT" ] || [ "$certbot_class" = "SSH_TRANSPORT_TIMEOUT" ] || [ "$certbot_class" = "SSH_SESSION_ENDED_UNEXPECTEDLY" ]; then
      ssh_run 'sudo /etc/letsencrypt/renewal-hooks/post/singbox-vpn-firewall.sh' >/dev/null 2>&1 || true
    fi
    # Diagnostics only — never the raw /var/log/letsencrypt/letsencrypt.log
    # (noisy, not useful for automated triage) and never anything from
    # /etc/vpn or the environment. `certbot certificates` prints lineage
    # names/paths/expiry (no key material); `ss`/`firewall-cmd` report
    # listening sockets/configured ports, not secrets.
    CERTBOT_DIAG=""
    if [ "$certbot_class" != "SSH_TRANSPORT_TIMEOUT" ]; then
      CERTBOT_DIAG="$(ssh_run '
        echo "--- certbot certificates ---"
        sudo certbot certificates 2>&1
        echo "--- nginx status ---"
        systemctl status nginx --no-pager -l --lines=20 2>&1
        echo "--- listeners on 80/443 ---"
        ss -lntp 2>&1 | grep -E ":80 |:443 " || true
        echo "--- firewalld configured ports ---"
        sudo firewall-cmd --list-ports 2>&1 || true
      ' 2>&1 || true)"
    fi
    concise_out="$(printf '%s\n' "$CERTBOT_DRY_OUT" | grep -E 'ERROR|WARNING|Failed|failure|timeout|challenge|renew|Congratulations|simulated' || true)"
    [ -z "$concise_out" ] && concise_out="$(printf '%s\n' "$CERTBOT_DRY_OUT" | tail -20)"
    fail_required "certbot renew --dry-run [${certbot_class:-UNKNOWN}]" "(exit=$certbot_dry_rc; sentinel=${sentinel_present}/${sentinel_rc:-none}; output: ${concise_out:-none}; diagnostics: ${CERTBOT_DIAG:-none})"
  fi
  else
    fail_required "certbot renew --dry-run [CERT_LINEAGE_LOOKUP_FAILED]" "(could not determine this deployment's own certificate lineage from /etc/vpn/deployment.toml + certbot certificates — refusing to run a global 'certbot renew' that could pass or fail based on unrelated lineages on this host; lookup output: ${cert_lookup_out:-none})"
  fi
else
  section "9-18. runtime/protocol/user/update/backup/renewal checks"
  block "stages 9-18" "(stage 8 did not establish a working baseline; dependent failures are intentionally not counted as separate bugs)"
fi

section "19. uninstall completely (offline singbox-vpn-uninstall)"
# Block github.com/raw.githubusercontent.com's CURRENT addresses to prove
# singbox-vpn-uninstall needs no network access at all. `iptables -I/-D -d
# <hostname>` each perform their OWN independent DNS lookup at the moment
# they run — both hostnames are round-robin DNS names with more than one
# A record, so the lookup done for the matching -D below can resolve to a
# DIFFERENT address than the one this -I used. When that happens, -D
# matches nothing (iptables matches by IP, not name) and silently leaves
# this -I rule in place forever, with every error here suppressed.
# Reproduced for real: an orphaned REJECT rule for a single github.com IP
# survived across separate lifecycle-acceptance.sh runs on a host with no
# other iptables rules of its own, and broke install.sh's GitHub
# downloads in a LATER, unrelated run in a way that looked exactly like a
# flaky network blip. Resolve once into OFFLINE_BLOCK_REMOTE_FILE and
# have the matching -D below (and the interrupted-run safety net in
# cleanup_lifecycle_tmp) remove only those exact recorded IPs — never
# re-resolve for the delete.
ssh_run "getent ahosts github.com raw.githubusercontent.com 2>/dev/null | cut -d' ' -f1 | sort -u > $OFFLINE_BLOCK_REMOTE_FILE; while read -r ip; do [ -n \"\$ip\" ] && sudo iptables -I OUTPUT -d \"\$ip\" -j REJECT 2>/dev/null; done < $OFFLINE_BLOCK_REMOTE_FILE" >/dev/null 2>&1 || true
OFFLINE_BLOCK_ACTIVE=1
if ssh_run 'test -x /opt/singbox-vpn/bin/singbox-vpn-uninstall' 2>/dev/null; then
  if run_uninstall_classified "singbox-vpn-uninstall --yes (offline, local binary only)"; then
    # Stage 19 PASS requires more than a zero exit code: verify the
    # uninstaller actually removed what it claims to, right now, before
    # the reinstall below creates fresh state that would otherwise mask
    # a real leak — stage 27 checks this again after the FINAL uninstall,
    # this is the same check at the first one.
    check_residue_vs_baseline \
      "offline uninstall left no NEW singbox-vpn-owned residue vs. pre-install host baseline" \
      "offline uninstall [UNINSTALL_POST_STATE_INVALID] left new singbox-vpn-owned residue beyond pre-install host baseline" || true
  fi
elif ssh_run '[ ! -e /etc/vpn ] && [ ! -e /opt/singbox-vpn ] && [ ! -e /var/lib/singbox-vpn ]' 2>/dev/null; then
  pass "target already clean (no installed uninstaller needed)"
else
  fail_required "offline uninstall available for partial state [UNINSTALL_MISSING_BINARY]" "(partial singbox-vpn state exists but local uninstaller is missing)"
fi
ssh_run "while read -r ip; do [ -n \"\$ip\" ] && sudo iptables -D OUTPUT -d \"\$ip\" -j REJECT 2>/dev/null; done < $OFFLINE_BLOCK_REMOTE_FILE 2>/dev/null; rm -f $OFFLINE_BLOCK_REMOTE_FILE" >/dev/null 2>&1 || true
OFFLINE_BLOCK_ACTIVE=0

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
  if ssh_run "sudo test -s $BACKUP_PATH" 2>/dev/null && ssh_run "sudo /usr/local/bin/vpn-admin restore $BACKUP_PATH" 2>/dev/null; then pass "vpn-admin restore applied the backup archive"; else fail_required "vpn-admin restore applied the backup archive"; fi

  section "23. verify restored user/key state works"
  POST_RESTORE_USERLIST="$(ssh_run 'sudo /usr/local/bin/vpn-admin user list' 2>/dev/null || true)"
  if [ -n "$PRE_BACKUP_USERLIST" ] && [ "$PRE_BACKUP_USERLIST" = "$POST_RESTORE_USERLIST" ]; then pass "restored user list matches the pre-backup snapshot exactly (ids/names/enabled state, including $TEST_USER_NAME)"; else fail_required "restored user list matches the pre-backup snapshot" "(pre-backup: $PRE_BACKUP_USERLIST | post-restore: $POST_RESTORE_USERLIST)"; fi
  if ssh_run "sudo /usr/local/bin/vpn-admin user list | grep -q $TEST_USER_NAME" 2>/dev/null; then pass "the persisted test user ($TEST_USER_NAME) survived uninstall/reinstall/restore"; else fail_required "the persisted test user ($TEST_USER_NAME) survived uninstall/reinstall/restore"; fi

  section "24. doctor/protocol checks again (post-restore)"
  if POST_RESTORE_PROTOCOL_OUT="$(ssh_run 'sudo /usr/local/bin/vpn-admin doctor --protocol --require-protocol' 2>&1)"; then post_restore_rc=0; else post_restore_rc=$?; fi
  if [ "$post_restore_rc" -eq 0 ] && printf '%s' "$POST_RESTORE_PROTOCOL_OUT" | grep -q 'completed a full handshake'; then pass "REALITY handshake self-test PASSES against the restored key material"; else fail_required "REALITY handshake self-test against restored key material" "(exit=$post_restore_rc; see remote output above)"; fi
else
  section "22-24. backup restore/state/protocol checks"
  block "stages 22-24" "(requires both a successful backup and stage-21 reinstall; dependent failures are not counted separately)"
fi

section "25. final uninstall (offline singbox-vpn-uninstall)"
ssh_run "sudo rm -f $BACKUP_PATH" >/dev/null 2>&1 || true
if ssh_run 'test -x /opt/singbox-vpn/bin/singbox-vpn-uninstall' 2>/dev/null; then
  run_uninstall_classified "final singbox-vpn-uninstall --yes (offline, local binary only)" || true
elif ssh_run '[ ! -e /etc/vpn ] && [ ! -e /opt/singbox-vpn ] && [ ! -e /var/lib/singbox-vpn ]' 2>/dev/null; then
  pass "target already clean at final uninstall"
else
  fail_required "final offline uninstall available for partial state [UNINSTALL_MISSING_BINARY]"
fi
cleanup_cert_snapshot

section "26. SSH after final uninstall (new connection, port $SSH_PORT)"
if ssh_reconnect 'systemctl is-active --quiet sshd' 2>/dev/null; then pass "SSH still active post-uninstall"; else fail_required "SSH still active post-uninstall"; fi

section "27. final uninstall residue audit (vs. host baseline from stage 1b) — singbox-vpn-owned service/config/state/firewall/sysctl residue"
# Field-by-field, not exact-string-equality: the "baseline" captured at
# stage 1b can itself already be dirty — e.g. a target this harness
# authorized destroying via --allow-destroy-existing-singbox-vpn-install
# (stage 0a) legitimately has singbox-vpn state BEFORE this run's own
# clean install ever happens, so the true baseline reflects that. If this
# run's uninstall cleans a field to LESS than that dirty baseline (e.g.
# var_lib_singbox-vpn 1 -> 0), that is strictly better than baseline, not
# residue — exact-equality would wrongly fail a run that left the host
# cleaner than it found it. Only flag a field that is HIGHER after this
# run than it was at baseline: something this run's uninstall left behind
# that baseline did not already have. (check_residue_vs_baseline() above
# — shared with stage 19's own post-uninstall check.)
check_residue_vs_baseline \
  "no NEW singbox-vpn-owned runtime/config/state residue vs. pre-install host baseline" \
  "new singbox-vpn-owned residue introduced beyond pre-install host baseline" || true

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
