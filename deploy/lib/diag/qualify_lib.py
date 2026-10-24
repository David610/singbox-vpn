#!/usr/bin/env python3
"""Evidence analysis and small helpers for candidate_qualify.sh (stdlib only).

Subcommands
  free-port                       print a currently free loopback TCP port
  merge-ips OUT IN [IN...]        union of IPv4 lists (JSON arrays, or objects with an "ips" array)
  socks-check --port P --expect-ip IP [--deadline S] [--pid N] [--host H] [--http-port N]
                                  wait for a SOCKS5 proxy and prove WHERE it exits
                                  (0 ready and exits at IP, 3 exits elsewhere, 4 not ready in time)
  verdict --dir D                 classify the evidence in D, write D/verdict.json
                                  (exit 0 PASS, 1 FAIL, 2 INCONCLUSIVE)
  network-verdict --dir D         classify only the TLS survey (0 continue, 1 TLS blocked, 2 not conclusive)
  get FILE KEY                    print one scalar from a JSON object (empty if absent)
  set-config FILE KEY VALUE       set one string field in a JSON object

Verdict classes (see deploy/lib/diag/README.md):
  PASS          every required measurement ran, was conclusive, and met its threshold
  FAIL          conclusive NEGATIVE evidence: media refused, bot wall, stalled, no start, or YouTube
                TLS blocked while controls work
  INCONCLUSIVE  anything that was not (fully) measured: a missing prerequisite, an unverified host,
                a crashed/timed-out/incomplete probe, a tunnel that does not exit where expected,
                an interrupted run, or a page that left /shorts/

A PASS describes only the measured workload on one run; it never certifies a network permanently.
"""
import argparse
import collections
import datetime
import json
import os
import socket
import sys
import time

SCHEMA_VERSION = 1
EXIT = {"PASS": 0, "FAIL": 1, "INCONCLUSIVE": 2}

# Stricter than "a request succeeded": a Short counts only if the clock advanced for most of the dwell
# and never stalled for long (same bar as shorts_media_probe.py's PLAYED).
PLAY_FRACTION = 0.6
MAX_STALL_S = 4.0
MIN_CONTROL_TLS_OK = 0.90
MIN_YOUTUBE_TLS_OK = 0.95
YOUTUBE_SNI = ("www.youtube.com", "i.ytimg.com", "youtubei.googleapis.com", "rr1---sn-aaaa.googlevideo.com")
CONTROL_SNI = ("www.google.com", "example.com", "")

SCOPE_NOTE = (
    "Measured on this run only, over this egress IP at this time, with a headless browser over a plain "
    "TCP SOCKS tunnel (no QUIC, no native app). A PASS is not a permanent or native-device guarantee; "
    "DEVICE_VERIFIED needs the native matrix in docs/DEVICE_ACCEPTANCE_TESTS.md."
)


# ---------------------------------------------------------------- helpers
def read_json(path):
    """Return (status, value): ok / missing / unreadable."""
    if not os.path.isfile(path):
        return "missing", None
    try:
        with open(path, encoding="utf-8") as fh:
            return "ok", json.load(fh)
    except (OSError, ValueError):
        return "unreadable", None


def read_stages(path):
    stages = []
    if not os.path.isfile(path):
        return stages
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            parts = line.rstrip("\n").split("\t")
            if len(parts) >= 2:
                stages.append({"stage": parts[0], "status": parts[1], "code": parts[2] if len(parts) > 2 else ""})
    return stages


# ---------------------------------------------------------------- classification
def classify_row(row, dwell, min_play_s=None, require_short=True):
    """One probe row -> (outcome, code). outcome: played | negative | unknown."""
    if not isinstance(row, dict):
        return "unknown", "evidence_unreadable"
    if row.get("err"):
        return "unknown", "probe_error"
    if require_short and not row.get("still_on_shorts", False):
        return "unknown", "not_a_short"
    if row.get("bot_challenge"):
        return "negative", "bot_challenge"
    need = min_play_s if min_play_s is not None else PLAY_FRACTION * dwell
    started = bool(row.get("started"))
    played = float(row.get("played_s") or 0)
    stall = float(row.get("max_stall_s") or 0)
    if started and played >= need and stall < MAX_STALL_S:
        return "played", "played"
    if started:
        return "negative", "stalled"
    if row.get("unavailable") or int(row.get("refusal_155B_like") or 0) > 0:
        return "negative", "media_refused"
    return "negative", "no_start"


def analyse_rows(path, expected, dwell, min_play_s=None, require_short=True):
    """Summarise one probe result file against the number of rows requested."""
    state, data = read_json(path)
    summary = {"file": os.path.basename(path), "expected": expected, "rows": 0, "played": 0,
               "negative": {}, "unknown": {}, "evidence": state}
    if state == "missing":
        summary["unknown"]["evidence_missing"] = expected
        return summary
    if state == "unreadable" or not isinstance(data, list):
        summary["evidence"] = "unreadable"
        summary["unknown"]["evidence_unreadable"] = expected
        return summary
    summary["rows"] = len(data)
    for row in data[:expected]:
        outcome, code = classify_row(row, dwell, min_play_s, require_short)
        if outcome == "played":
            summary["played"] += 1
        elif outcome == "negative":
            summary["negative"][code] = summary["negative"].get(code, 0) + 1
        else:
            summary["unknown"][code] = summary["unknown"].get(code, 0) + 1
    missing = expected - min(len(data), expected)
    if missing:
        summary["unknown"]["incomplete_probe"] = missing
    return summary


def analyse_tls(path):
    state, doc = read_json(path)
    out = {"evidence": state}
    if state != "ok" or not isinstance(doc, dict):
        out["evidence"] = "missing" if state == "missing" else "unreadable"
        return out
    rows = (doc.get("tls") or {}).get("rows")
    if not isinstance(rows, list) or not rows:
        out["evidence"] = "unreadable"
        return out
    yt = [r for r in rows if isinstance(r, list) and len(r) == 3 and r[1] in YOUTUBE_SNI]
    ct = [r for r in rows if isinstance(r, list) and len(r) == 3 and r[1] in CONTROL_SNI]
    ratio = lambda xs: (sum(1 for r in xs if r[2] == "ok") / len(xs)) if xs else None
    out.update({"youtube_sni_probes": len(yt), "control_probes": len(ct),
                "youtube_sni_ok_ratio": None if ratio(yt) is None else round(ratio(yt), 3),
                "control_ok_ratio": None if ratio(ct) is None else round(ratio(ct), 3)})
    return out


def build_verdict(directory):
    cfg_state, cfg = read_json(os.path.join(directory, "config.json"))
    cfg = cfg if cfg_state == "ok" and isinstance(cfg, dict) else {}
    stages = read_stages(os.path.join(directory, "stages.tsv"))
    ids = cfg.get("shorts_ids") or []
    dwell = float(cfg.get("dwell") or 25)
    min_played = int(cfg.get("min_shorts_played") or len(ids) or 0)
    rounds = int(cfg.get("rounds") or 1)
    sustained_id = cfg.get("sustained_video") or ""
    sustained_dwell = float(cfg.get("sustained_dwell") or 900)
    min_sustained = float(cfg.get("min_sustained_s") or 600)
    identity = cfg.get("host_identity") or "unverified"

    negatives = []      # (code, where) conclusive negative evidence
    unknowns = []       # (code, where) not measured / not conclusive
    sections = {}

    # --- execution outcome of each stage, in the order it happened
    for st in stages:
        if st["status"] not in ("ok", "skipped"):
            unknowns.append((st["code"] or "stage_failed", st["stage"]))

    # --- network stage
    survey_failed = any(s["stage"] in ("survey", "collect") and s["status"] not in ("ok", "skipped") for s in stages)
    tls = None
    if not survey_failed and any(s["stage"] == "survey" and s["status"] == "ok" for s in stages):
        tls = analyse_tls(os.path.join(directory, "survey.json"))
        sections["tls"] = tls
        if tls["evidence"] != "ok":
            unknowns.append(("evidence_" + tls["evidence"], "survey"))
        elif tls.get("control_ok_ratio") is None or tls["control_ok_ratio"] < MIN_CONTROL_TLS_OK:
            unknowns.append(("network_unhealthy", "survey"))
        elif tls.get("youtube_sni_ok_ratio") is None or tls["youtube_sni_ok_ratio"] < MIN_YOUTUBE_TLS_OK:
            negatives.append(("tls_blocked", "survey"))
    collect_state, collect = read_json(os.path.join(directory, "collect.json"))
    if collect_state == "ok" and isinstance(collect, dict):
        sections["egress_ipv4"] = collect.get("egress_ipv4")

    # --- media rounds
    round_summaries = []
    for n in range(1, rounds + 1):
        stage = "shorts_round_%d" % n
        ran = [s for s in stages if s["stage"] == stage]
        if not ran:
            continue
        if ran[-1]["status"] != "ok":
            # the stage failure is already an unknown; a partial file must not upgrade it
            part = analyse_rows(os.path.join(directory, "round_%d.json" % n), len(ids), dwell)
            if part["evidence"] == "ok":
                for code, cnt in part["negative"].items():
                    negatives.extend([(code, stage)] * cnt)
            round_summaries.append(dict(part, round=n, executed=ran[-1]["status"]))
            continue
        summary = analyse_rows(os.path.join(directory, "round_%d.json" % n), len(ids), dwell)
        summary["round"] = n
        summary["required_played"] = min_played
        unknown_total = sum(summary["unknown"].values())
        neg_total = sum(summary["negative"].values())
        for code, cnt in summary["negative"].items():
            if summary["played"] + unknown_total < min_played:
                negatives.extend([(code, stage)] * cnt)
        if summary["played"] < min_played:
            if summary["played"] + unknown_total >= min_played or neg_total == 0:
                for code, cnt in summary["unknown"].items():
                    unknowns.extend([(code, stage)] * cnt)
            else:
                pass  # conclusive: even if every unknown had played, the bar is unreachable
        round_summaries.append(summary)
    if round_summaries:
        sections["shorts_rounds"] = round_summaries

    # --- sustained long-form playback
    sustained_ran = [s for s in stages if s["stage"] == "sustained"]
    if sustained_id and sustained_ran:
        summary = analyse_rows(os.path.join(directory, "sustained.json"), 1, sustained_dwell,
                               min_play_s=min_sustained, require_short=False)
        sections["sustained"] = dict(summary, required_seconds=min_sustained)
        if sustained_ran[-1]["status"] == "ok" and summary["played"] < 1:
            for code, cnt in summary["negative"].items():
                negatives.extend([(code, "sustained")] * cnt)
            for code, cnt in summary["unknown"].items():
                unknowns.extend([(code, "sustained")] * cnt)
        elif sustained_ran[-1]["status"] != "ok":
            for code, cnt in summary["negative"].items():
                negatives.extend([(code, "sustained")] * cnt)

    # --- precedence: conclusive negatives > anything unmeasured > identity > sustained > PASS
    verdict, reasons = "PASS", []
    if negatives:
        verdict = "FAIL"
        reasons = list(collections.OrderedDict.fromkeys(c for c, _ in negatives))
    elif unknowns:
        verdict = "INCONCLUSIVE"
        reasons = list(collections.OrderedDict.fromkeys(c for c, _ in unknowns))
    elif not stages or not any(s["stage"].startswith("shorts_round_") for s in stages):
        verdict, reasons = "INCONCLUSIVE", ["no_media_evidence"]
    elif identity != "verified":
        verdict, reasons = "INCONCLUSIVE", ["host_identity_unverified"]
    elif not sustained_id or not sustained_ran:
        verdict, reasons = "INCONCLUSIVE", ["sustained_not_run"]

    return {
        "schema_version": SCHEMA_VERSION,
        "verdict": verdict,
        "reason": reasons[0] if reasons else "all_required_measurements_passed",
        "reasons": reasons,
        "label": cfg.get("label", ""),
        "target_host": cfg.get("target_host", ""),
        "utc": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "host_identity": identity,
        "workload": {"shorts_ids": len(ids), "dwell_s": dwell, "min_shorts_played": min_played, "rounds": rounds,
                     "sustained_video": bool(sustained_id), "sustained_dwell_s": sustained_dwell,
                     "min_sustained_s": min_sustained},
        "stages": stages,
        "evidence": sections,
        "scope": SCOPE_NOTE,
    }


# ---------------------------------------------------------------- subcommands
def cmd_free_port(_):
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        print(s.getsockname()[1])
    return 0


def cmd_merge_ips(args):
    ips = set()
    for path in args.inputs:
        state, data = read_json(path)
        if state != "ok":
            print("merge-ips: cannot read %s (%s)" % (path, state), file=sys.stderr)
            return 1
        items = data.get("ips") if isinstance(data, dict) else data
        if not isinstance(items, list):
            print("merge-ips: %s is not a list of addresses" % path, file=sys.stderr)
            return 1
        for ip in items:
            try:
                socket.inet_pton(socket.AF_INET, ip)
            except (OSError, TypeError):
                print("merge-ips: %r in %s is not an IPv4 address" % (ip, path), file=sys.stderr)
                return 1
            ips.add(ip)
    if not ips:
        print("merge-ips: no addresses collected", file=sys.stderr)
        return 1
    with open(args.out, "w", encoding="utf-8") as fh:
        json.dump(sorted(ips), fh)
    return 0


def socks_exit_ip(port, host, http_port, timeout=6.0):
    """Ask host through SOCKS5 (remote DNS) and return the body of GET /."""
    with socket.create_connection(("127.0.0.1", port), timeout=timeout) as s:
        s.settimeout(timeout)
        s.sendall(b"\x05\x01\x00")
        if s.recv(2) != b"\x05\x00":
            raise OSError("socks greeting refused")
        name = host.encode("idna")
        s.sendall(b"\x05\x01\x00\x03" + bytes([len(name)]) + name + http_port.to_bytes(2, "big"))
        reply = s.recv(10)
        if len(reply) < 2 or reply[1] != 0:
            raise OSError("socks connect refused")
        s.sendall(("GET / HTTP/1.0\r\nHost: %s\r\nUser-Agent: candidate-qualify\r\n\r\n" % host).encode())
        buf = b""
        while True:
            chunk = s.recv(4096)
            if not chunk:
                break
            buf += chunk
            if len(buf) > 65536:
                break
    body = buf.split(b"\r\n\r\n", 1)[-1].decode("ascii", "replace").strip()
    return body


def cmd_socks_check(args):
    deadline = time.time() + args.deadline
    last = "not reachable"
    while time.time() < deadline:
        if args.pid:
            try:
                os.kill(args.pid, 0)
            except OSError:
                print("socks-check: tunnel process %d exited" % args.pid, file=sys.stderr)
                return 4
        try:
            seen = socks_exit_ip(args.port, args.host, args.http_port)
        except (OSError, ValueError) as exc:
            last = str(exc)
            time.sleep(0.5)
            continue
        if seen == args.expect_ip:
            return 0
        print("socks-check: tunnel exits at %r, expected %r" % (seen[:64], args.expect_ip), file=sys.stderr)
        return 3
    print("socks-check: tunnel not ready after %ss (%s)" % (args.deadline, last), file=sys.stderr)
    return 4


def cmd_get(args):
    state, data = read_json(args.file)
    value = data.get(args.key) if state == "ok" and isinstance(data, dict) else None
    print("" if value is None or isinstance(value, (dict, list)) else value)
    return 0


def cmd_set_config(args):
    state, data = read_json(args.file)
    if state != "ok" or not isinstance(data, dict):
        print("set-config: cannot read %s (%s)" % (args.file, state), file=sys.stderr)
        return 1
    data[args.key] = args.value
    tmp = args.file + ".tmp"
    with open(tmp, "w", encoding="utf-8") as fh:
        json.dump(data, fh)
    os.replace(tmp, args.file)
    return 0


def cmd_network_verdict(args):
    tls = analyse_tls(os.path.join(args.dir, "survey.json"))
    if tls["evidence"] != "ok":
        return 2
    if tls.get("control_ok_ratio") is None or tls["control_ok_ratio"] < MIN_CONTROL_TLS_OK:
        return 2
    if tls.get("youtube_sni_ok_ratio") is None or tls["youtube_sni_ok_ratio"] < MIN_YOUTUBE_TLS_OK:
        return 1
    return 0


def cmd_verdict(args):
    verdict = build_verdict(args.dir)
    path = os.path.join(args.dir, "verdict.json")
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as fh:
        json.dump(verdict, fh, indent=2, sort_keys=False)
        fh.write("\n")
    os.replace(tmp, path)
    print("%s %s" % (verdict["verdict"], verdict["reason"]))
    return EXIT[verdict["verdict"]]


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("free-port").set_defaults(fn=cmd_free_port)
    m = sub.add_parser("merge-ips")
    m.add_argument("out")
    m.add_argument("inputs", nargs="+")
    m.set_defaults(fn=cmd_merge_ips)
    s = sub.add_parser("socks-check")
    s.add_argument("--port", type=int, required=True)
    s.add_argument("--expect-ip", required=True)
    s.add_argument("--deadline", type=float, default=25.0)
    s.add_argument("--pid", type=int, default=0)
    s.add_argument("--host", default="api.ipify.org")
    s.add_argument("--http-port", type=int, default=80)
    s.set_defaults(fn=cmd_socks_check)
    g = sub.add_parser("get")
    g.add_argument("file")
    g.add_argument("key")
    g.set_defaults(fn=cmd_get)
    sc = sub.add_parser("set-config")
    sc.add_argument("file")
    sc.add_argument("key")
    sc.add_argument("value")
    sc.set_defaults(fn=cmd_set_config)
    nv = sub.add_parser("network-verdict")
    nv.add_argument("--dir", required=True)
    nv.set_defaults(fn=cmd_network_verdict)
    v = sub.add_parser("verdict")
    v.add_argument("--dir", required=True)
    v.set_defaults(fn=cmd_verdict)
    args = ap.parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main())
