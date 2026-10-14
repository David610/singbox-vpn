#!/usr/bin/env bash
# summarize.sh results.jsonl -> markdown tables (medians across repetitions)
set -euo pipefail
f="$1"
echo "| Path | RTT ms | Loss % | Download Mbit/s (median of reps) | Server CPU % |"
echo "|---|---:|---:|---:|---:|"
jq -s -r '
  map(select(.test == "tput1"))
  | group_by([.label, .rtt_ms, .loss_pct])
  | map({label: .[0].label, rtt: .[0].rtt_ms, loss: .[0].loss_pct,
         mbps: (map(.mbps) | sort | .[(length - 1) / 2 | floor]),
         reps: (map(.mbps) | map(tostring) | join("/")),
         cpu: (map(.server_cpu_pct) | sort | .[(length - 1) / 2 | floor])})
  | sort_by(.loss, .rtt, .label)
  | .[] | "| \(.label) | \(.rtt) | \(.loss) | \(.mbps) (\(.reps)) | \(.cpu) |"' "$f"
echo
echo "| Path | RTT ms | Loss % | New connection p50/p95 ms | Idle RTT p50 ms | Loaded RTT p50/p95/p99 ms | New UDP session p50 ms | UDP RTT p50 ms |"
echo "|---|---:|---:|---:|---:|---:|---:|---:|"
jq -s -r '
  map(select(.test != "tput1" and .test != "icmp"))
  | group_by([.name, .rtt_ms, .loss_pct])
  | map(. as $g | {name: $g[0].name, rtt: $g[0].rtt_ms, loss: $g[0].loss_pct,
      c: ($g | map(select(.test == "connlat"))[0]),
      i: ($g | map(select(.test == "rtt_idle"))[0]),
      l: ($g | map(select(.test == "rtt_loaded"))[0]),
      uf: ($g | map(select(.test == "udp_fresh_session"))[0]),
      ue: ($g | map(select(.test == "udp_established"))[0])})
  | sort_by(.loss, .rtt, .name)
  | def r: if . == null then "-" else (. * 10 | round / 10 | tostring) end;
    def fails(x): if x == null then "" elif x.failures > 0 then " (\(x.failures) failed)" else "" end;
    .[] | "| \(.name) | \(.rtt) | \(.loss) | \(.c.p50_ms | r)/\(.c.p95_ms | r)\(fails(.c)) | \(.i.p50_ms | r) | \(.l.p50_ms | r)/\(.l.p95_ms | r)/\(.l.p99_ms | r)\(fails(.l)) | \(.uf.p50_ms | r)\(fails(.uf)) | \(.ue.p50_ms | r)\(fails(.ue)) |"' "$f"
