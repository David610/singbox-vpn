import * as L from "./lib.mjs";
const { assert, get, sql, sleep } = L;
const b = L.startApi(18822); await L.ready(b);
const f = await L.fixture("s2b", 1);
const ev = (cursor, wait = 1) => get(b, `/api/agent/events?wait=${wait}${cursor ? "&cursor=" + encodeURIComponent(cursor) : ""}`, f.raw).then((r) => r.json());
const j0 = await ev();                      // cursor taken at t0 (hub caches tokens for this node)
await sleep(1500);
sql(`update public.external_vpn_devices set revoked_at=now() where device_id='${f.devices[0].device}'`);
sql(`update public.subscriptions set status='canceled' where id=${f.sub}`);
await sleep(7000);                          // far longer than any hub interval; NO waiter active, so no watcher tick
const t = Date.now(); const j1 = await ev(j0.cursor, 1);
console.log("1st call with pre-change cursor, 7s after the change, no waiter:", JSON.stringify({ reason: j1.reason, changed: j1.changed, ms: Date.now() - t }));
const j2 = await ev(j1.cursor, 3);
console.log("2nd call with the cursor the 1st returned:", JSON.stringify({ reason: j2.reason, changed: j2.changed }));
b.stop();
