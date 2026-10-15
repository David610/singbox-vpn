import sys, json
d = json.load(open(sys.argv[1]))
print('label', d['label'], '| probes', d.get('probe_total'), '| failed probes', len(d.get('probe_failures', [])), '| max probe latency s', d.get('probe_max_latency_s'))
for o in d['ops']:
    print(f"{o['t_start']:6.1f}s  {o['op']:<46} rc={o['rc']} {o['cmd_seconds']:>5}s  restarted={str(o['restarted']):<5} in-flight survived={o.get('inflight_transfers_survived')}  {o['note']}")
print('probe failures:', d.get('probe_failures'))
