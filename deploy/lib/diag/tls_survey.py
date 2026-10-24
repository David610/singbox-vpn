import socket, ssl, time, json, sys, threading, collections, queue

YT = ['www.youtube.com', 'i.ytimg.com', 'youtubei.googleapis.com', 'rr1---sn-aaaa.googlevideo.com']
CT = ['www.google.com', 'example.com', '']


def hs(ctx, ip, sni):
    try:
        s = socket.create_connection((ip, 443), 3.5)
        s.settimeout(4)
        w = ctx.wrap_socket(s, server_hostname=sni or None)
        w.close()
        return 'ok'
    except socket.timeout:
        return 'noreply'
    except ConnectionResetError:
        return 'reset'
    except ssl.SSLError:
        return 'sslerr'
    except OSError:
        return 'noconnect'


def survey(ips, workers=24):
    """TLS handshake every (ip, sni) pair; returns [[ip, sni, result], ...]. Certificates are NOT
    verified (this measures reachability and SNI filtering, not authenticity)."""
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    q = queue.Queue()
    res = []
    lk = threading.Lock()
    for ip in ips:
        for s in YT + CT:
            q.put((ip, s))

    def work():
        while True:
            try:
                ip, s = q.get_nowait()
            except queue.Empty:
                return
            r = hs(ctx, ip, s)
            with lk:
                res.append((ip, s, r))

    ts = [threading.Thread(target=work) for _ in range(workers)]
    [t.start() for t in ts]
    [t.join() for t in ts]
    return [list(r) for r in res]


def render(res):
    rng = lambda ip: '.'.join(ip.split('.')[:2])
    tab = collections.defaultdict(lambda: collections.Counter())
    for ip, s, r in res:
        cls = 'YOUTUBE-SNI' if s in YT else 'control'
        tab[(rng(ip), cls)][r] += 1
    lines = ['%-10s %-12s %s' % ('range', 'class', 'results')]
    for k in sorted(tab):
        lines.append('%-10s %-12s %s' % (k[0], k[1], dict(tab[k])))
    tot = collections.Counter()
    [tot.update({('YT' if s in YT else 'CT', r): 1}) for ip, s, r in res]
    lines.append('TOTAL %s' % dict(tot))
    return '\n'.join(lines)


def main(argv):
    """tls_survey.py IPS_JSON LABEL [OUT_JSON]  (OUT_JSON defaults to /tmp/tls_survey_LABEL.json, the old path)."""
    if len(argv) < 3:
        sys.stderr.write(main.__doc__ + '\n')
        return 64
    with open(argv[1], encoding='utf-8') as fh:
        ips = json.load(fh)
    res = survey(ips)
    print(render(res))
    out = argv[3] if len(argv) > 3 else '/tmp/tls_survey_%s.json' % argv[2]
    with open(out, 'w', encoding='utf-8') as fh:
        json.dump(res, fh)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
