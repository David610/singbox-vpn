#!/usr/bin/env python3
"""Stdlib-only TCP/UDP analysis of a header-only pcap (snaplen 96). Read-only.

usage: python3 -I pcap_analyze.py FILE.pcap [--flows] [--series]
Payload length is taken from the IP header, not captured bytes.
"""
import struct, sys, collections, socket

def read(path):
    with open(path, 'rb') as f:
        g = f.read(24)
        magic = struct.unpack('<I', g[:4])[0]
        if magic in (0xa1b2c3d4, 0xa1b23c4d):
            e = '<'
        else:
            e = '>'
        nano = magic in (0xa1b23c4d,) or struct.unpack('>I', g[:4])[0] == 0xa1b23c4d
        _, _, _, _, snap, link = struct.unpack(e + 'HHiIII', g[4:24])
        yield ('hdr', snap, link, nano)
        trunc_tail = False
        while True:
            h = f.read(16)
            if len(h) == 0:
                break
            if len(h) < 16:
                trunc_tail = True
                break
            ts, tf, incl, orig = struct.unpack(e + 'IIII', h)
            d = f.read(incl)
            if len(d) < incl:
                trunc_tail = True
                break
            yield ('pkt', ts + tf / (1e9 if nano else 1e6), incl, orig, d, link)
        yield ('end', trunc_tail)

def ip_of(d, link):
    off = {1: 14, 113: 16, 12: 0, 101: 0}.get(link, 14)
    if link == 1 and d[12:14] == b'\x81\x00':
        off = 18
    return d[off:]

def main():
    path = sys.argv[1]
    flows = collections.OrderedDict()
    ts_all = []
    snap = link = None
    trunc = False
    total = 0
    nontcp = collections.Counter()
    maxlen = 0
    for rec in read(path):
        if rec[0] == 'hdr':
            snap, link = rec[1], rec[2]
            continue
        if rec[0] == 'end':
            trunc = rec[1]
            break
        _, ts, incl, orig, d, lk = rec
        total += 1
        ts_all.append(ts)
        ip = ip_of(d, lk)
        if not ip or (ip[0] >> 4) != 4:
            nontcp['non-ipv4'] += 1
            continue
        ihl = (ip[0] & 15) * 4
        proto = ip[9]
        tl = struct.unpack('>H', ip[2:4])[0]
        src = socket.inet_ntoa(ip[12:16]); dst = socket.inet_ntoa(ip[16:20])
        maxlen = max(maxlen, tl)
        if proto == 17:
            nontcp['udp'] += 1
            continue
        if proto != 6:
            nontcp['proto%d' % proto] += 1
            continue
        t = ip[ihl:]
        sp, dp, seq, ack, off_fl, win = struct.unpack('>HHIIHH', t[:14] + t[14:16]) if False else (
            struct.unpack('>H', t[0:2])[0], struct.unpack('>H', t[2:4])[0],
            struct.unpack('>I', t[4:8])[0], struct.unpack('>I', t[8:12])[0],
            struct.unpack('>H', t[12:14])[0], struct.unpack('>H', t[14:16])[0])
        doff = (off_fl >> 12) * 4
        fl = off_fl & 0x3f
        plen = tl - ihl - doff
        # endpoint pair keyed by the ru1/fin1 client port
        if sp == 443:
            key = (dst, dp, src, sp); dirn = 'down'   # ru1 -> fin1
        else:
            key = (src, sp, dst, dp); dirn = 'up'
        F = flows.setdefault(key, dict(first=ts, last=ts, up=0, down=0, upp=0, downp=0, syn=False, synack=False,
                                        fin=0, rst_up=0, rst_down=0, w0_up=0, w0_down=0, retr_up=0, retr_down=0,
                                        dup_up=0, dup_down=0, ooo_down=0, maxseq={}, lastack={}, gaps=[], lastdata=None,
                                        mss=None, win_min={'up': 1 << 30, 'down': 1 << 30}))
        F['last'] = ts
        if fl & 2:
            if fl & 16: F['synack'] = True
            else: F['syn'] = True
            for i in range(20, doff):  # parse MSS
                pass
            o = t[20:doff]; i = 0
            while i < len(o):
                k = o[i]
                if k == 0: break
                if k == 1: i += 1; continue
                l = o[i + 1]
                if k == 2: F['mss'] = struct.unpack('>H', o[i + 2:i + 4])[0]
                i += l
        if fl & 1: F['fin'] += 1
        if fl & 4: F['rst_' + dirn] += 1
        wv = win
        if not (fl & 2) and win == 0 and not (fl & 4): F['w0_' + dirn] += 1
        if not (fl & 2): F['win_min'][dirn] = min(F['win_min'][dirn], win)
        if plen > 0:
            F[dirn] += plen; F[dirn + 'p'] += 1
            ms = F['maxseq'].get(dirn)
            if ms is not None and (seq + plen - ms) & 0xffffffff > 0x7fffffff and ((seq - ms) & 0xffffffff) > 0x7fffffff:
                F['retr_' + dirn] += 1
            F['maxseq'][dirn] = seq + plen if ms is None or ((seq + plen - ms) & 0xffffffff) < 0x7fffffff else ms
            if F['lastdata'] is not None and ts - F['lastdata'] > 2.0:
                F['gaps'].append((round(F['lastdata'] - F['first'], 2), round(ts - F['lastdata'], 2), dirn))
            F['lastdata'] = ts
        else:
            la = F['lastack'].get(dirn)
            if la == ack and not (fl & 3) and plen == 0:
                F['dup_' + dirn] += 1
            F['lastack'][dirn] = ack
    print('FILE', path)
    print('link-type', link, 'snaplen', snap, 'packets', total, 'tail-truncated', trunc)
    if ts_all:
        print('first %.3f last %.3f span %.1fs' % (ts_all[0], ts_all[-1], ts_all[-1] - ts_all[0]))
    print('non-tcp', dict(nontcp), 'max IP len', maxlen)
    up = sum(f['up'] for f in flows.values()); dn = sum(f['down'] for f in flows.values())
    print('tcp flows', len(flows), 'payload up(fin1->ru1) %d down(ru1->fin1) %d' % (up, dn))
    agg = collections.Counter()
    for f in flows.values():
        for k in ('w0_up', 'w0_down', 'rst_up', 'rst_down', 'retr_up', 'retr_down', 'dup_up', 'dup_down', 'fin'):
            agg[k] += f[k]
        agg['no_handshake'] += (not f['syn'])
    print('totals (up=fin1 sender, down=ru1 sender):', dict(agg))
    life = sorted(f['last'] - f['first'] for f in flows.values())
    if life:
        print('conn lifetime s: min %.2f med %.2f p90 %.2f max %.2f' % (life[0], life[len(life)//2], life[int(len(life)*.9)], life[-1]))
    if '--flows' in sys.argv:
        t0 = ts_all[0]
        for k, f in flows.items():
            print('flow fin1:%d  start+%.1fs life %.1fs up %d down %d w0u/d %d/%d rstu/d %d/%d retr u/d %d/%d gaps>2s %s' % (
                k[1], f['first'] - t0, f['last'] - f['first'], f['up'], f['down'], f['w0_up'], f['w0_down'],
                f['rst_up'], f['rst_down'], f['retr_up'], f['retr_down'], f['gaps'][:4]))
    if '--series' in sys.argv:
        t0 = ts_all[0]
        b = collections.defaultdict(lambda: [0, 0])
        # recompute per-second bytes quickly
        for rec in read(path):
            if rec[0] != 'pkt': continue
            _, ts, incl, orig, d, lk = rec
            ip = ip_of(d, lk)
            if not ip or (ip[0] >> 4) != 4 or ip[9] != 6: continue
            ihl = (ip[0] & 15) * 4; tl = struct.unpack('>H', ip[2:4])[0]
            t = ip[ihl:]; doff = ((struct.unpack('>H', t[12:14])[0]) >> 12) * 4
            pl = tl - ihl - doff
            sp = struct.unpack('>H', t[0:2])[0]
            b[int(ts - t0)][0 if sp == 443 else 1] += pl
        for s in range(0, int(ts_all[-1] - t0) + 1):
            print('+%3ds down %8d up %7d' % (s, b[s][0], b[s][1]))

main()
