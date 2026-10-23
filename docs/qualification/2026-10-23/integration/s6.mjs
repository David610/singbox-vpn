// Agent lifecycle on the real stack: expiry during a DB outage, restart, credential rotation, node-key rotation.
import * as L from "./lib.mjs";
import { startAgent } from "./agent.mjs";
import { execSync } from "node:child_process";
import { encryptSecret } from "/root/vpn-web/functions/lib/crypto.js";
const kind = process.argv[2] || "fix";
const { sleep, sql, check, assert, get } = L;
const api = L.startApi(18951); await L.ready(api);
const f = await L.fixture("s6", 2);
// device 2 expires soon: shorten its validity
const soon = f.devices[1];
sql(`update public.compatibility_authorizations set valid_from=now()-interval '7 hours', valid_until=now()+interval '40 seconds' where principal_id='${soon.principal}'`);
const dir = `/root/integ/run/s6-${kind}`;
let ag = startAgent(kind, { url: api.base, node: f.node, key: f.raw, dir });
const waitFor = async (p, ms) => { const t = Date.now(); while (Date.now() - t < ms) { if (p()) return Date.now() - t; await sleep(100); } return null; };
const last = () => ag.applied().at(-1);
const active = () => (last()?.authorizations ?? last() ?? []).filter?.((x) => !x.revoked) ?? [];
const live = () => { const l = last(); if (!l) return -1; const now = Date.now() / 1000; return (l.authorizations ?? []).filter((x) => !x.revoked && x.valid_from <= now && now < x.valid_until).length; };

await check("initial apply: both credentials present", async () => { assert(await waitFor(() => (last()?.authorizations ?? []).length === 2, 20000) !== null, "no apply"); });
await check("expiry enforced locally WHILE the control plane is down (DB stopped before expiry)", async () => {
  execSync("pkill -x postgrest || true");
  const before = ag.applied().length;
  // the agent must re-render without the expired credential though no fetch can succeed
  const t = await waitFor(() => ag.applied().length > before && live() === 1, 70000);
  execSync("cd /root/integ && (nohup /tmp/pgrst/postgrest pgrst.conf >pgrst.log 2>&1 &)"); await sleep(2000);
  assert(t !== null, "expired credential was not removed during the outage"); return `${t} ms after outage start (credential had ~40 s left)`;
});
await check("agent restart with the control plane up: state restored, no duplicate/lost credentials", async () => {
  ag.stop(); await sleep(500);
  sql(`update public.external_vpn_devices set revoked_at=now() where device_id='${f.devices[0].device}'`);
  sql(`select public.revoke_external_vpn_device('${f.devices[0].device}','${f.account}')`);
  ag = startAgent(kind, { url: api.base, node: f.node, key: f.raw, dir: dir + "-r" });
  const t = await waitFor(() => (ag.applied().at(-1)?.authorizations ?? []).some((x) => x.principal_id === f.devices[0].principal && x.revoked), 20000);
  assert(t !== null, "revocation issued while the agent was down was not applied after restart"); return `${t} ms`;
});
await check("credential secret rotation: new ciphertext -> new revision -> applied with the new secret", async () => {
  const g = await L.fixture("s6rot", 1); ag.stop();
  const ag2 = startAgent(kind, { url: api.base, node: g.node, key: g.raw, dir: dir + "-rot" });
  assert(await waitFor(() => ag2.applied().length >= 1, 20000) !== null, "no first apply");
  const first = ag2.applied().at(-1).authorizations[0].vless_uuid ?? JSON.stringify(ag2.applied().at(-1).authorizations[0]);
  const nu = crypto.randomUUID(); const enc = await encryptSecret(JSON.stringify({ vless_uuid: nu }), L.ENC_KEY);
  sql(`update public.compatibility_authorizations set credential_ciphertext='${enc.ciphertext.replace(/'/g, "''")}', credential_nonce='${enc.nonce}' where principal_id='${g.devices[0].principal}'`);
  const t = await waitFor(() => JSON.stringify(ag2.applied().at(-1)).includes(nu), 20000);
  ag2.stop(); assert(t !== null, "rotated secret not applied"); return `${t} ms`;
});
await check("node API key rotation: old key refused (401), new key accepted", async () => {
  const g = await L.fixture("s6key", 1); const nk = "rotated-" + Math.random().toString(16).slice(2);
  sql(`update public.nodes set api_key_hash='${L.sha(nk)}' where node_id='${g.node}'`); await sleep(1300);
  assert((await get(api, "/api/agent/authorizations?schema=2", g.raw)).status === 401, "old key still accepted");
  assert((await get(api, "/api/agent/authorizations?schema=2", nk)).status === 200, "new key rejected");
});
ag.stop(); api.stop(); process.exit(L.summary() ? 1 : 0);
