#!/usr/bin/env bash
set -u
C=/etc/vpn/compat/sing-box/config.json; B=/root/rc2-eval/config.json.orig
cp -a $C $B
restore() { cp -a $B $C; systemctl restart sing-box; sleep 3; echo "restored original config; sing-box $(systemctl is-active sing-box)"; }
trap restore EXIT
run() { vpn doctor --protocol > /root/rc2-eval/doctor-$1.txt 2>&1; echo "doctor exit=$?  FAIL=$(grep -c '^\[FAIL\]' /root/rc2-eval/doctor-$1.txt)"; grep '^\[FAIL\]' /root/rc2-eval/doctor-$1.txt | cut -c1-300; }
echo "### A. valid-but-drifted on-disk config (Hysteria2 port 443 -> 4443; sing-box still running the old one)"
python3 - <<PY
import json
c=json.load(open("$C"))
for i in c["inbounds"]:
    if i["type"]=="hysteria2": i["listen_port"]=4443
json.dump(c,open("$C","w"),indent=2)
PY
run drift
cp -a $B $C
echo "### B. syntactically broken config file (truncated)"
head -c 600 $B > $C
run truncated
