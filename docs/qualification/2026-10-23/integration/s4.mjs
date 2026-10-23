import * as L from "./lib.mjs";
import { startAgent } from "./agent.mjs";
import { execSync } from "node:child_process";
const [agentKind, serverKind] = process.argv.slice(2);
const { sleep, sql, get } = L;
const cwd = serverKind === "old" ? "/root/vpn-web-old" : "/root/vpn-web";
const port = 18900 + Math.floor(Math.random() * 90);
const api = L.startApi(port, {}, cwd); await L.ready(api);
const f = await L.fixture(`a${agentKind[0]}${serverKind[0]}`, 2);
const reqs = () => api.stderr().split("\n").filter((l) => l.includes('"msg":"request"')).map((l) => { try { return JSON.parse(l); } catch { return null; } }).filter(Boolean);
const tally = (from = 0) => { const t = {}; for (const r of reqs().slice(from)) { const k = `${r.route} ${r.status}`; t[k] = (t[k] || 0) + 1; } return t; };
const ag = startAgent(agentKind, { url: api.base, node: f.node, key: f.raw, dir: `/root/integ/run/${agentKind}-${serverKind}` });
const out = { agent: agentKind, server: serverKind };
const waitFor = async (pred, ms) => { const t = Date.now(); while (Date.now() - t < ms) { if (pred()) return Date.now() - t; await sleep(100); } return null; };
const revokedIn = (principal) => () => { const a = ag.applied().at(-1); return a && (a.authorizations ?? []).some((x) => x.principal_id === principal && x.revoked === true); };
const liveCount = () => { const a = ag.applied().at(-1); return a ? (a.authorizations ?? []).filter((x) => !x.revoked).length : -1; };

out.first_apply_ms = await waitFor(() => liveCount() === 2, 25000);
await sleep(2000); let mark = reqs().length; await sleep(30000);
out.idle_30s = tally(mark); out.idle_requests_per_hour_projected = Math.round((reqs().length - mark) * 120);
const t0 = Date.now(); sql(`update public.external_vpn_devices set revoked_at=now() where device_id='${f.devices[0].device}'`);
out.revoke_latency_ms = await waitFor(revokedIn(f.devices[0].principal), 70000);
out.live_after_revoke = liveCount();
// outage: database down for 25 s
mark = reqs().length; execSync("pkill -x postgrest || true"); await sleep(25000);
out.outage_25s = tally(mark); out.outage_requests = reqs().length - mark;
execSync("cd /root/integ && (nohup /tmp/pgrst/postgrest pgrst.conf >pgrst.log 2>&1 &)"); await sleep(1500);
// revocation issued after recovery must still arrive
sql(`update public.external_vpn_devices set revoked_at=now() where device_id='${f.devices[1].device}'`);
out.post_outage_revoke_latency_ms = await waitFor(revokedIn(f.devices[1].principal), 90000);
out.live_final = liveCount();
out.agent_alive = ag.child.exitCode === null;
out.agent_errors = (ag.log().match(/ERROR|panicked/g) || []).length;
out.agent_log_tail = ag.log().split("\n").filter((l) => /WARN|ERROR|panic/.test(l)).slice(-4).map((l) => l.replace(/\x1b\[[0-9;]*m/g, "").slice(0, 200));
console.log(JSON.stringify(out, null, 1));
ag.stop(); api.stop();
