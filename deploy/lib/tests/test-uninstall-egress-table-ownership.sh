#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"; rm -f "${egress_table_verify_copy:-}"' EXIT
log(){ :; }; warn(){ :; }; die(){ return 1; }; note_removed(){ :; }
export OWNERSHIP_DIR="$TMP/state" OWNERSHIP_FILE="$TMP/state/ownership.env"
. "$ROOT/deploy/lib/ownership.sh"
cat >"$TMP/nft" <<'MOCK'
#!/usr/bin/env bash
set -eu
case "$1" in
  list) [ -f "$NFT_TABLE" ] && cat "$NFT_TABLE" ;;
  delete) [ "${NFT_DELETE_FAIL:-0}" != 1 ] && rm -f "$NFT_TABLE" ;;
  --check) [ -s "$3" ] ;;
  -f)
    [ "${NFT_APPLY_FAIL:-0}" != 1 ] || exit 1
    awk 'NR == 1 && /^delete table / { next } { print }' "$2" >"$NFT_TABLE"
    ;;
  *) exit 1 ;;
esac
MOCK
chmod +x "$TMP/nft"; export PATH="$TMP:$PATH" NFT_TABLE="$TMP/table.nft"
eval "$(awk '/^cleanup_egress_table\(\) \{/{p=1} p{print} p && /^}$/{exit}' "$ROOT/deploy/almalinux/uninstall.sh")"

# The function is extracted with eval, so ShellCheck cannot infer these are its inputs/outputs.
# Route every call through a statically visible wrapper instead of suppressing diagnostics.
ownership_manifest_present=0
egress_table_pre_existed=""
egress_table_backup=""
egress_table_verify_copy=""
run_cleanup(){
  : "$ownership_manifest_present" "$egress_table_pre_existed" "$egress_table_backup" "$egress_table_verify_copy"
  cleanup_egress_table
}
reset_case(){ CRITICAL_RESIDUE=(); egress_table_restored=0; rm -rf "$OWNERSHIP_DIR" "$NFT_TABLE"; mkdir -p "$OWNERSHIP_DIR"; }
TABLE='table inet arcana_egress_isolation { chain original { type filter hook output priority filter; policy accept; } }'
MANAGED='table inet arcana_egress_isolation { chain output { type filter hook output priority filter; policy accept; } }'

# Arcana-created table and ExecStop failure: bounded direct fallback removes it.
reset_case; egress_table_pre_existed=0; egress_table_backup=""; printf '%s\n' "$MANAGED" >"$NFT_TABLE"
run_cleanup; [ ! -e "$NFT_TABLE" ]; [ "${#CRITICAL_RESIDUE[@]}" -eq 0 ]
# ExecStop plus fallback failure is critical and cannot become COMPLETE.
reset_case; egress_table_pre_existed=0; egress_table_backup=""; printf '%s\n' "$MANAGED" >"$NFT_TABLE"
NFT_DELETE_FAIL=1 run_cleanup; [ -e "$NFT_TABLE" ]; [ "${#CRITICAL_RESIDUE[@]}" -gt 0 ]
# A pre-existing table is restored byte-for-byte; unrelated state is untouched.
reset_case; mkdir -p "$OWNERSHIP_DIR/preexisting-backups"; egress_table_backup="$OWNERSHIP_DIR/preexisting-backups/EGRESS_TABLE.nft"; egress_table_pre_existed=1
printf '%s\n' "$TABLE" >"$egress_table_backup"; printf '%s\n' "$MANAGED" >"$NFT_TABLE"; touch "$TMP/unrelated"
ownership_set EGRESS_TABLE_BACKED_UP 1; run_cleanup
cmp -s "$NFT_TABLE" "$egress_table_backup"; [ "$egress_table_restored" = 1 ]; [ -e "$TMP/unrelated" ]
# Ambiguous ownership preserves the table and reports critical manual work.
reset_case; ownership_manifest_present=1; touch "$OWNERSHIP_FILE"; egress_table_pre_existed=""; egress_table_backup=""; printf '%s\n' "$MANAGED" >"$NFT_TABLE"
run_cleanup; [ -e "$NFT_TABLE" ]; [ "${#CRITICAL_RESIDUE[@]}" -gt 0 ]
# A second uninstall after state removal preserves a restored/pre-existing table
# without turning idempotence into a false critical failure.
reset_case; NONCRITICAL_RESIDUE=(); ownership_manifest_present=0; egress_table_pre_existed=""; egress_table_backup=""; printf '%s\n' "$TABLE" >"$NFT_TABLE"
run_cleanup; [ -e "$NFT_TABLE" ]; [ "${#CRITICAL_RESIDUE[@]}" -eq 0 ]; [ "${#NONCRITICAL_RESIDUE[@]}" -gt 0 ]
grep -q 'Arcana-owned nftables table inet arcana_egress_isolation still exists' "$ROOT/deploy/almalinux/uninstall.sh"
echo 'uninstall egress-table ownership tests passed'
