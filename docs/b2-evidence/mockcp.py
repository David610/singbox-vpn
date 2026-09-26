#!/usr/bin/env python3
"""Minimal stand-in for vpn-web's agent API (B2 real test only).
Implements /api/agent/leases/sync with the same semantics as the
agent_sync_lease_slots RPC (incl. renewal `extend_to`, `urgent`
revocations and adopted valid_until). Secrets go only to a 0600 file;
nothing secret is ever printed or logged."""
import json, os, datetime, threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
D = "/root/b2"; os.makedirs(D, mode=0o700, exist_ok=True)
SECRETS = f"{D}/secrets.json"; STATE = f"{D}/state.json"; CTRL = f"{D}/control.json"
lock = threading.Lock()
def load(p, d):
    try: return json.load(open(p))
    except Exception: return d
def save(p, v):
    fd = os.open(p + ".tmp", os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f: json.dump(v, f)
    os.replace(p + ".tmp", p)
class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def reply(self, obj, code=200):
        b = json.dumps(obj).encode(); self.send_response(code)
        self.send_header("Content-Type", "application/json"); self.send_header("Content-Length", str(len(b)))
        self.end_headers(); self.wfile.write(b)
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
        if self.path == "/api/agent/claim": return self.reply({"job": None})
        if self.path != "/api/agent/leases/sync": return self.reply({})
        with lock:
            secrets = load(SECRETS, {}); state = load(STATE, {}); ctrl = load(CTRL, {})
            need, seen = [], set()
            for it in body["slots"]:
                k = str(it["slot"]); seen.add(k); row = state.get(k)
                if row and row["generation"] == it["generation"] and it["valid_until"] > row["valid_until"]:
                    row["valid_until"] = it["valid_until"]  # node adopted a renewal
                    secrets[k]["valid_until"] = it["valid_until"]
                if row and row["generation"] >= it["generation"]: continue
                if "vless_uuid" not in it: need.append(it["slot"]); continue
                state[k] = {"generation": it["generation"], "state": "active", "valid_until": it["valid_until"]}
                secrets[k] = {"generation": it["generation"], "vless_uuid": it["vless_uuid"],
                              "hysteria2_password": it["hysteria2_password"], "valid_until": it["valid_until"]}
            for k in list(state):
                if k not in seen: del state[k]
            # control: {"lease":[slot], "revoke":[slot], "revoke_urgent":[slot],
            #           "extend":{slot: rfc3339}} applied to the CURRENT generation
            for s in ctrl.pop("lease", []):
                if str(s) in state and state[str(s)]["state"] == "active": state[str(s)]["state"] = "leased"
            for s in ctrl.pop("revoke", []):
                if str(s) in state: state[str(s)]["state"] = "revoked"
            for s in ctrl.pop("revoke_urgent", []):
                if str(s) in state: state[str(s)].update(state="revoked", urgent=True)
            for s, t in ctrl.pop("extend", {}).items():
                if s in state and state[s]["state"] == "leased": state[s]["extend_to"] = t
            if "hysteria2_obfs_password" in body:
                secrets["_obfs"] = body["hysteria2_obfs_password"]
            save(SECRETS, secrets); save(STATE, state); save(CTRL, ctrl)
            with open(f"{D}/sync.log", "a") as log:
                log.write(f"{datetime.datetime.utcnow().isoformat()}Z sync slots={len(body['slots'])} "
                          f"with_secret={sum(1 for i in body['slots'] if 'vless_uuid' in i)} need={len(need)}\n")
            self.reply({"as_of": datetime.datetime.now(datetime.timezone.utc).isoformat().replace("+00:00", "Z"),
                        "min_remaining_seconds": 600, "obfs_stored": "hysteria2_obfs_password" in body,
                        "need_secret": need,
                        "slots": [{"slot": int(k), "generation": v["generation"], "state": v["state"],
                                   "urgent": v.get("urgent", False), "extend_to": v.get("extend_to")} for k, v in sorted(state.items(), key=lambda kv: int(kv[0]))]})
ThreadingHTTPServer(("127.0.0.1", 8788), H).serve_forever()
