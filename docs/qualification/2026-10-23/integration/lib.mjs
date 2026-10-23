import { execFileSync, spawn } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import { readFileSync } from "node:fs";
import { encryptSecret } from "/root/vpn-web/functions/lib/crypto.js";

export const ENC_KEY = "11".repeat(32);
export const JWT = readFileSync("/root/integ/service.jwt", "utf8").trim();
const env = { ...process.env, PGHOST: "localhost", PGUSER: "postgres", PGPASSWORD: "postgres" };
export const sql = (q, db = "arcana") => execFileSync("psql", ["-qAtX", "-d", db, "-v", "ON_ERROR_STOP=1", "-c", q], { env, encoding: "utf8" }).trim();
export const sha = (s) => createHash("sha256").update(s).digest("hex");
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let n = 0;
const tag = () => `${Date.now().toString(36)}${(n++).toString(36)}`.padEnd(43, "x");

export function startApi(port, extra = {}, cwd = "/root/vpn-web") {
  const child = spawn(process.execPath, ["server/index.mjs"], {
    cwd,
    env: { ...process.env, API_PORT: String(port), API_HOST: "127.0.0.1", API_ROUTE_PREFIXES: "/api/agent",
      SUPABASE_URL: "http://127.0.0.1:3001", SUPABASE_SERVICE_ROLE_KEY: JWT, VPN_SECRETS_ENCRYPTION_KEY: ENC_KEY,
      METRICS_TOKEN: "m", NODE_AUTH_TTL_MS: "1000", ...extra },
    stdio: ["ignore", "ignore", "pipe"],
  });
  let err = ""; child.stderr.on("data", (d) => (err += d));
  return { child, port, base: `http://127.0.0.1:${port}`, stderr: () => err, stop: () => child.kill("SIGTERM") };
}
export async function ready(api) {
  for (let i = 0; i < 60; i++) { try { if ((await fetch(api.base + "/healthz")).ok) return; } catch {} await sleep(100); }
  throw new Error("api not ready: " + api.stderr());
}

// One node, one account with an active subscription and `devices` external devices.
export async function fixture(name, devices = 1) {
  const raw = randomBytes(24).toString("hex");
  const node = `node-${name}-${tag().slice(0, 8).toLowerCase()}`;
  const user = crypto.randomUUID();
  sql(`insert into public.nodes(node_id,api_key_hash,location_id,lifecycle_state,provider,hostname) values ('${node}','${sha(raw)}','00000000-0000-4000-8000-000000000000','READY','mock','${node}.example.test')`);
  const route = `route_${node.replace(/[^a-z0-9]/g, "_")}`;
  sql(`insert into public.logical_routes(id,region,privacy_class,display_name) values ('${route}','r${node.slice(-8).replace(/[^a-z0-9]/g,"")}x','fast','T'); insert into public.logical_route_targets(route_id,hop,node_id,priority) values ('${route}',1,'${node}',100)`);
  sql(`insert into auth.users(id,email) values ('${user}','${node}@example.test')`);
  const account = sql(`select account_id from public.account_members where user_id='${user}'`);
  const sub = sql(`insert into public.subscriptions(account_id,stripe_subscription_id,status,name,extra_seats,current_period_end) values ('${account}','sub_${node}','active','S',0,now()+interval '30 days') returning id`).split("\n")[0];
  const out = [];
  for (let i = 0; i < devices; i++) out.push(await addDevice({ account, user, sub, route, node, i }));
  return { raw, node, user, account, sub, route, devices: out };
}
export async function addDevice(f) {
  const p = tag(), principal = `ext_${p}`, cred = `cred_${p}`;
  const enc = await encryptSecret(JSON.stringify({ vless_uuid: crypto.randomUUID() }), ENC_KEY);
  const creds = JSON.stringify([{ hop: 1, credential_id: cred, credential_ciphertext: enc.ciphertext, credential_nonce: enc.nonce }]);
  sql(`select public.create_external_vpn_device('${f.account}','${f.user}',${f.sub},'D${f.i ?? 0}','hiddify','${principal}','${randomBytes(32).toString("hex")}','${f.route}','${creds}'::jsonb, now()+interval '7 days')`);
  const device = sql(`select device_id from public.external_vpn_devices where principal_id='${principal}'`);
  return { principal, cred, device };
}
export const get = (api, path, raw, headers = {}) => fetch(api.base + path, { headers: { Authorization: `Bearer ${raw}`, ...headers } });

let pass = 0, fail = 0; const results = [];
export async function check(name, fn) {
  const t0 = Date.now();
  try { const detail = await fn(); pass++; results.push({ name, ok: true, detail }); console.log(`PASS ${name}${detail ? " :: " + detail : ""} (${Date.now() - t0} ms)`); }
  catch (e) { fail++; results.push({ name, ok: false, detail: String(e.message) }); console.log(`FAIL ${name} :: ${e.message}`); }
}
export const summary = () => { console.log(`\n${pass} passed, ${fail} failed`); return fail; };
export function assert(c, m) { if (!c) throw new Error(m); }
