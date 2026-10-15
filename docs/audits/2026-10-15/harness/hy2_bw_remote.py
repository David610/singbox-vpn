import json, subprocess, sys, time
d = sys.argv[1]
for v in json.load(open(d + "/v.json")):
    cp = "%s/%s.json" % (d, v["port"]); json.dump(v["config"], open(cp, "w"))
    lf = open("%s/%s.log" % (d, v["port"]), "w")
    p = subprocess.Popen(["sing-box", "run", "-c", cp], stdout=lf, stderr=subprocess.STDOUT)
    time.sleep(2)
    r = subprocess.run(["curl", "-sS", "-m", "10", "-x", "socks5h://127.0.0.1:%d" % v["port"], "-o", "/dev/null", "-w", "%{http_code}", "https://www.gstatic.com/generate_204"], capture_output=True, text=True)
    p.terminate(); p.wait()
    err = [l for l in open("%s/%s.log" % (d, v["port"])).read().splitlines() if "ERROR" in l]
    print("  %-38s http=%-4s %s" % (v["name"], r.stdout.strip() or "-", (err[0].split("]")[-1].strip()[:110] if err else "")))
