cd /root/b2
ts(){ date -u +%Y-%m-%dT%H:%M:%SZ; }
echo "$(ts) restarting mock control plane"
systemd-run --unit=b2-mockcp --collect /usr/bin/python3 /root/b2/mockcp.py >/dev/null 2>&1; sleep 8
tail -2 sync.log
echo "$(ts) state: $(python3 -c 'import json;s=json.load(open("/root/b2/state.json"));print({k:(v["generation"],v["state"]) for k,v in s.items()})')"
./client.py test vless current 1
./client.py test hy2 current 1
# secret leak check: no slot secret / obfs password appears in agent or sing-box journals, or the mock's log
python3 - <<'P'
import json, subprocess, glob
S = json.load(open("/root/b2/secrets.json")); vals = [S["_obfs"]] if S.get("_obfs") else []
for k, v in S.items():
    if k != "_obfs": vals += [v["vless_uuid"], v["hysteria2_password"]]
for snap in glob.glob("/root/b2/snap-*.json"):
    v = json.load(open(snap)); vals += [v["vless_uuid"], v["hysteria2_password"]]
logs = subprocess.run(["journalctl", "-u", "b2-agent", "-u", "sing-box", "-u", "b2-mockcp", "-u", "b2-renew", "-u", "b2-batch", "-u", "b2-urgent", "-u", "b2-expiry", "--no-pager", "-o", "cat"], capture_output=True, text=True).stdout
logs += open("/root/b2/sync.log").read()
for f in glob.glob("/root/b2/*.out"): logs += open(f).read()
print(f"secret-leak check: {len(set(vals))} distinct secrets checked against {len(logs.splitlines())} log lines -> {sum(v in logs for v in set(vals))} found")
P
stat -c '%a %U %n' /var/lib/vpn-provisioning-agent /var/lib/vpn-provisioning-agent/lease-pool.json /etc/vpn/compat/users/users.json
python3 -c 'import json;u=json.load(open("/etc/vpn/compat/users/users.json"));print("users.json lease users:", sorted(x["name"] for x in (u["users"] if isinstance(u,dict) else u) if x["id"].startswith("lease-")))'
