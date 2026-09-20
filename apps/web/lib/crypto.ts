import {
  createCipheriv,
  createDecipheriv,
  randomBytes
} from "node:crypto";
import { requiredEnv } from "@/lib/env";

const VERSION = "v1";

function key(): Buffer {
  const decoded = Buffer.from(requiredEnv("VPN_CONFIG_ENCRYPTION_KEY"), "base64");
  if (decoded.length !== 32) {
    throw new Error("VPN_CONFIG_ENCRYPTION_KEY must decode to exactly 32 bytes");
  }
  return decoded;
}

export function encryptSecret(plaintext: string): string {
  const iv = randomBytes(12);
  const cipher = createCipheriv("aes-256-gcm", key(), iv);
  const ciphertext = Buffer.concat([
    cipher.update(plaintext, "utf8"),
    cipher.final()
  ]);
  const tag = cipher.getAuthTag();

  return [
    VERSION,
    iv.toString("base64url"),
    tag.toString("base64url"),
    ciphertext.toString("base64url")
  ].join(":");
}

export function decryptSecret(payload: string): string {
  const [version, ivPart, tagPart, ciphertextPart] = payload.split(":");
  if (version !== VERSION || !ivPart || !tagPart || !ciphertextPart) {
    throw new Error("Unsupported encrypted secret format");
  }

  const decipher = createDecipheriv(
    "aes-256-gcm",
    key(),
    Buffer.from(ivPart, "base64url")
  );
  decipher.setAuthTag(Buffer.from(tagPart, "base64url"));

  return Buffer.concat([
    decipher.update(Buffer.from(ciphertextPart, "base64url")),
    decipher.final()
  ]).toString("utf8");
}
