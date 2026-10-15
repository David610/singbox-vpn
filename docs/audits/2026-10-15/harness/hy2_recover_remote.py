import json, subprocess, sys, threading, time
d = sys.argv[1]
cfg = json.load(open(d + "/v.json"))[0]
cp = d + "/c.json"; json.dump(cfg["config"], open(cp, "w"))
port = cfg["port"]
lf = open(d + "/client.log", "w")
p = subprocess.Popen(["sing-box", "run", "-c", cp], stdout=lf, stderr=subprocess.STDOUT)
def probe(t=3):
    s = time.time()
    r = subprocess.run(["curl", "-sS", "-m", str(t), "-x", "socks5h://127.0.0.1:%d" % port, "-o", "/dev/null", "-w", "%{http_code}",
                        "https://www.gstatic.com/generate_204"], capture_output=True, text=True)
    return r.stdout.strip() == "204", round(time.time() - s, 2), r.stderr.strip()[:50]
time.sleep(2)
assert probe(8)[0], "warm-up failed"
out = []
for n in range(3):
    ev = {}
    def restart():
        ev["restart_begin"] = time.time() - t0
        subprocess.run(["systemctl", "reset-failed", "sing-box"]); ev["restart_begin"] = time.time() - t0; subprocess.run(["systemctl", "restart", "sing-box"])
        ev["restart_end"] = time.time() - t0
    t0 = time.time()
    th = threading.Thread(target=restart); th.start()
    tl = []
    first_ok_after_restart_end = None
    while time.time() - t0 < 80:
        ok, dur, err = probe(3)
        t = round(time.time() - t0, 2)
        tl.append((t, ok, dur))
        if ok and "restart_end" in ev and t - dur >= ev["restart_end"] - 0.01 and first_ok_after_restart_end is None:
            first_ok_after_restart_end = t
        if first_ok_after_restart_end is not None and sum(1 for x in tl[-3:] if x[1]) == 3:
            break
    th.join()
    fails = [x for x in tl if not x[1]]
    out.append({"run": n + 1, "restart_cmd_s": [round(ev["restart_begin"], 2), round(ev["restart_end"], 2)],
                "first_probe_started_after_restart_that_succeeded_at_s": first_ok_after_restart_end,
                "failed_probes": len(fails), "last_failure_at_s": fails[-1][0] if fails else None,
                "total_probes": len(tl)})
    time.sleep(8)
p.terminate()
print(json.dumps(out, indent=0))
