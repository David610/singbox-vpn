#!/usr/bin/env node
// Paired control-plane x agent integration test (isolated: runs on a CI runner).
//   PostgreSQL 16 -> PostgREST -> vpn-web Node API (real handlers) -> the real agent binary.
// The agent's vpn-admin is a recording stand-in; nothing here touches a host service.
//
// env: VPNWEB_DIR AGENT_BIN POSTGREST_BIN [EXPECT="S1:pass,S2:fail,..."] [ONLY="S1,S3"]
// Exit 0 only if every scenario's outcome equals its expectation (default: pass).
import { spawn, execFileSync } from "node:child_process";
import http from "node:http";
import crypto from "node:crypto";
import { mkdtempSync, mkdirSync, writeFileSync, readdirSync, readFileSync as read } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";

const need = (k) => { if (!process.env[k]) { console.error(`${k} is required`); process.exit(2); } return process.env[k]; };
const VPNWEB = path.resolve(need("VPNWEB_DIR"));
const AGENT = path.resolve(need("AGENT_BIN"));
const POSTGREST = path.resolve(need("POSTGREST_BIN"));
const STUB = path.resolve(path.dirname(new URL(import.meta.url).pathname), "stub-vpn-admin");
const expect = Object.fromEntries((process.env.EXPECT || "").split(",").filter(Boolean).map((x) => x.split(":")));
const only = (process.env.ONLY || "").split(",").filter(Boolean);
const DB = "arcana_it", PGENV = { ...process.env, PGHOST: "localhost", PGPORT: "5432", PGUSER: "postgres", PGPASSWORD: "postgres", PGOPTIONS: "-c search_path=public,extensions -c client_min_messages=warning" };
const ENC_KEY = "22".repeat(32), JWT_SECRET = "TEST-ONLY-" + crypto.randomBytes(16).toString("hex");
const work = mkdtempSync(path.join(tmpdir(), "paired-it-"));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sha = (s) => crypto.createHash("sha256").update(s).digest("hex");
const psql = (q, db = DB) => execFileSync("psql", ["-qAtX", "-d", db, "-v", "ON_ERROR_STOP=1", "-c", q], { env: PGENV, encoding: "utf8" }).trim();
const psqlFile = (f, db = DB) => execFileSync("psql", ["-qX", "-d", db, "-v", "ON_ERROR_STOP=1", "-f", f], { env: PGENV, stdio: ["ignore", "ignore", "inherit"] });
const children = [];
const start = (cmd, args, opts = {}) => { const c = spawn(cmd, args, { stdio: ["ignore", "pipe", "pipe"], ...opts }); let log = ""; c.stdout.on("data", (d) => (log += d)); c.stderr.on("data", (d) => (log += d)); c.log = () => log; children.push(c); return c; };
const stopAll = () => children.forEach((c) => { try { c.kill("SIGTERM"); } catch {} });
process.on("exit", stopAll);

function jwt() {
  const b = (x) => Buffer.from(x).toString("base64url");
  const h = b(JSON.stringify({ alg: "HS256", typ: "JWT" })), p = b(JSON.stringify({ role: "service_role", exp: Math.floor(Date.now() / 1000) + 7200 }));
  return `${h}.${p}.${crypto.createHmac("sha256", JWT_SECRET).update(`${h}.${p}`).digest("base64url")}`;
}

async function bootstrap() {
  execFileSync("dropdb", ["--if-exists", DB], { env: PGENV }); execFileSync("createdb", [DB], { env: PGENV });
  psqlFile(path.join(VPNWEB, "scripts/sql/supabase-bootstrap.sql"));
  for (const m of readdirSync(path.join(VPNWEB, "supabase/migrations")).filter((f) => f.endsWith(".sql")).sort()) psqlFile(path.join(VPNWEB, "supabase/migrations", m));
  psqlFile(path.join(VPNWEB, "supabase/seed.sql"));
  psql("do $$ begin if not exists (select 1 from pg_roles where rolname='authenticator') then create role authenticator login noinherit password 'auth'; end if; end $$; grant anon, authenticated, service_role to authenticator;");
  writeFileSync(path.join(work, "pgrst.conf"), `db-uri = "postgres://authenticator:auth@localhost:5432/${DB}"\ndb-schemas = "public"\ndb-anon-role = "anon"\njwt-secret = "${JWT_SECRET}"\nserver-host = "127.0.0.1"\nserver-port = 3000\n`);
  const pg = start(POSTGREST, [path.join(work, "pgrst.conf")]);
  for (let i = 0; i < 100; i++) { try { if ((await fetch("http://127.0.0.1:3000/", { headers: { Authorization: `Bearer ${jwt()}` } })).status < 500) break; } catch {} await sleep(200); }
  http.createServer((req, res) => {
    const up = http.request({ host: "127.0.0.1", port: 3000, path: req.url.replace(/^\/rest\/v1/, "") || "/", method: req.method, headers: req.headers }, (r) => { res.writeHead(r.statusCode, r.headers); r.pipe(res); });
    up.on("error", () => { res.writeHead(502); res.end(); }); req.pipe(up);
  }).listen(3001, "127.0.0.1");
  const api = start(process.execPath, ["server/index.mjs"], { cwd: VPNWEB, env: { ...process.env, API_PORT: "18800", API_HOST: "127.0.0.1", API_ROUTE_PREFIXES: "/api/agent", SUPABASE_URL: "http://127.0.0.1:3001", SUPABASE_SERVICE_ROLE_KEY: jwt(), VPN_SECRETS_ENCRYPTION_KEY: ENC_KEY, METRICS_TOKEN: "m", NODE_AUTH_TTL_MS: "1000", EVENT_ENTITLEMENT_INTERVAL_MS: "2000" } });
  for (let i = 0; i < 100; i++) { try { if ((await fetch("http://127.0.0.1:18800/healthz")).ok) return api; } catch {} await sleep(200); }
  throw new Error("API did not start:\n" + api.log());
}

const { encryptSecret } = await import(pathToFileURL(path.join(VPNWEB, "functions/lib/crypto.js")).href);
let seq = 0;
const tag = () => `${Date.now().toString(36)}${(seq++).toString(36)}`.toLowerCase().padEnd(43, "x");

async function fixture(label, devices) {
  const key = crypto.randomBytes(24).toString("hex"), id = `${label}-${tag().slice(0, 8)}`.replace(/[^a-z0-9-]/g, "");
  const node = `node-${id}`, route = `route_${id.replace(/-/g, "_")}`, user = crypto.randomUUID();
  psql(`insert into public.nodes(node_id,api_key_hash,location_id,lifecycle_state,provider,hostname) values ('${node}','${sha(key)}','00000000-0000-4000-8000-000000000000','READY','mock','${node}.example.test')`);
  psql(`insert into public.logical_routes(id,region,privacy_class,display_name) values ('${route}','r${id.replace(/[^a-z0-9]/g, "").slice(-8)}x','fast','T'); insert into public.logical_route_targets(route_id,hop,node_id,priority) values ('${route}',1,'${node}',100)`);
  psql(`insert into auth.users(id,email) values ('${user}','${node}@example.test')`);
  const account = psql(`select account_id from public.account_members where user_id='${user}'`);
  const sub = psql(`insert into public.subscriptions(account_id,stripe_subscription_id,status,name,extra_seats,current_period_end) values ('${account}','sub_${node}','active','S',${Math.max(0, devices - 3)},now()+interval '30 days') returning id`).split("\n")[0];
  const list = [];
  for (let i = 0; i < devices; i++) {
    const p = tag(), principal = `ext_${p}`, cred = `cred_${p}`;
    const enc = await encryptSecret(JSON.stringify({ vless_uuid: crypto.randomUUID() }), ENC_KEY);
    const creds = JSON.stringify([{ hop: 1, credential_id: cred, credential_ciphertext: enc.ciphertext, credential_nonce: enc.nonce }]);
    psql(`select public.create_external_vpn_device('${account}','${user}',${sub},'D${i}','hiddify','${principal}','${crypto.randomBytes(32).toString("hex")}','${route}','${creds.replace(/'/g, "''")}'::jsonb, now()+interval '7 days')`);
    list.push({ principal, cred, device: psql(`select device_id from public.external_vpn_devices where principal_id='${principal}'`) });
  }
  return { key, node, account, sub, devices: list };
}

function startAgent(f, tagName) {
  const dir = path.join(work, tagName); mkdirSync(dir, { recursive: true });
  writeFileSync(path.join(dir, "agent.toml"), `worker_url = "http://127.0.0.1:18800"\nnode_id = "${f.node}"\nagent_api_key = "${f.key}"\npoll_interval_secs = 2\nvpn_admin_binary = "${STUB}"\nvpn_admin_config = "${dir}/none.toml"\nlease_pool_size = 0\nlease_state_file = "${dir}/lease.json"\nreport_queue_file = "${dir}/reports.json"\nop_dedup_file = "${dir}/dedup.json"\nexternal_authorization_state_file = "${dir}/ext.json"\nheartbeat_interval_secs = 60\n`);
  const c = start(AGENT, ["--config", path.join(dir, "agent.toml")], { env: { ...process.env, STUB_DIR: dir, RUST_LOG: "info" } });
  const latest = () => { const fs = readdirSync(dir).filter((x) => x.startsWith("applied-")).sort((a, b) => Number(a.slice(8, -5)) - Number(b.slice(8, -5))); return fs.length ? JSON.parse(read(path.join(dir, fs.at(-1)), "utf8")) : null; };
  const state = (cred) => { const a = latest(); const x = a?.authorizations?.find((y) => y.credential_id === cred); if (!x) return a ? "absent" : "none"; const now = Date.now() / 1000; return x.revoked ? "revoked" : x.valid_from <= now && now < x.valid_until ? "live" : "inactive"; };
  return { c, latest, state, stop: () => c.kill("SIGTERM"), log: () => c.log() };
}
async function until(pred, ms = 40000) { const t = Date.now(); while (Date.now() - t < ms) { if (pred()) return Date.now() - t; await sleep(250); } return null; }

const scenarios = {
  // Revocation by billing lapse changes no compatibility row.
  async S1(log) { const f = await fixture("s1", 1); const a = startAgent(f, "s1"); try {
    if (await until(() => a.state(f.devices[0].cred) === "live") === null) return "never became live";
    psql(`update public.subscriptions set status='canceled' where id=${f.sub}`);
    const t = await until(() => a.state(f.devices[0].cred) === "revoked"); log(`lapse applied in ${t} ms`); return t !== null ? true : "lapse never applied"; } finally { a.stop(); } },
  // I-100: renewal after a lapse must re-enable the credential.
  async S2(log) { const f = await fixture("s2", 1); const a = startAgent(f, "s2"); try {
    if (await until(() => a.state(f.devices[0].cred) === "live") === null) return "never became live";
    psql(`update public.subscriptions set status='canceled' where id=${f.sub}`);
    if (await until(() => a.state(f.devices[0].cred) === "revoked") === null) return "lapse never applied";
    psql(`update public.subscriptions set status='active' where id=${f.sub}`);
    const t = await until(() => a.state(f.devices[0].cred) === "live"); log(`renewal applied in ${t} ms`); return t !== null ? true : "renewal never re-enabled the credential"; } finally { a.stop(); } },
  // I-102: revoking a device younger than any minimum lifetime (real RPC shortens valid_until).
  async S3(log) { const f = await fixture("s3", 1); const a = startAgent(f, "s3"); try {
    if (await until(() => a.state(f.devices[0].cred) === "live") === null) return "never became live";
    psql(`select public.revoke_external_vpn_device('${f.devices[0].device}','${f.account}')`);
    const t = await until(() => a.state(f.devices[0].cred) === "revoked"); log(`fresh-device revoke applied in ${t} ms`); return t !== null ? true : "revocation of a fresh device never applied"; } finally { a.stop(); } },
  // I-102: one invalid credential must not block others or a legitimate revocation, and the
  // partially applied revision must not be acknowledged.
  async S4(log) { const f = await fixture("s4", 3); const [A, B, C] = f.devices; const a = startAgent(f, "s4"); try {
    if (await until(() => [A, B, C].every((d) => a.state(d.cred) === "live")) === null) return "never became live";
    psql(`update public.compatibility_authorizations set valid_until = valid_from + interval '40 days' where principal_id='${B.principal}'`);
    psql(`select public.revoke_external_vpn_device('${C.device}','${f.account}')`);
    const t = await until(() => a.state(C.cred) === "revoked"); log(`revocation beside a bad credential applied in ${t} ms`);
    if (t === null) return "a legitimate revocation was blocked by an unrelated invalid credential";
    if (a.state(A.cred) !== "live") return "a valid credential was disturbed";
    if (a.state(B.cred) === "live") return "the invalid credential stayed live (not fail closed)";
    const st = psql(`select state||':'||(applied_revision<desired_revision)::text from public.compatibility_authorization_node_state where node_id='${f.node}'`);
    if (!st.endsWith(":true")) return `partially applied revision was acknowledged (${st})`;
    return true; } finally { a.stop(); } },
};

const api = await bootstrap();
let bad = 0; const out = {};
for (const [name, fn] of Object.entries(scenarios)) {
  if (only.length && !only.includes(name)) continue;
  let res; const notes = [];
  try { res = await fn((m) => notes.push(m)); } catch (e) { res = `error: ${e.message}`; }
  const passed = res === true, want = (expect[name] || "pass") === "pass";
  const ok = passed === want; if (!ok) bad++;
  out[name] = { passed, expected: want ? "pass" : "fail", detail: passed ? notes.join("; ") : res };
  console.log(`${ok ? "OK  " : "BAD "} ${name}: ${passed ? "PASS" : "FAIL"} (expected ${want ? "pass" : "fail"}) ${passed ? notes.join("; ") : "- " + res}`);
}
writeFileSync(path.join(process.env.RESULT_DIR || work, "results.json"), JSON.stringify(out, null, 1));
if (bad) console.log("API log tail:\n" + api.log().split("\n").slice(-15).join("\n"));
process.exit(bad ? 1 : 0);
