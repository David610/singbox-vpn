#!/usr/bin/env bash
# Regression coverage for the certbot pre/post renewal firewall hooks
# (docs/FINAL_PRODUCTION_AUDIT.md F-06): every renewal attempt for either
# certificate lineage uses HTTP-01, so TCP/80 must be reachable again for
# EACH renewal, not just the initial install. These hooks reopen it
# around each attempt and close it again afterward — this exercises both
# supported backends (firewalld/ufw), the "already open" no-op case (must
# never remove a rule it did not add itself), and the no-managed-backend
# case, entirely against fake firewall-cmd/ufw/systemctl binaries (no
# real firewalld/ufw/systemd needed).
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
PRE="$ROOT/deploy/almalinux/certbot-firewall-pre-hook.sh"
POST="$ROOT/deploy/almalinux/certbot-firewall-post-hook.sh"
bash -n "$PRE"
bash -n "$POST"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# --- fakes --------------------------------------------------------------
# firewall-cmd fake: tracks open/closed state for port 80 in a state file
# under $1 (the fake's own directory), so successive pre/post-hook
# invocations in the same test see a consistent picture, just like a real
# firewalld daemon would.
make_firewalld_fakes() {
  local dir=$1 initially_open=$2
  mkdir -p "$dir/bin"
  [ "$initially_open" -eq 1 ] && touch "$dir/port80-open" || rm -f "$dir/port80-open"
  cat > "$dir/bin/firewall-cmd" <<EOF
#!/usr/bin/env bash
case "\$1" in
  --query-port=80/tcp) [ -e "$dir/port80-open" ] && exit 0 || exit 1 ;;
  --add-port=80/tcp) touch "$dir/port80-open"; exit 0 ;;
  --remove-port=80/tcp) rm -f "$dir/port80-open"; exit 0 ;;
esac
exit 1
EOF
  chmod +x "$dir/bin/firewall-cmd"
  cat > "$dir/bin/systemctl" <<'EOF'
#!/usr/bin/env bash
case "$1 $2" in
  "is-active --quiet") exit 0 ;;
esac
[ "$1" = "is-active" ] && exit 0
exit 0
EOF
  chmod +x "$dir/bin/systemctl"
}


make_ufw_fake() {
  local dir=$1 initially_open=$2
  mkdir -p "$dir/bin"
  [ "$initially_open" -eq 1 ] && touch "$dir/port80-open" || rm -f "$dir/port80-open"
  cat > "$dir/bin/ufw" <<EOF
#!/usr/bin/env bash
if [ "\$1" = "status" ]; then
  echo "Status: active"
  [ -e "$dir/port80-open" ] && echo "80/tcp                     ALLOW       Anywhere"
  exit 0
fi
if [ "\$1" = "allow" ] && [ "\$2" = "80/tcp" ]; then touch "$dir/port80-open"; exit 0; fi
if [ "\$1" = "delete" ] && [ "\$2" = "allow" ] && [ "\$3" = "80/tcp" ]; then rm -f "$dir/port80-open"; exit 0; fi
exit 1
EOF
  chmod +x "$dir/bin/ufw"
}

run_pre() {
  local dir=$1
  env -i PATH="$dir/bin:/usr/bin:/bin" \
    SINGBOX_VPN_CERTBOT_PORT80_MARKER="$dir/marker" \
    SINGBOX_VPN_CERTBOT_NGINX_MARKER="$dir/nginx-marker" \
    bash "$PRE"
}
run_post() {
  local dir=$1
  env -i PATH="$dir/bin:/usr/bin:/bin" \
    SINGBOX_VPN_CERTBOT_PORT80_MARKER="$dir/marker" \
    SINGBOX_VPN_CERTBOT_NGINX_MARKER="$dir/nginx-marker" \
    bash "$POST"
}

# --- Test A: firewalld, port closed — pre opens it, post closes it -----
dir="$WORK/a"; make_firewalld_fakes "$dir" 0
run_pre "$dir" | grep -q "temporarily allowed"
[ -e "$dir/port80-open" ] || { echo "Test A FAILED: pre-hook did not open port 80"; exit 1; }
[ -e "$dir/marker" ] || { echo "Test A FAILED: pre-hook did not record a marker"; exit 1; }
run_post "$dir" | grep -q "removed the temporary"
[ ! -e "$dir/port80-open" ] || { echo "Test A FAILED: post-hook did not close port 80"; exit 1; }
[ ! -e "$dir/marker" ] || { echo "Test A FAILED: post-hook did not clean up its marker"; exit 1; }
echo "Test A (firewalld, closed -> opened -> closed): PASS"

# --- Test B: firewalld, port already open (e.g. an operator's own
# permanent rule) — pre-hook must be a no-op, post-hook must NOT touch it
dir="$WORK/b"; make_firewalld_fakes "$dir" 1
run_pre "$dir" | grep -q "already allowed"
[ ! -e "$dir/marker" ] || { echo "Test B FAILED: pre-hook wrote a marker for a rule it did not add"; exit 1; }
run_post "$dir"
[ -e "$dir/port80-open" ] || { echo "Test B FAILED: post-hook removed a rule it never added"; exit 1; }
echo "Test B (firewalld, already open -> left untouched): PASS"

# --- Test C: ufw, port closed — pre opens it, post closes it -----------
dir="$WORK/c"; make_ufw_fake "$dir" 0
run_pre "$dir" | grep -q "temporarily allowed"
[ -e "$dir/port80-open" ] || { echo "Test C FAILED: pre-hook did not open port 80 (ufw)"; exit 1; }
run_post "$dir" | grep -q "removed the temporary"
[ ! -e "$dir/port80-open" ] || { echo "Test C FAILED: post-hook did not close port 80 (ufw)"; exit 1; }
echo "Test C (ufw, closed -> opened -> closed): PASS"

# --- Test D: no managed firewall backend — both hooks are no-ops, exit 0
dir="$WORK/d"; mkdir -p "$dir/bin"
run_pre "$dir" | grep -q "no managed firewalld/ufw backend"
[ ! -e "$dir/marker" ] || { echo "Test D FAILED: pre-hook wrote a marker with no firewall backend"; exit 1; }
run_post "$dir"
echo "Test D (no managed firewall backend -> safe no-op): PASS"

# Combined firewalld+nginx fake for tests E/F: reproduced directly on a
# real VPS — `certbot renew --dry-run` failed with "Could not bind TCP
# port 80" even with the firewall correctly opened, because nginx (which
# has no role in the ACME challenge — see certbot-firewall-pre-hook.sh)
# was still bound to :80 via its OS-default vhost. A single systemctl
# fake has to answer both firewalld's and nginx's is-active checks (the
# per-scenario fakes above each only answer one), and track nginx's
# stop/start state for real, unlike the blanket "always active" fake used
# by tests A-D (which never actually exercises the stop branch).
make_firewalld_and_nginx_fakes() {
  local dir=$1 firewall_initially_open=$2 nginx_initially_active=$3
  mkdir -p "$dir/bin"
  [ "$firewall_initially_open" -eq 1 ] && touch "$dir/port80-open" || rm -f "$dir/port80-open"
  [ "$nginx_initially_active" -eq 1 ] && echo active > "$dir/nginx-state" || echo inactive > "$dir/nginx-state"
  cat > "$dir/bin/nginx" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
  chmod +x "$dir/bin/nginx"
  cat > "$dir/bin/firewall-cmd" <<EOF
#!/usr/bin/env bash
case "\$1" in
  --query-port=80/tcp) [ -e "$dir/port80-open" ] && exit 0 || exit 1 ;;
  --add-port=80/tcp) touch "$dir/port80-open"; exit 0 ;;
  --remove-port=80/tcp) rm -f "$dir/port80-open"; exit 0 ;;
esac
exit 1
EOF
  chmod +x "$dir/bin/firewall-cmd"
  cat > "$dir/bin/systemctl" <<EOF
#!/usr/bin/env bash
case "\$1 \$2 \$3" in
  "is-active --quiet firewalld") exit 0 ;;
  "is-active --quiet nginx") [ "\$(cat "$dir/nginx-state" 2>/dev/null)" = "active" ] && exit 0 || exit 1 ;;
esac
case "\$1 \$2" in
  "stop nginx") echo inactive > "$dir/nginx-state"; exit 0 ;;
  "start nginx") echo active > "$dir/nginx-state"; exit 0 ;;
esac
exit 0
EOF
  chmod +x "$dir/bin/systemctl"
}

# --- Test E: nginx active + firewall closed — pre-hook stops nginx AND
# opens the port; post-hook restarts nginx AND closes the port again ----
dir="$WORK/e"; make_firewalld_and_nginx_fakes "$dir" 0 1
pre_out="$(run_pre "$dir")"
printf '%s' "$pre_out" | grep -q "temporarily stopped nginx" || { echo "Test E FAILED: pre-hook did not report stopping nginx: $pre_out"; exit 1; }
printf '%s' "$pre_out" | grep -q "temporarily allowed" || { echo "Test E FAILED: pre-hook did not also open the firewall port"; exit 1; }
[ "$(cat "$dir/nginx-state")" = "inactive" ] || { echo "Test E FAILED: nginx was not actually stopped"; exit 1; }
[ -e "$dir/nginx-marker" ] || { echo "Test E FAILED: pre-hook did not record an nginx marker"; exit 1; }
post_out="$(run_post "$dir")"
printf '%s' "$post_out" | grep -q "restarted nginx" || { echo "Test E FAILED: post-hook did not report restarting nginx: $post_out"; exit 1; }
[ "$(cat "$dir/nginx-state")" = "active" ] || { echo "Test E FAILED: nginx was not actually restarted"; exit 1; }
[ ! -e "$dir/port80-open" ] || { echo "Test E FAILED: post-hook did not close the firewall port"; exit 1; }
[ ! -e "$dir/nginx-marker" ] || { echo "Test E FAILED: post-hook did not clean up its nginx marker"; exit 1; }
echo "Test E (nginx active -> stopped for the challenge -> restarted): PASS"

# --- Test F: nginx already inactive — pre-hook must not touch it, and
# must never claim to have stopped something that wasn't running --------
dir="$WORK/f"; make_firewalld_and_nginx_fakes "$dir" 0 0
pre_out="$(run_pre "$dir")"
printf '%s' "$pre_out" | grep -qv "temporarily stopped nginx" 2>/dev/null
if printf '%s' "$pre_out" | grep -q "temporarily stopped nginx"; then
  echo "Test F FAILED: pre-hook claimed to stop nginx when it was already inactive"; exit 1
fi
[ ! -e "$dir/nginx-marker" ] || { echo "Test F FAILED: pre-hook wrote an nginx marker when nginx was already inactive"; exit 1; }
run_post "$dir"
[ "$(cat "$dir/nginx-state")" = "inactive" ] || { echo "Test F FAILED: post-hook started nginx even though the pre-hook never stopped it"; exit 1; }
echo "Test F (nginx already inactive -> left untouched): PASS"

echo "certbot-firewall-hooks tests: PASS"
