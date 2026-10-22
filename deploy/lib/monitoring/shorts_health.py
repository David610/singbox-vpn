#!/usr/bin/env python3
"""Shorts health monitor for one egress path.

Tunnel health and Shorts-MEDIA health are different properties: a tunnel can be perfect while YouTube withholds Shorts
media from the egress IP. Each run therefore answers two separate questions and never blames one for the other:

  1. PATH   - can this path (direct, or through the given SOCKS proxy) fetch a Google and a neutral HTTPS control at all?
  2. MEDIA  - do genuine Shorts really play (media delivered AND the clock advances)?

Status (exit code 0 only for OK; a failed unit is visible by design):
  OK            path ok, every genuine Short played
  SUSPECT       first miss with a working path (not paged yet; never OK)
  DEGRADED      WARN_AFTER consecutive non-OK runs
  CRITICAL      CRIT_AFTER consecutive non-OK runs
  PATH_DOWN     the path itself failed its controls (DNS/connect/TLS/timeout): NOT an egress-refusal verdict
  UNVERIFIED    the probe could not run or too few genuine Shorts loaded (a broken harness is not a broken VPN)

`cause` is the OBSERVED outcome of a non-OK run: PATH_DOWN, HARNESS, or MEDIA_PLAYBACK_FAILED (genuine Shorts did not play on
a working path; `failure_mode` WITHHELD / STALLED / MIXED keeps the detail). `explanation` is the INFERRED reason, kept
separate (I-83): UNKNOWN unless stronger evidence exists. A 155-byte application/vnd.yt-ump reply, an "unavailable" page,
a bot check or zero media through a signed-out automated browser (`context` synthetic-signed-out-automated) do not by
themselves prove an IP refusal. EGRESS_REFUSED needs the same harness to play every genuine Short through a reference
egress in the same run; AUTOMATION_SPECIFIC_FAILURE needs a fresh native (non-automated) success on this egress;
NETWORK_FAILURE and PROBE_INVALID come from the path controls and the harness. Alerts fire on transitions only (with a cooldown) and on recovery, which
needs RECOVER_AFTER consecutive OK runs (no flapping). There is deliberately NO automatic re-routing here: moving Google
traffic to another egress is an owner-approved action (docs/operations/YOUTUBE_EGRESS_ARCHITECTURE.md, INV-6/7/8); this
monitor only detects, classifies, records and alerts. Only public video ids and aggregate counts are stored.
"""
from __future__ import annotations

import argparse
import datetime
import fcntl
import json
import os
import random
import re
import subprocess
import sys
import time

MIN_SHORTS = 3
WARN_AFTER = 2
CRIT_AFTER = 4
RECOVER_AFTER = 2
# A media-failure streak older than this is not "consecutive" with a new failure (probe/host down for hours in between).
MEDIA_STALE_S = 3 * 3600
ALERT_COOLDOWN_S = 6 * 3600
NATIVE_EVIDENCE_MAX_AGE_S = 3600
SYNTHETIC_CONTEXT = "synthetic-signed-out-automated"
MIN_INTERVAL_S = 300
ID_RE = re.compile(r"^[A-Za-z0-9_-]{11}$")

# Controls: one Google endpoint (the destination class that matters) and one neutral endpoint (separates "the tunnel is down"
# from "Google specifically is unreachable"). Overridable; both are plain GETs that return 204/200 and carry no user data.
DEFAULT_CONTROLS = {"google": "https://www.gstatic.com/generate_204", "neutral": "https://www.cloudflare.com/cdn-cgi/trace"}
# Fallback only; the operator rotates a larger corpus file (SHORTS_CORPUS). Ids that are not Shorts are excluded by the probe.
DEFAULT_SHORTS = ["sAcElROnYIE", "NlHemkN8xq4", "GJVO2DrTqUY"]

CURL_CLASS = {6: "DNS", 7: "CONNECT", 28: "TIMEOUT", 35: "TLS", 51: "TLS", 58: "TLS", 60: "TLS", 97: "PROXY", 52: "EMPTY", 56: "RESET"}

# ---------------------------------------------------------------------------------------------------------------- pure logic


def classify_row(r: dict) -> str:
    """One probe row (kind == short) -> a single class. Order matters: the most specific evidence wins."""
    if not r.get("counts_as_short"):
        return "NOT_SHORT"
    if r.get("play"):
        return "PLAY"
    if r.get("error"):
        return "PROBE_TIMEOUT" if "Timeout" in str(r["error"]) else "PROBE_ERROR"
    if r.get("bot_wall"):
        return "BOT_CHECK"
    media = (r.get("media") or {}).get("responses", 0)
    ps = r.get("playabilityStatus")
    if ps and ps != "OK" or r.get("ui_unavailable"):
        return "REFUSED"
    if media == 0:
        return "MEDIA_WITHHELD"
    return "STALLED"


REFUSAL = {"BOT_CHECK", "REFUSED", "MEDIA_WITHHELD"}


def summarize(probe: dict | None) -> dict:
    if probe is None:
        return {"genuine": 0, "played": 0, "classes": {}, "videos": "0/0"}
    rows = [r for r in probe.get("results", []) if r.get("kind") == "short"]
    classes: dict[str, int] = {}
    for r in rows:
        c = classify_row(r)
        classes[c] = classes.get(c, 0) + 1
    genuine = [r for r in rows if r.get("counts_as_short")]
    vids = [r for r in probe.get("results", []) if r.get("kind") == "video"]
    return {"genuine": len(genuine), "played": sum(1 for r in genuine if r.get("play")), "classes": classes,
            "videos": f"{sum(1 for v in vids if v.get('play'))}/{len(vids)}"}


def classify_path(controls: dict[str, dict]) -> tuple[bool, str]:
    """controls: name -> {"ok": bool, "class": str}. Path is OK only if every control worked."""
    bad = {k: v.get("class", "UNKNOWN") for k, v in controls.items() if not v.get("ok")}
    if not bad:
        return True, "all controls reachable"
    return False, "control(s) failed: " + ", ".join(f"{k}={c}" for k, c in sorted(bad.items()))


NO_EVIDENCE_CAUSES = ("HARNESS", "PATH_DOWN")


def klass(cause: str | None) -> str:
    """Evidence class of a run: 'other' = it produced NO media evidence (probe could not run / too few Shorts / path down),
    'media' = real playback evidence against the egress. The two have separate streaks: a broken probe is not a refusal."""
    return "other" if cause in NO_EVIDENCE_CAUSES else "media"


def effective_media_streak(prev: dict, now: float) -> int:
    """The persisted media streak, or 0 when its last media observation is too old to be 'consecutive'."""
    n = int(prev.get("media_streak", prev.get("streak", 0)) or 0) if "media_last" in prev or "media_streak" in prev else 0
    return n if now - float(prev.get("media_last", 0) or 0) <= MEDIA_STALE_S else 0


def explain(s: dict, probe: dict | None, native_played: bool | None) -> str:
    """Inferred reason for a media playback failure (I-83). Only comparative evidence moves it off UNKNOWN."""
    ref = (probe or {}).get("reference") or {}
    if ref.get("genuine", 0) >= MIN_SHORTS and ref.get("played") == ref.get("genuine"):
        return "EGRESS_REFUSED"            # same harness, same run, only the egress differs
    if native_played is True:
        return "AUTOMATION_SPECIFIC_FAILURE"
    return "UNKNOWN"


def evaluate(prev_streak: int, probe: dict | None, path_ok: bool = True, path_reason: str = "", prev_other: int = 0,
             native_played: bool | None = None) -> dict:
    """Pure: -> {status, cause, explanation, failure_mode, context, streak, media_streak, other_streak, reason, summary}.
    `prev_streak` is the MEDIA streak, `prev_other` the no-evidence streak. A media failure advances the media streak and clears
    the other one (the probe demonstrably works); a run without evidence advances only the other streak and leaves the media
    streak untouched; OK clears both. `streak` is the streak of this run's own class (it drives the status)."""
    s = summarize(probe)
    if not path_ok:
        # The path is down: the media verdict is meaningless (and must not be blamed on YouTube).
        return _bad(prev_streak, prev_other, "PATH_DOWN", "PATH_DOWN", path_reason or "path controls failed", s, "NETWORK_FAILURE")
    if probe is None:
        return _bad(prev_streak, prev_other, "UNVERIFIED", "HARNESS", "probe did not run or produced no result", s, "PROBE_INVALID")
    if s["genuine"] < MIN_SHORTS:
        return _bad(prev_streak, prev_other, "UNVERIFIED", "HARNESS",
                    f"only {s['genuine']} genuine Shorts loaded (need {MIN_SHORTS}); non-Shorts are excluded", s, "PROBE_INVALID")
    if s["played"] == s["genuine"]:
        return {"status": "OK", "cause": None, "explanation": None, "failure_mode": None, "context": SYNTHETIC_CONTEXT,
                "streak": 0, "media_streak": 0, "other_streak": 0,
                "reason": f"{s['played']}/{s['genuine']} genuine Shorts played", "summary": s}
    failing = {c: n for c, n in s["classes"].items() if c not in ("PLAY", "NOT_SHORT")}
    refused = sum(n for c, n in failing.items() if c in REFUSAL)
    stalled = failing.get("STALLED", 0)
    errors = sum(n for c, n in failing.items() if c.startswith("PROBE_"))
    reason = f"{s['played']}/{s['genuine']} genuine Shorts played; {failing}"
    if errors and not refused and not stalled:
        # only probe errors/timeouts on a path whose controls passed: not evidence against the egress
        return _bad(prev_streak, prev_other, "FAILING", "HARNESS", reason, s, "PROBE_INVALID")
    if refused and refused >= max(stalled, errors):
        mode = "WITHHELD"
    elif stalled > refused and stalled >= errors:
        mode = "STALLED"
    else:
        mode = "MIXED"
    return _bad(prev_streak, prev_other, "FAILING", "MEDIA_PLAYBACK_FAILED", reason, s, explain(s, probe, native_played), mode)


def _bad(prev_media: int, prev_other: int, kind: str, cause: str, reason: str, s: dict, explanation: str = "UNKNOWN",
         failure_mode: str | None = None) -> dict:
    if klass(cause) == "other":
        media, other = prev_media, prev_other + 1      # no evidence: the media streak is neither advanced nor reset
        n = other
    else:
        media, other = prev_media + 1, 0               # real evidence: the probe works, so any no-evidence streak ends
        n = media
    if kind == "PATH_DOWN":
        status = "CRITICAL" if n >= CRIT_AFTER else "DEGRADED" if n >= WARN_AFTER else "PATH_DOWN"
    elif n >= CRIT_AFTER:
        status = "CRITICAL"
    elif n >= WARN_AFTER:
        status = "DEGRADED"
    else:
        status = "SUSPECT" if kind == "FAILING" else "UNVERIFIED"
    return {"status": status, "cause": cause, "explanation": explanation, "failure_mode": failure_mode,
            "context": SYNTHETIC_CONTEXT, "streak": n, "media_streak": media, "other_streak": other,
            "reason": f"{kind}: {reason}", "summary": s}


def next_state(prev: dict, ev: dict, now: float) -> tuple[dict, list[dict]]:
    """Pure: previous persisted state + this run's evaluation -> (new state, alert events).
    Alerts: entering DEGRADED/CRITICAL (once per level per cooldown) and RECOVERED after RECOVER_AFTER consecutive OK runs."""
    st = dict(prev)
    events: list[dict] = []
    level = {"OK": 0, "SUSPECT": 1, "UNVERIFIED": 1, "PATH_DOWN": 1, "DEGRADED": 2, "CRITICAL": 3}[ev["status"]]
    st["streak"] = ev["streak"]
    st["media_streak"] = ev.get("media_streak", ev["streak"] if klass(ev["cause"]) == "media" else st.get("media_streak", 0))
    st["other_streak"] = ev.get("other_streak", ev["streak"] if klass(ev["cause"]) == "other" else 0)
    if klass(ev["cause"]) == "media" and ev["status"] != "OK":
        st["media_last"] = now
    st["status"] = ev["status"]
    if ev["status"] == "OK":
        st["ok_streak"] = st.get("ok_streak", 0) + 1
        if st.get("alerted_level", 0) and st["ok_streak"] >= RECOVER_AFTER:
            events.append({"event": "RECOVERED", "cause": st.get("alerted_cause"), "ok_streak": st["ok_streak"]})
            st["alerted_level"] = 0
            st["alerted_levels"] = {}
            st["alerted_cause"] = None
    else:
        st["ok_streak"] = 0
        if level >= 2:
            # Deduplicate per evidence class: "the monitor is blind" (HARNESS/PATH_DOWN) must never mask "the egress is refused".
            k = klass(ev["cause"])
            last = st.get("alert_times", {}).get(f"{k}:{level}", 0)
            if level > st.get("alerted_levels", {}).get(k, 0) or now - last >= ALERT_COOLDOWN_S:
                events.append({"event": ev["status"], "cause": ev["cause"], "explanation": ev.get("explanation"),
                               "context": ev.get("context"), "streak": ev["streak"], "reason": ev["reason"]})
                st.setdefault("alert_times", {})[f"{k}:{level}"] = now
                st.setdefault("alerted_levels", {})[k] = max(st.get("alerted_levels", {}).get(k, 0), level)
                st["alerted_level"] = max(st.get("alerted_level", 0), level)
                st["alerted_cause"] = ev["cause"]
    return st, events


EXIT = {"OK": 0, "SUSPECT": 1, "DEGRADED": 1, "CRITICAL": 2, "UNVERIFIED": 3, "PATH_DOWN": 3}


def load_native_evidence(path: str | None, label: str, now: float) -> bool | None:
    """A recorded native (non-automated) observation for this egress: {label, unix, shorts_played, context}. Only a fresh
    record for this label counts; anything else is no evidence (None). It can refine the explanation, never the status."""
    if not path:
        return None
    try:
        rec = json.load(open(path))
    except (OSError, ValueError):
        return None
    if rec.get("label") != label or not isinstance(rec.get("shorts_played"), bool):
        return None
    if not 0 <= now - float(rec.get("unix", 0) or 0) <= NATIVE_EVIDENCE_MAX_AGE_S:
        return None
    return rec["shorts_played"]


def load_corpus(path: str | None) -> list[str]:
    ids: list[str] = []
    if path:
        try:
            for line in open(path):
                v = line.split("#", 1)[0].strip()
                if ID_RE.match(v):
                    ids.append(v)
        except OSError:
            pass
    return ids or [i for i in DEFAULT_SHORTS if ID_RE.match(i)]


def pick(corpus: list[str], n: int, rng: random.Random) -> list[str]:
    """A rotating sample, so no single static id decides the verdict forever. Corpus entries that turn out not to be
    Shorts are excluded by the probe (counts_as_short=False), never counted as a pass or a failure."""
    return rng.sample(corpus, min(n, len(corpus)))


# ---------------------------------------------------------------------------------------------------------------- effects


def path_controls(proxy: str | None, controls: dict[str, str], run=subprocess.run) -> dict[str, dict]:
    out: dict[str, dict] = {}
    for name, url in controls.items():
        cmd = ["curl", "-sS", "-o", "/dev/null", "--max-time", "20", "-w", "%{http_code}"]
        if proxy:
            cmd += ["--proxy", proxy.replace("socks5://", "socks5h://", 1)]
        cmd.append(url)
        try:
            p = run(cmd, capture_output=True, text=True, timeout=30)
            code = (p.stdout or "").strip()
            if p.returncode == 0 and code and code[0] in "23":
                out[name] = {"ok": True, "class": "OK", "http": code}
            else:
                out[name] = {"ok": False, "class": CURL_CLASS.get(p.returncode, "HTTP_" + code if p.returncode == 0 else f"CURL_{p.returncode}")}
        except subprocess.TimeoutExpired:
            out[name] = {"ok": False, "class": "TIMEOUT"}
        except OSError:
            out[name] = {"ok": False, "class": "NO_CURL"}
    return out


def run_probe(a, shorts: list[str], out: str, run=subprocess.run) -> dict | None:
    cmd = [a.python, a.probe, "--label", a.label, "--videos", "--shorts", *shorts, "--discover", str(a.discover), "--out", out]
    if a.proxy:
        cmd += ["--proxy", a.proxy]
    try:
        if os.path.exists(out):
            os.unlink(out)
        r = run(cmd, capture_output=True, text=True, timeout=600)
        # keep the tail of the probe's stderr: a HARNESS verdict without a reason is not operable (public ids/paths only)
        with open(out + ".err", "w") as ef:
            ef.write((r.stderr or "")[-4000:])
        return json.load(open(out))
    except Exception:  # noqa: BLE001 - any failure to obtain a result is a HARNESS problem, never a verdict on the egress
        return None


def send_alert(cmd: str | None, event: dict, run=subprocess.run) -> str | None:
    """Pipe one JSON event (no secrets) to the operator's alert command. Returns an error string or None."""
    if not cmd:
        return None
    try:
        p = run(cmd, input=json.dumps(event) + "\n", shell=True, capture_output=True, text=True, timeout=30)  # noqa: S602 - operator-supplied
        return None if p.returncode == 0 else f"alert command exited {p.returncode}"
    except Exception as e:  # noqa: BLE001
        return f"alert command failed: {type(e).__name__}"


def main(argv=None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--label", required=True, help="egress path label, e.g. exit-ipv4")
    ap.add_argument("--proxy", default=None, help="SOCKS URL of the path under test; default: $PROXY_<label> or direct")
    ap.add_argument("--state-dir", default="/var/lib/singbox-vpn/shorts-health")
    ap.add_argument("--probe", default=os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "youtube_evidence_probe.py"))
    ap.add_argument("--python", default=os.environ.get("PROBE_PYTHON", sys.executable))
    ap.add_argument("--corpus", default=os.environ.get("SHORTS_CORPUS"))
    ap.add_argument("--sample", type=int, default=2, help="ids taken from the corpus per run")
    ap.add_argument("--discover", type=int, default=3, help="ids discovered from the live Shorts feed per run")
    ap.add_argument("--alert-cmd", default=os.environ.get("SHORTS_ALERT_CMD"))
    ap.add_argument("--native-evidence", default=os.environ.get("SHORTS_NATIVE_EVIDENCE"),
                    help="JSON record of a recent native-device observation on this egress (refines the explanation only)")
    ap.add_argument("--force", action="store_true", help="ignore the minimum interval between runs")
    a = ap.parse_args(argv)
    if a.proxy is None:
        a.proxy = os.environ.get("PROXY_" + a.label.replace("-", "_")) or os.environ.get("PROXY_" + a.label)
    os.makedirs(a.state_dir, mode=0o750, exist_ok=True)
    lock = open(os.path.join(a.state_dir, f"{a.label}.lock"), "w")
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        print(json.dumps({"label": a.label, "status": "SKIPPED", "reason": "another run holds the lock"}))
        return 0
    state_file = os.path.join(a.state_dir, f"{a.label}.state.json")
    try:
        prev = json.load(open(state_file))
    except (OSError, ValueError):
        prev = {}
    now = time.time()
    if not a.force and now - prev.get("last_run", 0) < MIN_INTERVAL_S:
        print(json.dumps({"label": a.label, "status": "SKIPPED", "reason": "ran less than 5 minutes ago (rate limit)"}))
        return 0
    controls = path_controls(a.proxy, DEFAULT_CONTROLS)
    path_ok, path_reason = classify_path(controls)
    shorts = pick(load_corpus(a.corpus), a.sample, random.Random(int(now) // 1800))
    probe = None
    t0 = time.time()
    if path_ok:   # no point (and no load on Google) in driving a browser through a dead path
        probe = run_probe(a, shorts, os.path.join(a.state_dir, f"{a.label}.last-probe.json"))
    ev = evaluate(effective_media_streak(prev, now), probe, path_ok, path_reason, int(prev.get("other_streak", 0) or 0),
                  load_native_evidence(a.native_evidence, a.label, now))
    st, events = next_state(prev, ev, now)
    st["last_run"] = now
    st["utc"] = datetime.datetime.fromtimestamp(now, datetime.timezone.utc).isoformat(timespec="seconds")
    alert_errors = []
    for e in events:
        e = {**e, "label": a.label, "utc": st["utc"]}
        err = send_alert(a.alert_cmd, e)
        with open(os.path.join(a.state_dir, "alerts.jsonl"), "a") as f:
            f.write(json.dumps({**e, "delivery_error": err}) + "\n")
        if err:
            alert_errors.append(err)
    with open(state_file + ".tmp", "w") as f:
        json.dump(st, f)
    os.replace(state_file + ".tmp", state_file)
    line = {"utc": st["utc"], "label": a.label, "status": ev["status"], "cause": ev["cause"],
            "explanation": ev["explanation"], "failure_mode": ev["failure_mode"], "context": ev["context"], "streak": ev["streak"],
            "media_streak": ev["media_streak"], "other_streak": ev["other_streak"], "reason": ev["reason"], "path": {k: v["class"] for k, v in controls.items()}, "sample": shorts,
            "summary": ev["summary"], "probe_seconds": round(time.time() - t0, 1), "alerts": [e["event"] for e in events],
            "alert_errors": alert_errors}
    with open(os.path.join(a.state_dir, "shorts-health.jsonl"), "a") as f:
        f.write(json.dumps(line) + "\n")
    print(json.dumps(line))
    # An undeliverable alert must be loud: it is the one failure that would otherwise hide all the others.
    return 4 if alert_errors else EXIT[ev["status"]]


if __name__ == "__main__":
    sys.exit(main())
