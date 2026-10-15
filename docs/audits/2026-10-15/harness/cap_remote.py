import json, subprocess, sys, time, threading
d = sys.argv[1]; cp = d + "/c.json"
p = subprocess.Popen(["sing-box", "run", "-c", cp], stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
time.sleep(2)
cpu = []
def stat():
    v = [int(x) for x in open("/proc/stat").readline().split()[1:]]
    return sum(v), v[3] + v[4]
done = threading.Event()
def sample():
    t0, i0 = stat()
    while not done.is_set():
        time.sleep(1)
        t1, i1 = stat()
        cpu.append(round(100.0 * (1 - (i1 - i0) / max(1, t1 - t0)), 1)); t0, i0 = t1, i1
threading.Thread(target=sample, daemon=True).start()
res = []
def one(out):
    r = subprocess.run(["curl", "-sS", "-m", "60", "-x", "socks5h://127.0.0.1:17920", "-o", "/dev/null", "-w", "%{http_code} %{speed_download} %{time_total}",
                        "https://speed.cloudflare.com/__down?bytes=25000000"], capture_output=True, text=True)
    c, sp_, t = (r.stdout.split() + ["0", "0", "0"])[:3]
    out.append({"http": c, "mbit_s": round(float(sp_) * 8 / 1e6, 1), "seconds": float(t)})
for streams in (1, 4, 8):
    out = []
    ths = [threading.Thread(target=one, args=(out,)) for _ in range(streams)]
    t0 = time.time(); [t.start() for t in ths]; [t.join() for t in ths]
    ok = [o for o in out if o["http"] == "200"]
    res.append({"parallel_streams": streams, "ok": len(ok), "wall_s": round(time.time() - t0, 2),
                "aggregate_mbit_s": round(sum(o["mbit_s"] for o in ok), 1)})
    time.sleep(1.5)
done.set(); p.terminate()
print(json.dumps({"loopback_REALITY_25MB_downloads": res, "host_cpu_pct_samples": cpu, "host_cpu_pct_max": max(cpu or [0])}))
