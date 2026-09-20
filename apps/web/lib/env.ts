export function requiredEnv(name: string): string {
  const value = process.env[name]?.trim();
  if (!value) {
    throw new Error(`Missing required environment variable: ${name}`);
  }
  return value;
}

export function appUrl(): string {
  return requiredEnv("NEXT_PUBLIC_APP_URL").replace(/\/$/, "");
}

export function entitledStatuses(): Set<string> {
  const raw = process.env.VPN_ENTITLED_STATUSES || "active,trialing";
  return new Set(raw.split(",").map((value) => value.trim()).filter(Boolean));
}

export function productName(): string {
  return process.env.NEXT_PUBLIC_PRODUCT_NAME?.trim() || "VPN";
}

export function supportEmail(): string {
  return process.env.SUPPORT_EMAIL?.trim() || process.env.LEGAL_EMAIL?.trim() || "";
}
