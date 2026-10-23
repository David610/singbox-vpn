import * as L from "./lib.mjs";
import { execSync } from "node:child_process";
const { get, sleep } = L;
const a = L.startApi(18841); await L.ready(a);
const f = await L.fixture("s3b", 1);
await get(a, "/api/agent/authorizations?schema=2", f.raw);
execSync("pkill -x postgrest || true"); await sleep(1500);
for (const p of ["/api/agent/authorizations?schema=2", "/api/agent/events?wait=1"]) {
  try { const r = await get(a, p, f.raw); console.log(p, r.status, r.headers.get("retry-after")); } catch (e) { console.log(p, "THROW", e.cause?.code || e.message); }
}
await sleep(500);
console.log("alive:", a.child.exitCode === null, "exit:", a.child.exitCode, a.child.signalCode);
console.log(a.stderr().split("\n").filter(l=>!/Warning|Reparsing|type"|trace-warn|"request"/.test(l)).slice(-12).join("\n"));
execSync("cd /root/integ && (nohup /tmp/pgrst/postgrest pgrst.conf >pgrst.log 2>&1 &)");
a.stop();
