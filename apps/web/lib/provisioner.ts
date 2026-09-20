import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { requiredEnv } from "@/lib/env";

const execFileAsync = promisify(execFile);
const USER_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

type ProvisionResult = {
  id: string;
  subscription_url: string;
  provisioning_url: string;
  enabled?: boolean;
};

function assertUserId(userId: string) {
  if (!USER_ID.test(userId)) {
    throw new Error("Invalid Supabase user id");
  }
}

function parseProvisionResult(stdout: string): ProvisionResult {
  const start = stdout.indexOf("{");
  const end = stdout.lastIndexOf("}");
  if (start < 0 || end <= start) {
    throw new Error("Provisioning helper did not return JSON");
  }

  const value = JSON.parse(stdout.slice(start, end + 1)) as Partial<ProvisionResult>;
  if (
    typeof value.id !== "string" ||
    typeof value.subscription_url !== "string" ||
    typeof value.provisioning_url !== "string"
  ) {
    throw new Error("Provisioning helper returned an invalid contract");
  }

  return value as ProvisionResult;
}

async function run(action: "provision" | "enable" | "disable", userId: string) {
  assertUserId(userId);
  const helper = process.env.VPN_PROVISION_HELPER || "/usr/local/sbin/vpn-web-provision";

  const { stdout } = await execFileAsync(
    "sudo",
    ["-n", helper, action, userId],
    {
      encoding: "utf8",
      timeout: 30_000,
      maxBuffer: 256 * 1024,
      env: {
        ...process.env,
        PATH: "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
      }
    }
  );

  return stdout;
}

export async function provision(userId: string): Promise<ProvisionResult> {
  return parseProvisionResult(await run("provision", userId));
}

export async function enable(userId: string): Promise<void> {
  await run("enable", userId);
}

export async function disable(userId: string): Promise<void> {
  await run("disable", userId);
}
