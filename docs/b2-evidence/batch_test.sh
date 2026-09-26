# Batch interval bounds restarts: one non-urgent revocation every 30 s for
# 8 minutes (16 revocations) against a 120 s rotation grid.
set -u; . /root/b2/lib.sh
T=$(date +%s); PID0=$(sbpid)
echo "$(ts) start; grid=120s; $(state)"
for i in $(seq 0 15); do
  s=$((i % 4))
  ctl "{\"lease\":[$s],\"revoke\":[$s]}"
  echo "$(ts) revoke #$((i+1)) slot $s (non-urgent)"
  sleep 30
done
sleep 5
echo "--- agent rotations:"
agentlog_since $T | grep -E 'rotating'
echo "--- sing-box starts per 120 s grid window:"
journalctl -u sing-box --since "@$T" --no-pager -o short-unix | grep 'Started sing-box' | awk '{t=int($1); w=int(t/120)*120; c[w]++} END {for (w in c) print strftime("%H:%M:%S",w,1)"-"strftime("%H:%M:%S",w+120,1)": "c[w]}' | sort
echo "$(ts) revocations=16 restarts=$(restarts_since $T) window=$(( $(date +%s) - T ))s ($(( ($(date +%s) - T + 119) / 120 )) grid windows touched)"
