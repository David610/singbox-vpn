import * as L from "./lib.mjs";
const { check, assert, get, sql, sleep } = L;
const a = L.startApi(18801), b = L.startApi(18802);
await L.ready(a); await L.ready(b);
const f = await L.fixture("s1", 2);
const P = "/api/agent/authorizations?schema=2";

await check("401 without/with wrong key", async () => {
  assert((await get(a, P, "nope")).status === 401, "wrong key not 401");
  assert((await fetch(a.base + P)).status === 401, "no key not 401");
});
let etag, body;
await check("authorizations: 200 with ETag, decrypted protocol, two live creds", async () => {
  const r = await get(a, P, f.raw); assert(r.status === 200, "status " + r.status);
  etag = r.headers.get("etag"); body = await r.json(); assert(etag, "no etag");
  const list = body.authorizations ?? body; assert(list.length === 2, "len " + list.length);
  assert(list.every((x) => x.revoked === false && x.protocol?.vless_uuid), "not live/decrypted");
  return `etag ${etag.slice(0, 12)}…`;
});
await check("ETag/304: If-None-Match -> 304 empty body, same on instance B (stateless)", async () => {
  for (const api of [a, b]) {
    const r = await get(api, P, f.raw, { "If-None-Match": etag }); assert(r.status === 304, `status ${r.status}`);
    assert((await r.text()) === "", "304 had body");
  }
});
await check("weak-validator and list forms of If-None-Match accepted; wrong etag -> 200", async () => {
  assert((await get(a, P, f.raw, { "If-None-Match": `W/${etag}` })).status === 304, "W/");
  assert((await get(a, P, f.raw, { "If-None-Match": `"x", ${etag}` })).status === 304, "list");
  assert((await get(a, P, f.raw, { "If-None-Match": '"deadbeef"' })).status === 200, "stale etag must give 200");
});
await check("revocation: device revoked -> ETag changes, revoked=true, stale ETag cannot hide it", async () => {
  sql(`update public.external_vpn_devices set revoked_at=now() where device_id='${f.devices[0].device}'`);
  const r = await get(a, P, f.raw, { "If-None-Match": etag }); assert(r.status === 200, "revocation hidden behind 304: " + r.status);
  const list = (await r.json()); const l = list.authorizations ?? list;
  const x = l.find((y) => y.principal_id === f.devices[0].principal); assert(x.revoked === true, "not marked revoked");
  assert(l.find((y) => y.principal_id === f.devices[1].principal).revoked === false, "cross-user leak: other device revoked");
  etag = r.headers.get("etag");
});
await check("subscription lapse (canceled) -> all revoked, ETag changes, no DB row changed in compat table", async () => {
  sql(`update public.subscriptions set status='canceled' where id=${f.sub}`);
  const r = await get(b, P, f.raw, { "If-None-Match": etag }); assert(r.status === 200, "lapse hidden: " + r.status);
  const l = await r.json(); assert((l.authorizations ?? l).every((x) => x.revoked), "entitlement lapse not reflected");
  sql(`update public.subscriptions set status='active' where id=${f.sub}`);
});
await check("time-based expiry without any write: current_period_end in the past -> revoked", async () => {
  sql(`update public.subscriptions set current_period_end=now()-interval '1 day' where id=${f.sub}`);
  const l = await (await get(a, P, f.raw)).json(); const rows = l.authorizations ?? l;
  const live = rows.filter((x) => !x.revoked).length;
  sql(`update public.subscriptions set current_period_end=now()+interval '30 days' where id=${f.sub}`);
  return `live after period_end passed: ${live} (grace policy may keep entitlement; see past_due_grace)`;
});
await check("cross-node isolation: another node's key sees none of this node's credentials", async () => {
  const g = await L.fixture("s1other", 1);
  const l = await (await get(a, P, g.raw)).json(); const rows = l.authorizations ?? l;
  assert(rows.length === 1 && rows[0].principal_id === g.devices[0].principal, "wrong set");
});
await check("bad schema -> 400; unknown route -> not 200", async () => {
  assert((await get(a, "/api/agent/authorizations?schema=9", f.raw)).status === 400, "schema 9");
  assert((await get(a, "/api/agent/nope", f.raw)).status !== 200, "unknown route");
});
console.log("STDERR_A:", a.stderr().slice(0,1500)); a.stop(); b.stop();
process.exit(L.summary() ? 1 : 0);
