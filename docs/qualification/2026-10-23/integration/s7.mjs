// New server (PR #84 + #85) on a database WITHOUT migration 20261018000000.
import * as L from "./lib.mjs";
const { check, assert, get, sleep } = L;
const a = L.startApi(18971); await L.ready(a);
const f = await L.fixture("s7", 2);
await check("authorizations works via the per-device fallback when device_entitlements_batch is missing", async () => {
  const r = await get(a, "/api/agent/authorizations?schema=2", f.raw); assert(r.status === 200, "status " + r.status);
  const j = await r.json(); assert((j.authorizations ?? j).length === 2, "len"); assert(r.headers.get("etag"), "etag");
});
await check("revocation still reflected without the migration", async () => {
  L.sql(`update public.subscriptions set status='canceled' where id=${f.sub}`);
  const j = await (await get(a, "/api/agent/authorizations?schema=2", f.raw)).json(); assert((j.authorizations ?? j).every((x) => x.revoked), "lapse not reflected");
});
await check("events endpoint without the wake-token functions: status (agents must fall back to polling, never storm)", async () => {
  const r = await get(a, "/api/agent/events?wait=1", f.raw);
  assert(r.status === 501, "expected 501 poll-fallback, got " + r.status); return `status=${r.status} retry-after=${r.headers.get("retry-after")}`;
});
a.stop(); process.exit(L.summary() ? 1 : 0);
