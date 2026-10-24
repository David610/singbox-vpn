#!/usr/bin/env python3
"""Stand-in for deploy/lib/diag/shorts_media_probe.py (no browser, no network).

Honours the same CLI (--proxy --ids --dwell --label --out, plus --self-check) and the same
result-row shape. Behaviour comes from QT_PROBE_MODE.
"""
import argparse
import json
import os
import sys
import time

MODE = os.environ.get("QT_PROBE_MODE", "all_played")
STATE = os.environ.get("QT_STATE", ".")

ap = argparse.ArgumentParser()
ap.add_argument("--proxy")
ap.add_argument("--ids", default="")
ap.add_argument("--dwell", type=float, default=25)
ap.add_argument("--label", default="run")
ap.add_argument("--out")
ap.add_argument("--self-check", action="store_true")
a = ap.parse_args()

if a.self_check:
    if MODE == "selfcheck_fail":
        sys.stderr.write("playwright not importable\n")
        sys.exit(3)
    sys.exit(0)

with open(os.path.join(STATE, "probe.pid"), "a", encoding="utf-8") as fh:
    fh.write(str(os.getpid()) + "\n")
with open(os.path.join(STATE, "probe.args"), "a", encoding="utf-8") as fh:
    fh.write(json.dumps({"proxy": a.proxy, "ids": a.ids, "dwell": a.dwell, "label": a.label}) + "\n")

if MODE == "hang":
    time.sleep(600)
if MODE == "crash":
    sys.stderr.write("Traceback: boom\n")
    sys.exit(3)


def row(vid, kind):
    base = dict(id=vid, err=None, started=True, start_s=1.5, played_s=a.dwell, max_stall_s=0.5,
                bot_challenge=False, unavailable=False, still_on_shorts=not vid.startswith("watch:"),
                videoplayback_reqs=12, refusal_155B_like=0, media_bytes=900000, statuses=[200, 206],
                verdict="PLAYED")
    if kind == "refused":
        base.update(started=False, start_s=None, played_s=0, unavailable=True, refusal_155B_like=3,
                    media_bytes=0, verdict="REFUSED")
    elif kind == "bot":
        base.update(started=False, start_s=None, played_s=0, bot_challenge=True, media_bytes=0, verdict="BOT_CHALLENGE")
    elif kind == "frozen":
        base.update(played_s=a.dwell * 0.2, max_stall_s=12.0, verdict="FROZE")
    elif kind == "no_start":
        base.update(started=False, start_s=None, played_s=0, media_bytes=0, verdict="NO_START")
    elif kind == "nav_error":
        base.update(started=False, start_s=None, played_s=0, media_bytes=0, err="TimeoutError", verdict="NO_START")
    elif kind == "not_short":
        base.update(still_on_shorts=False)
    return base


ids = [i for i in a.ids.split(",") if i]
rows = []
for n, vid in enumerate(ids):
    if MODE in ("all_played", "garbage", "no_output"):
        kind = "ok"
    elif MODE == "refused":
        kind = "refused"
    elif MODE == "bot":
        kind = "bot"
    elif MODE == "frozen":
        kind = "frozen"
    elif MODE == "no_start":
        kind = "no_start"
    elif MODE == "nav_error":
        kind = "nav_error"
    elif MODE == "not_short":
        kind = "not_short"
    elif MODE == "one_refused":
        kind = "refused" if n == 0 else "ok"
    elif MODE == "truncated":
        if n >= max(1, len(ids) - 3):
            break
        kind = "ok"
    elif MODE == "sustained_frozen":
        kind = "frozen" if vid.startswith("watch:") else "ok"
    else:
        raise SystemExit("unknown QT_PROBE_MODE " + MODE)
    rows.append(row(vid, kind))

if MODE == "garbage":
    open(a.out, "w", encoding="utf-8").write("{not json")
elif MODE != "no_output":
    json.dump(rows, open(a.out, "w", encoding="utf-8"))
print("SUMMARY", a.label)
