import { decryptSecret, encryptSecret } from "@/lib/crypto";
import * as provisioner from "@/lib/provisioner";
import {
  getVpnAccess,
  markVpnEnabled,
  saveVpnAccess
} from "@/lib/store";

export type AccessLinks = {
  hiddify: string;
  uri: string;
  singbox: string;
  provision: string;
};

function withFormat(subscriptionUrl: string, format: "hiddify" | "uri" | "singbox") {
  const url = new URL(subscriptionUrl);
  url.searchParams.set("format", format);
  return url.toString();
}

export async function enableVpnAccess(userId: string) {
  const existing = await getVpnAccess(userId);

  if (existing) {
    if (!existing.enabled) {
      await provisioner.enable(userId);
      await markVpnEnabled(userId, true);
    }
    return;
  }

  const created = await provisioner.provision(userId);
  await saveVpnAccess({
    userId,
    vpnUserId: created.id,
    subscriptionUrlEncrypted: encryptSecret(created.subscription_url),
    provisioningUrlEncrypted: encryptSecret(created.provisioning_url),
    enabled: true
  });
}

export async function disableVpnAccess(userId: string) {
  const existing = await getVpnAccess(userId);
  if (!existing || !existing.enabled) return;

  await provisioner.disable(userId);
  await markVpnEnabled(userId, false);
}

export async function accessLinks(userId: string): Promise<AccessLinks | null> {
  const access = await getVpnAccess(userId);
  if (!access?.enabled) return null;

  const subscriptionUrl = decryptSecret(access.subscription_url_encrypted);
  const provisioningUrl = decryptSecret(access.provisioning_url_encrypted);

  return {
    hiddify: withFormat(subscriptionUrl, "hiddify"),
    uri: withFormat(subscriptionUrl, "uri"),
    singbox: withFormat(subscriptionUrl, "singbox"),
    provision: provisioningUrl
  };
}
