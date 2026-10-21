#!/usr/bin/env bash
# proc-safe.sh must never return or signal the caller, an ancestor or an unrelated process, and must not be fooled by a
# process whose command line only CONTAINS the pattern.
# shellcheck disable=SC2015  # ok/bad never fail, so A && ok || bad is a safe if/else here
set -Eeuo pipefail
LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=deploy/lib/proc-safe.sh
. "$LIB/proc-safe.sh"
fail=0
ok() { echo "ok: $1"; }
bad() { echo "FAIL: $1"; fail=$((fail + 1)); }
T="$(mktemp -d)"; trap 'kill $(jobs -p) 2>/dev/null || true; rm -rf "$T"' EXIT
MARK="procsafe-$$-$RANDOM"

cat >"$T/target.sh" <<S
#!/usr/bin/env bash
exec -a "/opt/fake/$MARK-svc --serve" sleep 300
S
bash "$T/target.sh" & TP=$!
# an unrelated process whose argv merely CONTAINS the marker (an editor/tail of the same name)
bash -c 'sleep 301' "/opt/fake/$MARK-svc --serve" & DECOY=$!
sleep 0.3

# 1. exact match finds the target and not the decoy
got="$(proc_pids_exact "/opt/fake/$MARK-svc --serve 300" | tr '\n' ' ')"
[ "$got" = "$TP " ] && ok "exact match returns only the target" || bad "exact match returned [$got], want [$TP]"

# 2. a bare substring (command-line-substring matching) matches nothing here
[ -z "$(proc_pids_exact "$MARK-svc")" ] && ok "a bare substring does not match (anchored)" || bad "substring matched"

# 3. the caller and its ancestors are never returned, even when the pattern matches this very script
self="$(proc_pids_exact ".*test-proc-safe.*" | tr '\n' ' ')"
case " $self " in *" $$ "*|*" $BASHPID "*) bad "returned the caller [$self]";; *) ok "caller never returned";; esac

# 4. refuses to signal itself, its parent or PID 1 even when the pattern matches everything. Runs in a DISPOSABLE tree
#    (outer -> inner) so a regression can only hurt processes created here, never the real controller running this test.
cat >"$T/inner.sh" <<S
. "$LIB/proc-safe.sh"
for p in \$\$ \$PPID 1; do
  if proc_kill_verified "\$p" ".*" KILL; then echo "SIGNALLED \$p"; else echo "refused \$p"; fi
done
S
cat >"$T/outer.sh" <<S
bash "$T/inner.sh"
echo outer-alive
S
res="$(bash "$T/outer.sh" 2>&1 || true)"
if [ "$(grep -c '^refused' <<<"$res")" = 3 ] && grep -q '^outer-alive' <<<"$res" && ! grep -q SIGNALLED <<<"$res"; then
  ok "refused self, parent and pid 1 (parent survived)"
else
  bad "protected pids not refused: $res"
fi

# 5. identity mismatch -> refuse (decoy is not the service)
if proc_kill_verified "$DECOY" "/opt/fake/$MARK-svc --serve 300" TERM; then bad "killed a decoy"; else ok "refused mismatched identity"; fi
kill -0 "$DECOY" && ok "decoy still alive"

# 6. verified target is signalled
proc_kill_verified "$TP" "/opt/fake/$MARK-svc --serve 300" TERM && ok "verified target signalled" || bad "target not signalled"
wait "$TP" 2>/dev/null || true
kill -0 "$TP" 2>/dev/null && bad "target survived" || ok "target terminated"

# 7. non-numeric / empty arguments are refused
proc_kill_verified "abc" ".*" && bad "accepted non-numeric pid" || ok "non-numeric pid refused"
[ "$fail" = 0 ]
