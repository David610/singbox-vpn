import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync, readdirSync, readFileSync, rmSync } from "node:fs";
export const AGENT = { old: "/root/tgt-old/release/vpn-provisioning-agent", new: "/root/tgt-new/release/vpn-provisioning-agent", fix: "/root/tgt-fix/release/vpn-provisioning-agent" };
export function startAgent(kind, { url, node, key, dir, extra = "" }) {
  rmSync(dir, { recursive: true, force: true }); mkdirSync(dir, { recursive: true });
  writeFileSync(`${dir}/agent.toml`, `worker_url = "${url}"
node_id = "${node}"
agent_api_key = "${key}"
poll_interval_secs = 5
vpn_admin_binary = "/root/integ/stub-vpn-admin"
vpn_admin_config = "${dir}/none.toml"
lease_pool_size = 0
lease_state_file = "${dir}/lease.json"
report_queue_file = "${dir}/reports.json"
op_dedup_file = "${dir}/dedup.json"
external_authorization_state_file = "${dir}/extauth.json"
heartbeat_interval_secs = 60
${extra}`);
  const child = spawn(AGENT[kind], ["--config", `${dir}/agent.toml`], { env: { ...process.env, STUB_DIR: dir, RUST_LOG: "info" }, stdio: ["ignore", "pipe", "pipe"] });
  let log = ""; child.stdout.on("data", (d) => (log += d)); child.stderr.on("data", (d) => (log += d));
  const applied = () => readdirSync(dir).filter((f) => f.startsWith("applied-")).sort().map((f) => JSON.parse(readFileSync(`${dir}/${f}`, "utf8")));
  return { child, log: () => log, applied, stop: () => child.kill("SIGTERM"), dir };
}
