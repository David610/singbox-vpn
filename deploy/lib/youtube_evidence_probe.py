#!/usr/bin/env python3
"""Machine-readable YouTube playback evidence for one egress path.

Per video: playabilityStatus/reason (diagnostic only, NOT proof), count and
bytes of googlevideo.com media responses, and whether <video>.currentTime
advanced. PLAY = media delivered AND time advanced. Emits one JSON document.
No cookies/tokens are recorded. Evidence class: browser emulation (TCP only).
"""
from __future__ import annotations

import argparse
import datetime
import json
import sys

CONSENT = {"name": "SOCS", "value": "CAESEwgDEgk0ODE3Nzk3MjQaAmVuIAEaBgiA_LyaBg",
           "domain": ".youtube.com", "path": "/"}
STATE_JS = """() => {const v=[...document.querySelectorAll('video')].find(x=>x.readyState>0||x.currentTime>0)||document.querySelector('video');
 let pr=null; try{pr=window.ytInitialPlayerResponse&&window.ytInitialPlayerResponse.playabilityStatus}catch(e){}
 return {t:v?+v.currentTime.toFixed(2):null, ready:v?v.readyState:null,
  ps:pr?pr.status:null, reason:pr?(pr.reason||'').slice(0,80):null,
  bot:(document.body.innerText||'').toLowerCase().includes('not a bot'),
  unavailable:(document.body.innerText||'').toLowerCase().includes('unavailable')}}"""


def one(ctx, url: str) -> dict:
    page = ctx.new_page()
    media = {"responses": 0, "bytes": 0, "status": {}}

    def on_resp(r):
        if "googlevideo.com/videoplayback" in r.url:
            media["responses"] += 1
            media["status"][str(r.status)] = media["status"].get(str(r.status), 0) + 1
            try:
                media["bytes"] += int(r.headers.get("content-length", 0))
            except ValueError:
                pass
    page.on("response", on_resp)
    try:
        page.goto(url, wait_until="domcontentloaded", timeout=45000)
        page.wait_for_timeout(2500)
        a = page.evaluate(STATE_JS)
        page.wait_for_timeout(5000)
        b = page.evaluate(STATE_JS)
    except Exception as e:  # noqa: BLE001
        page.close()
        return {"url": url, "error": type(e).__name__, "play": False}
    page.close()
    adv = a["t"] is not None and b["t"] is not None and b["t"] > a["t"] + 0.5
    return {"url": url, "final_path": b["path"], "playabilityStatus": b["ps"], "reason": b["reason"], "bot_wall": b["bot"],
            "ui_unavailable": b["unavailable"], "time_start": a["t"], "time_end": b["t"],
            "media": media, "play": bool(adv and media["responses"] > 0)}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--proxy")
    ap.add_argument("--label", required=True, help="path label, e.g. direct-ipv4-domain")
    ap.add_argument("--shorts", nargs="*", default=["sAcElROnYIE", "NlHemkN8xq4", "GJVO2DrTqUY"])
    ap.add_argument("--videos", nargs="*", default=["jNQXAC9IpQU", "aqz-KE-bpKQ"])
    ap.add_argument("--trials", type=int, default=1)
    ap.add_argument("--out")
    ap.add_argument("--channel", default="chromium")
    a = ap.parse_args()
    from playwright.sync_api import sync_playwright
    rows = []
    with sync_playwright() as p:
        br = p.chromium.launch(channel=a.channel, headless=True, proxy={"server": a.proxy} if a.proxy else None,
                               args=["--mute-audio", "--autoplay-policy=no-user-gesture-required", "--disable-quic"])
        for trial in range(1, a.trials + 1):
            ctx = br.new_context(**p.devices["iPhone 13"], locale="en-US", timezone_id="Europe/Berlin")
            ctx.add_cookies([CONSENT])
            for s in a.shorts:
                rows.append({"kind": "short", "trial": trial, **one(ctx, f"https://m.youtube.com/shorts/{s}")})
            for v in a.videos:
                rows.append({"kind": "video", "trial": trial, **one(ctx, f"https://m.youtube.com/watch?v={v}")})
            ctx.close()
        br.close()
    # A "Short" counts only if the page really stayed on /shorts/ (YouTube
    # redirects ids that are not Shorts to /watch); everything else is reported
    # separately and never inflates the Shorts success count.
    for r in rows:
        r["counts_as_short"] = r["kind"] == "short" and str(r.get("final_path", "")).startswith("/shorts/")
    shorts = [r for r in rows if r["counts_as_short"]]
    excluded = [r for r in rows if r["kind"] == "short" and not r["counts_as_short"]]
    vids = [r for r in rows if r["kind"] == "video"]
    summ = {"short": f"{sum(r['play'] for r in shorts)}/{len(shorts)}",
            "video": f"{sum(r['play'] for r in vids)}/{len(vids)}",
            "non_short_excluded": len(excluded)}
    doc = {"schema": 1, "label": a.label, "utc": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
           "evidence_class": "browser-emulation-tcp-only", "summary": summ, "results": rows}
    out = json.dumps(doc, indent=1)
    if a.out:
        open(a.out, "w").write(out + "\n")
    print(json.dumps({"label": a.label, **summ}))
    return 0 if shorts and all(r["play"] for r in shorts) and all(r["play"] for r in vids) else 1


if __name__ == "__main__":
    sys.exit(main())
