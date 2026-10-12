#!/usr/bin/env python3
"""Does YouTube actually serve Shorts through this egress? (docs/YOUTUBE_FIX_INVARIANTS.md)

Loads consecutive Shorts in Chrome with iPhone emulation and reports whether
each one really plays. `playabilityStatus: OK` is NOT evidence: YouTube
answers OK and then withholds the media from refused IPs, so the probe checks
the <video> element advancing instead.

Usage:
  youtube_shorts_probe.py                                  # this machine's own line (control)
  youtube_shorts_probe.py --proxy socks5://127.0.0.1:1080  # any SOCKS proxy

To test whether an IP is accepted with no VPN protocol involved, forward plain
TCP through that node (e.g. `ssh -D 1080 root@<node>`) and use that port.
An accepted IP plays N/N; a refused one shows 0/N with "unavailable".

Requires: pip install playwright, and Google Chrome installed.
Exit status: 0 only if every Short played.
"""

from __future__ import annotations

import argparse
import sys


# SOCS pre-accepts the EU consent dialog so it cannot mask a result.
CONSENT_COOKIE = {
    "name": "SOCS",
    "value": "CAESEwgDEgk0ODE3Nzk3MjQaAmVuIAEaBgiA_LyaBg",
    "domain": ".youtube.com",
    "path": "/",
}

PLAYER_STATE_JS = """() => {
  const vids = [...document.querySelectorAll('video')];
  const v = vids.find(x => x.readyState > 0 || x.currentTime > 0) || vids[0];
  const text = (document.body.innerText || '').toLowerCase();
  return {
    path: location.pathname,
    time: v ? +v.currentTime.toFixed(2) : null,
    ready: v ? v.readyState : null,
    unavailable: text.includes('video unavailable') || text.includes('unavailable'),
  };
}"""


def run(proxy: str | None, count: int, start_id: str, channel: str) -> int:
    from playwright.sync_api import sync_playwright
    with sync_playwright() as p:
        browser = p.chromium.launch(
            channel=channel,
            headless=True,
            proxy={"server": proxy} if proxy else None,
            args=["--mute-audio", "--autoplay-policy=no-user-gesture-required", "--disable-quic"],
        )
        context = browser.new_context(
            **p.devices["iPhone 13"], locale="en-US", timezone_id="Europe/Berlin"
        )
        context.add_cookies([CONSENT_COOKIE])
        page = context.new_page()
        page.goto(
            f"https://m.youtube.com/shorts/{start_id}",
            wait_until="domcontentloaded",
            timeout=45000,
        )
        played = 0
        for index in range(1, count + 1):
            page.wait_for_timeout(2000)
            first = page.evaluate(PLAYER_STATE_JS)
            page.wait_for_timeout(4500)
            second = page.evaluate(PLAYER_STATE_JS)
            advancing = (
                first["time"] is not None
                and second["time"] is not None
                and second["time"] > first["time"] + 0.5
            )
            played += advancing
            print(
                f"short #{index:<2} {second['path']:<24} t={second['time']} "
                f"ready={second['ready']} playing={advancing} unavailable={second['unavailable']}"
            )
            page.keyboard.press("ArrowDown")
        browser.close()
    print(f"RESULT: {played}/{count} Shorts actually playing")
    return 0 if played == count else 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--proxy", help="e.g. socks5://127.0.0.1:1080 (default: direct)")
    parser.add_argument("--count", type=int, default=6, help="consecutive Shorts to try")
    parser.add_argument("--start-id", default="sAcElROnYIE", help="first Short's video id")
    parser.add_argument("--channel", default="chrome", help="Playwright browser channel")
    parser.add_argument("--literal-family", type=int, choices=(4, 6), help="resolve on probe machine and send literals through loopback SOCKS; TCP only")
    args = parser.parse_args()
    if not 1 <= args.count <= 20:
        parser.error("count must be 1..20")
    if args.literal_family and not args.proxy:
        parser.error("literal-family requires a loopback SOCKS proxy")
    print("Evidence: browser emulation, TCP only; NOT native iPhone acceptance or automatic IPv4 fallback")
    if args.literal_family:
        from literal_ip_socks_front import literal_front
        try:
            with literal_front(args.proxy, args.literal_family) as proxy:
                print(f"Destination form: literal IPv{args.literal_family}; DNS runs on probe machine")
                return run(proxy, args.count, args.start_id, args.channel)
        except ValueError:
            parser.error("literal mode requires a credential-free loopback SOCKS URL")
    return run(args.proxy, args.count, args.start_id, args.channel)


if __name__ == "__main__":
    sys.exit(main())
