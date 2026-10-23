#!/usr/bin/env python3
"""Advisory external-URL check for current (non-historical) Markdown docs.

Never part of a required check: the network is not deterministic. It classifies
each distinct URL so a human can act on real rot without false alarms:
  OK          2xx, or 3xx that resolves to 2xx (redirects are followed)
  MOVED       resolved, but only after a redirect to a different URL (update the link)
  RESTRICTED  401/403/429/999: the server answered, so the host is alive; access is limited
  DEAD        404/410 on both HEAD and GET, repeated
  TRANSIENT   timeouts, DNS/connection errors, 5xx after retries (UNVERIFIED, not dead)
Exit code is 0 always unless --fail-on-dead is given and a DEAD URL exists.
"""
import argparse, json, os, re, subprocess, sys, time, urllib.request, urllib.error

URL = re.compile(r"https?://[^\s)<>\]\"'`]+")
SKIP_HOSTS = ("example.com", "example.org", "example.net", "localhost", "127.0.0.1")
HISTORICAL = ("docs/archive/", "docs/handoffs/", "docs/qualification/", "docs/reviews/", "docs/incidents/", "docs/release-notes/", "docs/health-probe-evidence/", "docs/b2-evidence/", "docs/superpowers/", "TASKS.md")


def collect(root):
    out = subprocess.run(["git", "-C", root, "ls-files", "*.md"], capture_output=True, text=True, check=True).stdout.split()
    urls = {}
    for rel in out:
        if rel.startswith(HISTORICAL):
            continue
        for line in open(os.path.join(root, rel), encoding="utf-8", errors="replace"):
            for u in URL.findall(line):
                u = u.rstrip(".,;:'")
                host = re.sub(r"^https?://([^/:]+).*", r"\1", u)
                if not host.endswith(SKIP_HOSTS) and "<" not in u and "{" not in u and "$" not in u:
                    urls.setdefault(u, rel)
    return urls


def probe(url, attempts=3, timeout=15):
    last = None
    for i in range(attempts):
        for method in ("HEAD", "GET"):
            try:
                req = urllib.request.Request(url, method=method, headers={"User-Agent": "singbox-vpn-docs-check/1"})
                with urllib.request.urlopen(req, timeout=timeout) as r:
                    return ("MOVED" if r.geturl().rstrip("/") != url.rstrip("/") else "OK"), r.status, r.geturl()
            except urllib.error.HTTPError as e:
                last = ("HTTP", e.code)
                if e.code in (401, 403, 429, 999):
                    return "RESTRICTED", e.code, url
                if e.code in (404, 410) and method == "GET":
                    if e.code == 404 and re.match(r"^https://github\.com/[^/]+/[^/]+/?$", url):
                        return "RESTRICTED", "404 (private repo or gone)", url  # GitHub answers 404 for private repos
                    if i == attempts - 1:
                        return "DEAD", e.code, url
                if e.code >= 500:
                    break
            except ValueError:  # not a real URL (placeholder / literal-IPv6 example)
                return "TRANSIENT", ("UNPARSEABLE", 0), url
            except Exception as e:  # DNS, TLS, timeout, reset
                last = ("ERR", type(e).__name__)
                break
        time.sleep(2 ** i)
    return "TRANSIENT", last, url


def main():
    ap = argparse.ArgumentParser(); ap.add_argument("--root", default="."); ap.add_argument("--json", action="store_true"); ap.add_argument("--fail-on-dead", action="store_true")
    a = ap.parse_args()
    results = {}
    for url, rel in sorted(collect(os.path.abspath(a.root)).items()):
        results[url] = (rel,) + probe(url)
    if a.json:
        print(json.dumps({u: dict(zip(("file", "class", "detail", "final"), v)) for u, v in results.items()}, indent=1))
    else:
        for u, (rel, cls, detail, final) in results.items():
            print(f"{cls:10} {detail!s:22} {u}  ({rel})" + (f" -> {final}" if cls == "MOVED" else ""))
    dead = [u for u, v in results.items() if v[1] == "DEAD"]
    print(f"\n{len(results)} URLs: " + ", ".join(f"{c}={sum(1 for v in results.values() if v[1] == c)}" for c in ("OK", "MOVED", "RESTRICTED", "DEAD", "TRANSIENT")), file=sys.stderr)
    return 1 if (a.fail_on_dead and dead) else 0


if __name__ == "__main__":
    sys.exit(main())
