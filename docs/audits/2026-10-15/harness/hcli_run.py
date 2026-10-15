"""Start Hiddify's own core on each saved subscription body and push traffic through it."""
import os
import subprocess
import sys
import time

SP = os.path.dirname(os.path.abspath(__file__))
CLI = os.path.join(SP, "hiddify-cli.exe")
WORK = os.path.join(SP, "hcli-work")
PORT = 17960
cases = sys.argv[2:] or ["A", "D", "E", "F"]


def curl(args: list[str], t: int = 12) -> tuple[int, str]:
    p = subprocess.run(["curl.exe", "-sS", "--max-time", str(t), "-x", f"http://127.0.0.1:{PORT}", *args], capture_output=True, text=True)
    return p.returncode, (p.stdout + p.stderr).strip()[:100]


rows = []
for c in cases:
    log = open(os.path.join(WORK, f"run-{c}.log"), "w")
    t0 = time.time()
    proc = subprocess.Popen([CLI, "-D", WORK, "--disable-color", "run", "-c", f"case-{c}.txt", "--in-proxy-port", str(PORT),
                             "--web-port", "17961", "--log-level", "info"], stdout=log, stderr=subprocess.STDOUT, cwd=WORK)
    ok_at = None
    for _ in range(40):
        rc, out = curl(["-o", os.devnull, "-w", "%{http_code}", "https://www.gstatic.com/generate_204"], 6)
        if rc == 0 and out == "204":
            ok_at = round(time.time() - t0, 1)
            break
        if proc.poll() is not None:
            break
        time.sleep(1)
    row = {"case": c, "core_exited": proc.poll() is not None, "first_204_after_s": ok_at}
    if ok_at:
        row["egress_ip"] = curl(["https://api.ipify.org"])[1]
        row["wikipedia"] = curl(["-o", os.devnull, "-w", "%{http_code} ttfb=%{time_starttransfer}", "https://www.wikipedia.org/"])[1]
    proc.terminate()
    try:
        proc.wait(8)
    except subprocess.TimeoutExpired:
        proc.kill()
    log.close()
    tail = [ln for ln in open(os.path.join(WORK, f"run-{c}.log"), errors="replace").read().splitlines() if ln.strip()][-4:]
    row["log_tail"] = [t[:170] for t in tail] if not ok_at else "ok"
    rows.append(row)
    time.sleep(2)
import json
json.dump(rows, open(sys.argv[1], "w"), indent=1)
for r in rows:
    print(r)
