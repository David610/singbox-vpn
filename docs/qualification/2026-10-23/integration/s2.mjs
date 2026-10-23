import * as L from "./lib.mjs";
const { check, assert, get, sql, sleep } = L;
const a = L.startApi(18811), b = L.startApi(18812);
await L.ready(a); await L.ready(b);
const f = await L.fixture("s2", 2);
const ev = (api, cursor, wait = 3, raw = f.raw) => get(api, `/api/agent/events?wait=${wait}${cursor ? "&cursor=" + encodeURIComponent(cursor) : ""}`, raw);
let cur;
await check("no cursor -> immediate resync with all channels + cursor", async () => {
  const r = await ev(a); assert(r.status === 200, r.status); const j = await r.json();
  assert(j.reason === "resync" && j.cursor && j.changed.length === 3, JSON.stringify(j)); cur = j.cursor;
});
await check("quiet node: long poll returns reason=timeout after ~wait with unchanged cursor", async () => {
  const t = Date.now(); const j = await (await ev(a, cur, 2)).json(); const dt = Date.now() - t;
  assert(j.reason === "timeout" && j.cursor === cur, JSON.stringify(j)); assert(dt >= 1800 && dt < 3500, "dt " + dt); return dt + " ms";
});
await check("garbage / oversized cursor -> resync, never an error or hang", async () => {
  for (const c of ["!!!", "a".repeat(600), Buffer.from("[1,2]").toString("base64url")]) {
    const r = await ev(b, c, 1); assert(r.status === 200, r.status); assert((await r.json()).reason === "resync", "no resync for " + c.slice(0, 8));
  }
});
await check("wait validation: non-numeric -> 400", async () => { assert((await ev(a, cur, "abc")).status === 400, "wait=abc"); });
await check("pending job wakes a waiting agent on BOTH instances (changed includes jobs)", async () => {
  for (const api of [a, b]) {
    const j0 = await (await ev(api)).json();
    const t = Date.now(); const p = ev(api, j0.cursor, 10);
    await sleep(300);
    sql(`insert into public.provisioning_jobs(idempotency_key,node_id,job_type,payload) values ('j-${Date.now()}-${api.port}','${f.node}','CREATE_USER','{}')`);
    const j = await (await p).json(); assert(j.reason === "change" && j.changed.includes("jobs"), JSON.stringify(j));
    assert(Date.now() - t < 4000, "slow wake " + (Date.now() - t));
    sql(`update public.provisioning_jobs set status='done' where node_id='${f.node}' and status='pending'`);
  }
});
let c2;
await check("revocation wakes the agent as 'authorizations' within the entitlement interval", async () => {
  const j0 = await (await ev(a)).json(); c2 = j0.cursor;
  const t = Date.now(); const p = ev(a, c2, 20); await sleep(300);
  sql(`update public.external_vpn_devices set revoked_at=now() where device_id='${f.devices[0].device}'`);
  const j = await (await p).json(); const dt = Date.now() - t;
  assert(j.reason === "change" && j.changed.includes("authorizations"), JSON.stringify(j)); assert(dt < 9000, "dt " + dt); return dt + " ms";
});
await check("MISSED event: stale cursor from before the revocation still reports the change (catch-up, no loss)", async () => {
  const j = await (await ev(b, c2, 1)).json();
  assert(j.reason === "change" && j.changed.includes("authorizations"), JSON.stringify(j));
});
await check("billing lapse (no compat row changes) wakes agent via entitlement token", async () => {
  const j0 = await (await ev(a)).json(); const t = Date.now(); const p = ev(a, j0.cursor, 20); await sleep(300);
  sql(`update public.subscriptions set status='canceled' where id=${f.sub}`);
  const j = await (await p).json(); assert(j.reason === "change", JSON.stringify(j)); return (Date.now() - t) + " ms";
});
await check("second concurrent waiter for the same node supersedes the first (409 + Retry-After)", async () => {
  const j0 = await (await ev(a)).json(); const p1 = ev(a, j0.cursor, 5); await sleep(300); const p2 = ev(a, j0.cursor, 1);
  const r1 = await p1; assert(r1.status === 409 && r1.headers.get("retry-after"), "status " + r1.status); await p2;
});
await check("node isolation: other node's events not triggered by this node's change", async () => {
  const g = await L.fixture("s2g", 1); const j0 = await (await ev(a, undefined, 1, g.raw)).json();
  const p = ev(a, j0.cursor, 3, g.raw); sql(`update public.subscriptions set status='active' where id=${f.sub}`);
  assert((await (await p).json()).reason === "timeout", "leaked change across nodes");
});
await check("revoked node key rejected within auth-cache TTL (NODE_AUTH_TTL_MS=1000)", async () => {
  const g = await L.fixture("s2r", 1); assert((await ev(a, undefined, 0, g.raw)).status === 200, "pre");
  sql(`update public.nodes set revoked_at=now() where node_id='${g.node}'`); await sleep(1300);
  assert((await ev(a, undefined, 0, g.raw)).status === 401, "revoked node still accepted");
});
console.log("STDERR_A tail:", a.stderr().split("\n").filter((l) => /error|Error|level\":\"error/.test(l)).slice(0, 5).join("\n"));
a.stop(); b.stop();
process.exit(L.summary() ? 1 : 0);
