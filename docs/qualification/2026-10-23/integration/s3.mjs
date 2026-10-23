import * as L from "./lib.mjs";
import { execSync } from "node:child_process";
const { check, assert, get, sleep } = L;
const a = L.startApi(18831); await L.ready(a);
const f = await L.fixture("s3", 1);
const ok = await get(a, "/api/agent/authorizations?schema=2", f.raw); assert(ok.status === 200, "baseline");
execSync("pkill -x postgrest || true"); await sleep(500);
await check("DB outage: /authorizations must not answer 401 (a credential verdict) for an infrastructure fault", async () => {
  await sleep(1100); // let the 1s auth cache expire
  const r = await get(a, "/api/agent/authorizations?schema=2", f.raw);
  assert(r.status === 503 && r.headers.get("retry-after"), `got ${r.status}`); return `status=${r.status} retry-after=${r.headers.get("retry-after")}`;
});
await check("DB outage: /events status", async () => {
  const r = await get(a, "/api/agent/events?wait=1", f.raw); assert(r.status === 503, `got ${r.status}`); return `status=${r.status} retry-after=${r.headers.get("retry-after")}`;
});
await check("DB outage: /healthz", async () => { const r = await fetch(a.base + "/healthz"); return `status=${r.status}`; });
execSync("cd /root/integ && (nohup /tmp/pgrst/postgrest pgrst.conf >pgrst.log 2>&1 &)"); await sleep(2500);
await check("recovery: authorizations 200 again after DB returns", async () => { await sleep(1200); const r = await get(a, "/api/agent/authorizations?schema=2", f.raw); assert(r.status === 200, "status " + r.status); });
console.log("api exit:", a.child.exitCode, a.child.signalCode, a.stderr().split("\n").filter(l=>!/Warning|Reparsing|type"|trace-warn|"request"/.test(l)).slice(-8).join("\n")); a.stop(); process.exit(L.summary() ? 1 : 0);
