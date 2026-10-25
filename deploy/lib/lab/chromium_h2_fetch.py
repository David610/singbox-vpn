#!/usr/bin/env python3
"""Fetch N parallel HTTP/2 streams of BYTES each through a SOCKS proxy with headless Chromium (Playwright).
usage: chromium_h2_fetch.py SOCKS_PORT TARGET_PORT STREAMS BYTES ROUNDS   -> prints 'ok=<n> bad=<n>'"""
import sys
from playwright.sync_api import sync_playwright

socks, target, streams, nbytes, rounds = (int(x) for x in sys.argv[1:6])
js = """async ([base, streams, nbytes, rnd]) => {
  const one = async (i) => { try { const r = await fetch(`${base}/blob?n=${nbytes}&r=${rnd}_${i}`, {cache: 'no-store'});
      const b = await r.arrayBuffer(); return b.byteLength === nbytes; } catch (e) { return false; } };
  const t = new Promise(res => setTimeout(() => res('timeout'), 30000));
  const all = Promise.all(Array.from({length: streams}, (_, i) => one(i)));
  const out = await Promise.race([all, t]);
  return out === 'timeout' ? {ok: 0, bad: streams, timeout: true} : {ok: out.filter(x => x).length, bad: out.filter(x => !x).length};
}"""
ok = bad = 0
with sync_playwright() as p:
    b = p.chromium.launch(proxy={"server": f"socks5://127.0.0.1:{socks}"}, args=["--ignore-certificate-errors", "--disable-quic"])
    ctx = b.new_context(ignore_https_errors=True)
    pg = ctx.new_page()
    try:
        pg.goto(f"https://127.0.0.1:{target}/", wait_until="domcontentloaded", timeout=30000)
    except Exception as e:
        print(f"ok=0 bad={streams * rounds} goto_error={type(e).__name__}")
        sys.exit(0)
    for r in range(rounds):
        res = pg.evaluate(js, [f"https://127.0.0.1:{target}", streams, nbytes, r])
        ok += res["ok"]; bad += res["bad"]
    b.close()
print(f"ok={ok} bad={bad}")
