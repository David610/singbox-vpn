#!/usr/bin/env node
// Stand-in for vpn-web's agent API for the Phase 4 protocol-health real
// test. It is NOT a reimplementation of the decision logic: it imports
// vpn-web's own functions/lib modules (protocol-health.js,
// protocol-health-store.js, node-health-transition.js, node-lifecycle.js)
// and runs them against an in-memory `nodes` / `node_probe_results` /
// `node_probe_credentials` store shaped like the Supabase client they
// expect. Only the heartbeat handler's glue is mirrored here (same order:
// protocol report first, then the node's own evaluation + recovery gate,
// then silence detection).
//
// Usage: VW_LIB=/path/to/vpn-web/functions/lib PORT=18788 node mockcp.mjs
// Nodes are seeded from NODES="id=ip,id=ip" (bearer token == node id; test
// only). Admin helpers: GET /state, POST /admin/set {node_id, lifecycle_state, failed_reason}.
// Probe credentials are held in memory only and never printed.
import http from "node:http";
import fs from "node:fs";

const LIB = process.env.VW_LIB;
const { sanitizeProtocolReport, protocolAllowsRecovery, choosePeers } = await import(`${LIB}/protocol-health.js`);
const { applyProtocolReport } = await import(`${LIB}/protocol-health-store.js`);
const { evaluateProbeResult, isNodeSilent, HEARTBEAT_INTERVAL_MS } = await import(`${LIB}/node-health-transition.js`);
const { canTransitionLifecycle } = await import(`${LIB}/node-lifecycle.js`);

const LOG = process.env.LOG ?? "./mockcp-events.log";
const nodes = {};
for (const pair of (process.env.NODES ?? "").split(",").filter(Boolean)) {
  const [id, ip] = pair.split("=");
  nodes[id] = {
    node_id: id, ip_address: ip, lifecycle_state: "READY", failed_reason: null, last_seen_at: null,
    consecutive_probe_failures: 0, consecutive_probe_successes: 0,
    protocol_probe_failures: 0, protocol_probe_successes: 0, last_peer_probe_at: null,
    protocol_health: null, hysteria2_cert_days: null,
  };
}
const results = [];
const creds = {};
const log = (msg) => {
  const line = `${new Date().toISOString()} ${msg}`;
  fs.appendFileSync(LOG, line + "\n");
  console.log(line);
};
function setState(id, to, reason, why) {
  const n = nodes[id];
  if (!n || n.lifecycle_state === to) return;
  log(`TRANSITION ${id} ${n.lifecycle_state} -> ${to}${reason ? ` (failed_reason=${reason})` : ""} [${why}]`);
  n.lifecycle_state = to;
  n.failed_reason = reason ?? null;
}

// Minimal Supabase-shaped client over the in-memory tables.
const supabase = {
  from(table) {
    if (table === "node_probe_results") {
      return { insert: async (rows) => (results.push(...rows), { error: null }) };
    }
    return {
      select: () => {
        const f = [];
        const q = {
          eq: (c, v) => (f.push([c, v]), q),
          maybeSingle: async () => ({ data: Object.values(nodes).find((n) => f.every(([c, v]) => n[c] === v)) ?? null, error: null }),
        };
        return q;
      },
      update: (patch) => {
        const f = [];
        const apply = () => {
          const n = Object.values(nodes).find((x) => f.every(([c, v]) => x[c] === v));
          if (!n) return null;
          if ("lifecycle_state" in patch) {
            setState(n.node_id, patch.lifecycle_state, patch.failed_reason, "protocol-health-store CAS");
            const { lifecycle_state, failed_reason, ...rest } = patch; // eslint-disable-line no-unused-vars
            Object.assign(n, rest);
          } else Object.assign(n, patch);
          return n;
        };
        const chain = {
          eq: (c, v) => (f.push([c, v]), chain),
          select: () => chain,
          maybeSingle: async () => ({ data: apply(), error: null }),
          then: (r) => (apply(), r({ error: null })),
        };
        return chain;
      },
    };
  },
  rpc: async (name, { p_target_node_id, p_keep_hours, p_max_rows }) => {
    const cutoff = Date.now() - p_keep_hours * 3600_000;
    let kept = 0;
    for (let i = results.length - 1; i >= 0; i--) {
      const r = results[i];
      if (r.target_node_id !== p_target_node_id) continue;
      if (Date.parse(r.observed_at) < cutoff || ++kept > p_max_rows) results.splice(i, 1);
    }
    return { error: null };
  },
};

async function heartbeat(nodeId, body) {
  const autoHealth = true;
  const probeOk = typeof body.probe_ok === "boolean" ? body.probe_ok : null;
  const node = nodes[nodeId];
  node.last_seen_at = new Date().toISOString();
  const report = sanitizeProtocolReport(body.protocol_probe);
  if (report) {
    if (report.certDays !== null) node.hysteria2_cert_days = report.certDays;
    await applyProtocolReport({ supabase, reporterNodeId: nodeId, report, autoHealth });
    const summary = report.results
      .map((r) => `${r.targetNodeId}/${r.vantage}/${r.protocol}=${r.ok ? "ok" : `FAIL(${r.error})`} tcp=${r.dims.tcp_connect} dns=${r.dims.dns} v4=${r.dims.https_ipv4} v6=${r.dims.ipv6}(${r.dims.egress_ipv6}) egress=${r.dims.egress_ipv4} match=${r.dims.egress_ip_match} lat=${r.latencyMs} loss=${r.lossPct}`)
      .join(" | ");
    log(`REPORT from ${nodeId} cert_days=${report.certDays} :: ${summary}`);
  }
  const e = evaluateProbeResult({
    probeOk,
    currentFailures: node.consecutive_probe_failures,
    currentSuccesses: node.consecutive_probe_successes,
    lifecycleState: node.lifecycle_state,
    failedReason: node.failed_reason,
  });
  node.consecutive_probe_failures = e.failures;
  node.consecutive_probe_successes = e.successes;
  const blocked = e.nextState === "READY" && node.lifecycle_state === "DEGRADED" && !protocolAllowsRecovery(node);
  if (e.nextState && !blocked && canTransitionLifecycle(node.lifecycle_state, e.nextState)) {
    setState(nodeId, e.nextState, null, "own heartbeat (evaluateProbeResult)");
  }
  for (const other of Object.values(nodes)) {
    if (other.node_id !== nodeId && isNodeSilent(other, Date.now(), HEARTBEAT_INTERVAL_MS)) {
      setState(other.node_id, "FAILED", "SILENCE", "silence");
    }
  }
}

const reply = (res, obj, code = 200) => {
  const b = JSON.stringify(obj);
  res.writeHead(code, { "Content-Type": "application/json", "Content-Length": Buffer.byteLength(b) });
  res.end(b);
};

http
  .createServer(async (req, res) => {
    let raw = "";
    for await (const c of req) raw += c;
    const body = raw ? JSON.parse(raw) : {};
    const nodeId = (req.headers.authorization ?? "").replace(/^Bearer /, "");
    if (req.url === "/state") {
      const view = Object.values(nodes).map(({ node_id, lifecycle_state, failed_reason, protocol_probe_failures, protocol_probe_successes, last_seen_at, hysteria2_cert_days, protocol_health }) => ({
        node_id, lifecycle_state, failed_reason, protocol_probe_failures, protocol_probe_successes, last_seen_at, hysteria2_cert_days, protocol_health,
      }));
      return reply(res, { nodes: view, probe_rows: results.length, credentials_published: Object.keys(creds) });
    }
    if (req.url === "/admin/set") {
      setState(body.node_id, body.lifecycle_state, body.failed_reason, "admin");
      return reply(res, { ok: true });
    }
    if (!nodes[nodeId]) return reply(res, { error: "Unauthorized" }, 401);
    switch (req.url) {
      case "/api/agent/claim":
        return reply(res, { job: null });
      case "/api/agent/heartbeat":
        await heartbeat(nodeId, body);
        return reply(res, { ok: true, node_id: nodeId });
      case "/api/agent/probe-credential":
        creds[nodeId] = { reality_uri: body.reality_uri ?? null, hysteria2_uri: body.hysteria2_uri ?? null };
        log(`CREDENTIAL published by ${nodeId} (reality=${!!body.reality_uri} hysteria2=${!!body.hysteria2_uri})`);
        return reply(res, { ok: true });
      case "/api/agent/probe-targets": {
        const peers = Object.values(nodes).filter((n) => n.node_id !== nodeId && ["WARMING_UP", "READY", "DEGRADED"].includes(n.lifecycle_state) && creds[n.node_id]);
        return reply(res, {
          targets: choosePeers(nodeId, peers, Date.now()).map((p) => ({ node_id: p.node_id, expected_ipv4: p.ip_address, ...creds[p.node_id] })),
        });
      }
      default:
        return reply(res, {});
    }
  })
  .listen(Number(process.env.PORT ?? 18788), "127.0.0.1", () => log(`mockcp listening, nodes=${Object.keys(nodes).join(",")}`));
