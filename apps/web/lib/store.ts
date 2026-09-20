import { createAdminClient } from "@/lib/supabase/admin";

export type SubscriptionRow = {
  user_id: string;
  stripe_subscription_id: string;
  stripe_price_id: string | null;
  status: string;
  current_period_end: string | null;
  cancel_at_period_end: boolean;
  updated_at: string;
};

export type VpnAccessRow = {
  user_id: string;
  vpn_user_id: string;
  subscription_url_encrypted: string;
  provisioning_url_encrypted: string;
  enabled: boolean;
  provisioned_at: string;
  updated_at: string;
};

export async function getCustomerId(userId: string): Promise<string | null> {
  const db = createAdminClient();
  const { data, error } = await db
    .from("billing_customers")
    .select("stripe_customer_id")
    .eq("user_id", userId)
    .maybeSingle();

  if (error) throw error;
  return data?.stripe_customer_id ?? null;
}

export async function saveCustomer(userId: string, stripeCustomerId: string) {
  const db = createAdminClient();
  const { error } = await db.from("billing_customers").upsert(
    {
      user_id: userId,
      stripe_customer_id: stripeCustomerId,
      updated_at: new Date().toISOString()
    },
    { onConflict: "user_id" }
  );
  if (error) throw error;
}

export async function userIdForCustomer(stripeCustomerId: string): Promise<string | null> {
  const db = createAdminClient();
  const { data, error } = await db
    .from("billing_customers")
    .select("user_id")
    .eq("stripe_customer_id", stripeCustomerId)
    .maybeSingle();

  if (error) throw error;
  return data?.user_id ?? null;
}

export async function getSubscription(userId: string): Promise<SubscriptionRow | null> {
  const db = createAdminClient();
  const { data, error } = await db
    .from("subscriptions")
    .select("*")
    .eq("user_id", userId)
    .maybeSingle();

  if (error) throw error;
  return (data as SubscriptionRow | null) ?? null;
}

export async function saveSubscription(row: Omit<SubscriptionRow, "updated_at">) {
  const db = createAdminClient();
  const { error } = await db.from("subscriptions").upsert(
    {
      ...row,
      updated_at: new Date().toISOString()
    },
    { onConflict: "user_id" }
  );
  if (error) throw error;
}

export async function getVpnAccess(userId: string): Promise<VpnAccessRow | null> {
  const db = createAdminClient();
  const { data, error } = await db
    .from("vpn_access")
    .select("*")
    .eq("user_id", userId)
    .maybeSingle();

  if (error) throw error;
  return (data as VpnAccessRow | null) ?? null;
}

export async function saveVpnAccess(input: {
  userId: string;
  vpnUserId: string;
  subscriptionUrlEncrypted: string;
  provisioningUrlEncrypted: string;
  enabled: boolean;
}) {
  const db = createAdminClient();
  const now = new Date().toISOString();
  const { error } = await db.from("vpn_access").upsert(
    {
      user_id: input.userId,
      vpn_user_id: input.vpnUserId,
      subscription_url_encrypted: input.subscriptionUrlEncrypted,
      provisioning_url_encrypted: input.provisioningUrlEncrypted,
      enabled: input.enabled,
      provisioned_at: now,
      updated_at: now
    },
    { onConflict: "user_id" }
  );
  if (error) throw error;
}

export async function markVpnEnabled(userId: string, enabled: boolean) {
  const db = createAdminClient();
  const { error } = await db
    .from("vpn_access")
    .update({ enabled, updated_at: new Date().toISOString() })
    .eq("user_id", userId);

  if (error) throw error;
}

export async function beginStripeEvent(eventId: string, eventType: string): Promise<boolean> {
  const db = createAdminClient();
  const { data: existing, error: readError } = await db
    .from("stripe_events")
    .select("processed_at")
    .eq("event_id", eventId)
    .maybeSingle();

  if (readError) throw readError;
  if (existing?.processed_at) return false;

  if (!existing) {
    const { error } = await db.from("stripe_events").insert({
      event_id: eventId,
      event_type: eventType
    });
    if (error && error.code !== "23505") throw error;
  }

  return true;
}

export async function finishStripeEvent(eventId: string) {
  const db = createAdminClient();
  const { error } = await db
    .from("stripe_events")
    .update({
      processed_at: new Date().toISOString(),
      last_error: null
    })
    .eq("event_id", eventId);

  if (error) throw error;
}

export async function failStripeEvent(eventId: string, message: string) {
  const db = createAdminClient();
  await db
    .from("stripe_events")
    .update({
      last_error: message.slice(0, 500)
    })
    .eq("event_id", eventId);
}
