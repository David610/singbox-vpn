#!/usr/bin/env bash
# Node role/identity across the shell lifecycle (fresh install, repair,
# update, rollback): deploy/lib/node-identity.sh, the deployment.toml
# templates, install.sh's resolve_node_identity()/render_deployment_toml()
# and update.sh's identity snapshot/guard/rollback. Fixture-only: never
# touches the host. The Rust side of the same rules (and a load of the
# rendered templates through DeploymentConfig) is
# crates/compat-config/tests/relay_role_policy.rs.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
INSTALL_SH="$REPO_ROOT/deploy/almalinux/install.sh"
UPDATE_SH="$REPO_ROOT/deploy/almalinux/update.sh"
TEMPLATE="$REPO_ROOT/deploy/almalinux/templates/deployment.toml.template"
RELAY_TEMPLATE="$REPO_ROOT/deploy/almalinux/templates/relay-ingress.toml.template"
DEPLOYMENT_RS="$REPO_ROOT/crates/compat-config/src/deployment.rs"
CASES="$REPO_ROOT/deploy/lib/tests/fixtures/node-id-cases.tsv"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/node-identity.sh"

echo "--- static: fresh-install template writes the CURRENT schema with explicit identity ---"
rust_version="$(sed -n 's/^pub const DEPLOYMENT_SCHEMA_VERSION: u32 = \([0-9][0-9]*\);$/\1/p' "$DEPLOYMENT_RS")"
template_version="$(deployment_top_level_value "$TEMPLATE" schema_version)"
if [ -n "$rust_version" ] && [ "$template_version" = "$rust_version" ]; then
  ok "template schema_version ($template_version) == DEPLOYMENT_SCHEMA_VERSION ($rust_version)"
else
  fail "template schema_version '$template_version' != DEPLOYMENT_SCHEMA_VERSION '$rust_version' — fresh installs would not start current"
fi
[ "$(deployment_top_level_value "$TEMPLATE" node_id)" = "{{NODE_ID}}" ] \
  && ok "template carries a top-level node_id placeholder" \
  || fail "template lacks a top-level node_id = \"{{NODE_ID}}\""
[ "$(deployment_top_level_value "$TEMPLATE" role)" = "{{NODE_ROLE}}" ] \
  && ok "template carries a top-level role placeholder" \
  || fail "template lacks a top-level role = \"{{NODE_ROLE}}\""
if grep -q '^via_endpoint_id = "reality-1"$' "$RELAY_TEMPLATE" && grep -q '^kind = "relay"$' "$RELAY_TEMPLATE" \
    && ! grep -qE '^capabilities = .*udp' "$RELAY_TEMPLATE"; then
  ok "relay ingress template declares reality-1 as a TCP-only relay first hop"
else
  fail "relay ingress template does not declare reality-1 as a TCP-only relay first hop"
fi

echo
echo "--- functional: default node_id rule == shared fixture table (also read by the Rust test) ---"
cases=0
while IFS= read -r line; do
  case "$line" in \#*|'') continue ;; esac
  host="$(printf '%s' "$line" | awk -F'\t' '{print $1}')"
  expected="$(printf '%s' "$line" | awk -F'\t' '{print $2}')"
  got="$(default_node_id_for_host "$host")"
  cases=$((cases + 1))
  if [ "$got" = "$expected" ] && node_id_is_valid "$got"; then
    ok "default_node_id_for_host '$host' -> '$got'"
  else
    fail "default_node_id_for_host '$host' -> '$got' (expected '$expected')"
  fi
done <"$CASES"
[ "$cases" -ge 10 ] && ok "$cases fixture cases checked" || fail "only $cases fixture cases found"

echo
echo "--- functional: strict role and node_id validators ---"
for role in exit relay; do
  node_role_is_valid "$role" && ok "role '$role' accepted" || fail "role '$role' refused"
done
for role in "" Exit RELAY entry "exit " both; do
  if node_role_is_valid "$role"; then fail "invalid role '$role' accepted"; else ok "invalid role '$role' refused"; fi
done
for id in de1 ru-1 a.b_c "$(printf 'a%.0s' $(seq 63))"; do
  node_id_is_valid "$id" && ok "node_id '${id:0:20}' accepted" || fail "node_id '$id' refused"
done
for id in "" "-de1" ".x" "de 1" "de/1" "$(printf 'a%.0s' $(seq 64))"; do
  if node_id_is_valid "$id"; then fail "invalid node_id '${id:0:20}' accepted"; else ok "invalid node_id '${id:0:20}' refused"; fi
done

echo
echo "--- functional: top-level key reader ignores tables and comments ---"
cat >"$WORK/scoped.toml" <<'EOF'
schema_version = 2
# role = "relay"
node_id = "de1"   # trailing comment
public_host = "de1.example.test"

[reality]
role = "relay"
EOF
[ "$(deployment_effective_role "$WORK/scoped.toml")" = "exit" ] \
  && ok "a role key inside a table or comment is not the node role" \
  || fail "deployment_effective_role read a non-top-level role"
[ "$(deployment_top_level_value "$WORK/scoped.toml" node_id)" = "de1" ] \
  && ok "node_id read without quotes or trailing comment" \
  || fail "node_id not read correctly"

echo
echo "--- functional: identity guard detects role/node changes, tolerates a newly stamped node_id ---"
printf 'public_host = "x"\n' >"$WORK/legacy.toml"
cp "$WORK/legacy.toml" "$WORK/guard.toml"
printf 'schema_version = 2\nnode_id = "x"\nrole = "exit"\npublic_host = "x"\n' >"$WORK/guard.toml"
deployment_identity_unchanged "$WORK/guard.toml" exit "" >/dev/null \
  && ok "legacy exit migrated to explicit exit + new node_id is unchanged identity" \
  || fail "guard rejected a legitimate migration"
printf 'schema_version = 2\nnode_id = "x"\nrole = "relay"\npublic_host = "x"\n' >"$WORK/guard.toml"
if deployment_identity_unchanged "$WORK/guard.toml" exit "" >/dev/null; then fail "exit -> relay not detected"; else ok "exit -> relay detected"; fi
printf 'schema_version = 2\nnode_id = "x"\nrole = "exit"\npublic_host = "x"\n' >"$WORK/guard.toml"
if deployment_identity_unchanged "$WORK/guard.toml" relay "x" >/dev/null; then fail "relay -> exit not detected"; else ok "relay -> exit detected"; fi
if deployment_identity_unchanged "$WORK/guard.toml" exit "y" >/dev/null; then fail "node rename not detected"; else ok "node rename detected"; fi

# Runs a snippet with install.sh's real functions sourced, die() replaced by
# a plain exit so no rollback/uninstall path can ever run from a test.
with_install_functions() {
  (
    set +e
    # shellcheck source=/dev/null
    . "$INSTALL_SH"
    trap - ERR INT TERM
    set +e
    die() { echo "DIE: $*"; exit 1; }
    log() { echo "LOG: $*"; }
    eval "$1"
  )
}

echo
echo "--- functional: install.sh flag parsing refuses a bad role/node-id before any mutation ---"
out="$(with_install_functions 'parse_cli_args --role gateway; echo PARSED' 2>&1 || true)"
echo "$out" | grep -q "DIE: invalid --role 'gateway'" && ! echo "$out" | grep -q PARSED \
  && ok "--role gateway refused" || fail "--role gateway not refused: $out"
out="$(with_install_functions 'parse_cli_args --node-id "-bad"; echo PARSED' 2>&1 || true)"
echo "$out" | grep -q "DIE: invalid --node-id" && ok "--node-id -bad refused" || fail "--node-id -bad not refused: $out"
out="$(with_install_functions 'parse_cli_args --role=relay --node-id=ru1; echo "role=$NODE_ROLE node=$NODE_ID"' 2>&1)"
echo "$out" | grep -q "role=relay node=ru1" && ok "--role=relay --node-id=ru1 parsed" || fail "flags not parsed: $out"

echo
echo "--- functional: fresh install renders exit/relay deployment.toml with identity ---"
render_fixture() {
  local role="$1" node="$2" dest="$3"
  with_install_functions "
    DEPLOYMENT_TOML='$dest'
    PUBLIC_HOST=de1.example.test SUBSCRIPTION_HOST=de1.example.test SUBSCRIPTION_PORT=8443
    REALITY_HANDSHAKE_SERVER=www.example.com
    NODE_ROLE='$role' NODE_ID='$node'
    resolve_node_identity
    render_deployment_toml
  " >/dev/null 2>&1
}
render_fixture "" "" "$WORK/fresh-default.toml" || true
if [ "$(deployment_top_level_value "$WORK/fresh-default.toml" schema_version)" = "$rust_version" ] \
    && [ "$(deployment_top_level_value "$WORK/fresh-default.toml" role)" = "exit" ] \
    && [ "$(deployment_top_level_value "$WORK/fresh-default.toml" node_id)" = "de1" ] \
    && ! grep -q '^\[\[access_paths\]\]' "$WORK/fresh-default.toml" \
    && ! grep -q '{{' "$WORK/fresh-default.toml"; then
  ok "default fresh install is a current-schema exit with node_id derived from the host"
else
  fail "default fresh install render is wrong: $(cat "$WORK/fresh-default.toml" 2>/dev/null)"
fi
render_fixture relay ru1 "$WORK/fresh-relay.toml" || true
if [ "$(deployment_top_level_value "$WORK/fresh-relay.toml" role)" = "relay" ] \
    && [ "$(deployment_top_level_value "$WORK/fresh-relay.toml" node_id)" = "ru1" ] \
    && grep -q '^via_endpoint_id = "reality-1"$' "$WORK/fresh-relay.toml" \
    && ! grep -q '^\[\[peer_endpoints\]\]' "$WORK/fresh-relay.toml"; then
  ok "--role relay renders an unpaired relay with its ingress declaration"
else
  fail "relay fresh install render is wrong: $(cat "$WORK/fresh-relay.toml" 2>/dev/null)"
fi

echo
echo "--- functional: repair keeps the existing role/identity and refuses conflicting flags ---"
cp "$WORK/fresh-relay.toml" "$WORK/existing.toml"
out="$(with_install_functions "DEPLOYMENT_TOML='$WORK/existing.toml'; PUBLIC_HOST=ru1.example.test; NODE_ROLE=''; NODE_ID=''; resolve_node_identity; echo \"role=\$NODE_ROLE node=\$NODE_ID\"" 2>&1)"
echo "$out" | grep -q "role=relay node=ru1" && ok "repair without flags keeps role=relay node_id=ru1" || fail "repair lost identity: $out"
out="$(with_install_functions "DEPLOYMENT_TOML='$WORK/existing.toml'; PUBLIC_HOST=ru1.example.test; NODE_ROLE=exit; NODE_ID=''; resolve_node_identity; echo CONTINUED" 2>&1 || true)"
echo "$out" | grep -q "DIE: this deployment is already installed with role 'relay'" && ! echo "$out" | grep -q CONTINUED \
  && ok "repair with --role exit on a relay is refused" || fail "relay -> exit conversion not refused: $out"
out="$(with_install_functions "DEPLOYMENT_TOML='$WORK/existing.toml'; PUBLIC_HOST=ru1.example.test; NODE_ROLE=''; NODE_ID=ru2; resolve_node_identity; echo CONTINUED" 2>&1 || true)"
echo "$out" | grep -q "would rename a live node" && ! echo "$out" | grep -q CONTINUED \
  && ok "repair with a different --node-id is refused" || fail "node rename not refused: $out"
cp "$WORK/legacy.toml" "$WORK/existing-legacy.toml"
out="$(with_install_functions "DEPLOYMENT_TOML='$WORK/existing-legacy.toml'; PUBLIC_HOST=vpn.example.test; NODE_ROLE=''; NODE_ID=''; resolve_node_identity; echo \"role=\$NODE_ROLE node=\$NODE_ID\"" 2>&1)"
echo "$out" | grep -q "role=exit node=vpn" && ok "legacy deployment without role/node_id repairs as an exit with the documented node_id" \
  || fail "legacy repair identity wrong: $out"
[ "$(cat "$WORK/existing.toml")" = "$(cat "$WORK/fresh-relay.toml")" ] \
  && ok "resolve_node_identity never writes deployment.toml" || fail "resolve_node_identity modified an existing deployment.toml"

echo
echo "--- static: install.sh wiring ---"
preflight_body="$(sed -n '/^preflight_stage() {/,/^}/p' "$INSTALL_SH")"
if echo "$preflight_body" | grep -q '^\s*resolve_host_config$' && echo "$preflight_body" | grep -q '^\s*resolve_node_identity$'; then
  host_line="$(echo "$preflight_body" | grep -n '^\s*resolve_host_config$' | cut -d: -f1)"
  node_line="$(echo "$preflight_body" | grep -n '^\s*resolve_node_identity$' | cut -d: -f1)"
  manifest_line="$(echo "$preflight_body" | grep -n 'write_install_state_manifest "installing"' | cut -d: -f1)"
  [ "$host_line" -lt "$node_line" ] && [ "$node_line" -lt "$manifest_line" ] \
    && ok "identity is resolved in preflight after the host and before the first manifest write" \
    || fail "resolve_node_identity is mis-ordered in preflight_stage"
else
  fail "preflight_stage does not resolve node identity"
fi
acceptance_body="$(sed -n '/^acceptance_stage() {/,/^}/p' "$INSTALL_SH")"
echo "$acceptance_body" | grep -q 'verify_relay_subscription_fail_closed_through_nginx' \
  && ok "acceptance uses the relay fail-closed subscription check for relays" \
  || fail "acceptance_stage has no relay-specific subscription check"
for key in node_id role; do
  sed -n '/^write_install_state_manifest() {/,/^}/p' "$INSTALL_SH" | grep -q "\"$key\"" \
    && ok "install manifest records $key" || fail "install manifest lacks $key"
  grep -q "\"$key\": \"\$(deployment_" "$UPDATE_SH" \
    && ok "update manifest records $key from deployment.toml" || fail "update manifest lacks $key"
done

echo
echo "--- static: update.sh snapshots identity, guards migration, restores deployment.toml on rollback (both paths) ---"
[ "$(grep -c '^\s*snapshot_deployment_identity$' "$UPDATE_SH")" -eq 2 ] \
  && ok "identity snapshot taken in both the production and dev-rebuild paths" \
  || fail "snapshot_deployment_identity is not called exactly twice"
[ "$(grep -c '^\s*assert_deployment_identity_unchanged$' "$UPDATE_SH")" -eq 2 ] \
  && ok "identity guard runs after migration in both paths" \
  || fail "assert_deployment_identity_unchanged is not called exactly twice"
[ "$(grep -c 'restore_deployment_toml_snapshot || failed=1' "$UPDATE_SH")" -eq 2 ] \
  && ok "both rollback functions restore the pre-update deployment.toml" \
  || fail "rollback does not restore deployment.toml in both paths"
prod_body="$(sed -n '/^exec 201>\/run\/lock\/singbox-vpn.lock$/,/^mutation_started=1$/p' "$UPDATE_SH")"
echo "$prod_body" | grep -q '^snapshot_deployment_identity$' \
  && ok "production snapshot happens under the state lock, before mutation starts" \
  || fail "production snapshot is not between the lock and mutation_started=1"

echo
echo "--- static: a relay never switches to a vpn-admin without fail-closed relay enforcement ---"
rust_marker="$(sed -n 's/^pub const RELAY_ENFORCEMENT_CAPABILITY: &str = "\(.*\)";$/\1/p' "$DEPLOYMENT_RS")"
[ -n "$rust_marker" ] && [ "$rust_marker" = "$RELAY_ENFORCEMENT_CAPABILITY" ] \
  && ok "shell and Rust agree on the relay enforcement capability marker" \
  || fail "RELAY_ENFORCEMENT_CAPABILITY differs: rust='$rust_marker' shell='$RELAY_ENFORCEMENT_CAPABILITY'"
precheck_body="$(sed -n '/^if \[ -f "\$DEPLOYMENT_TOML" \]; then$/,/^fi$/p' "$UPDATE_SH")"
echo "$precheck_body" | grep -q 'admin_output_declares_relay_enforcement "\$precheck_output"' \
  && echo "$precheck_body" | grep -q 'die "this node is a RELAY' \
  && ok "update.sh's pre-switch check refuses a relay target without relay enforcement" \
  || fail "update.sh does not guard relay nodes against a vpn-admin without relay enforcement"
admin_output_declares_relay_enforcement "deployment.toml: CURRENT
$RELAY_ENFORCEMENT_CAPABILITY (node role: relay)" \
  && ok "marker detected in current vpn-admin output" || fail "marker not detected"
if admin_output_declares_relay_enforcement "deployment.toml (x): CURRENT (schema_version 2)
MODE: OK"; then fail "output of a build without relay enforcement was accepted"; else ok "output without the marker is refused"; fi

echo
echo "--- functional: update.sh's real restore_deployment_toml_snapshot() rewinds only a changed file ---"
eval "$(sed -n '/^restore_deployment_toml_snapshot() {/,/^}/p' "$UPDATE_SH")"
BACKUP_DIR="$WORK/backup"; mkdir -p "$BACKUP_DIR"
DEPLOYMENT_TOML="$WORK/live.toml"
printf 'schema_version = 1\nrole = "relay"\n' >"$BACKUP_DIR/deployment.toml"
printf 'schema_version = 2\nnode_id = "ru1"\nrole = "relay"\n' >"$DEPLOYMENT_TOML"
restore_deployment_toml_snapshot
cmp -s "$BACKUP_DIR/deployment.toml" "$DEPLOYMENT_TOML" \
  && ok "a migrated deployment.toml is rewound to the exact pre-update bytes" \
  || fail "rollback did not rewind deployment.toml"
rm -f "$BACKUP_DIR/deployment.toml"
restore_deployment_toml_snapshot && ok "no snapshot (nothing taken) is a no-op" || fail "missing snapshot failed rollback"

echo
if [ "$failures" -ne 0 ]; then
  echo "$failures node identity test(s) failed"
  exit 1
fi
echo "all node identity tests passed"
