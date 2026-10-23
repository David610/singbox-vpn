// New agent (#172+#186) + new server on a DB WITHOUT the new migration: does the 503 from /events slow revocation?
import * as L from "./lib.mjs"; import { startAgent } from "./agent.mjs";
const { sleep, sql } = L; const api = L.startApi(18981); await L.ready(api);
const f = await L.fixture("s8", 2);
const reqs = () => api.stderr().split("\n").filter((l) => l.includes('"msg":"request"')).map((l) => { try { return JSON.parse(l); } catch { return null; } }).filter(Boolean);
const tally = () => { const t = {}; for (const r of reqs()) { const k = `${r.route} ${r.status}`; t[k] = (t[k] || 0) + 1; } return t; };
const ag = startAgent("new", { url: api.base, node: f.node, key: f.raw, dir: "/root/integ/run/s8" });
const waitFor = async (p, ms) => { const t = Date.now(); while (Date.now() - t < ms) { if (p()) return Date.now() - t; await sleep(100); } return null; };
const first = await waitFor(() => (ag.applied().at(-1)?.authorizations ?? []).length === 2, 30000); await sleep(8000);
const before = ag.applied().length;
sql(`update public.external_vpn_devices set revoked_at=now() where device_id='${f.devices[0].device}'`);
sql(`select public.revoke_external_vpn_device('${f.devices[0].device}','${f.account}')`);
const rev = await waitFor(() => ag.applied().length > before && (ag.applied().at(-1)?.authorizations ?? []).some((x) => x.principal_id === f.devices[0].principal && x.revoked), 60000);
console.log(JSON.stringify({ first_apply_ms: first, revoke_latency_ms: rev, requests: tally(), total: reqs().length }, null, 1)); ag.stop(); api.stop();
