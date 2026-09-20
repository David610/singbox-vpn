import type Stripe from "stripe";
import { entitledStatuses, requiredEnv } from "@/lib/env";
import {
  getSubscription,
  saveSubscription,
  userIdForCustomer
} from "@/lib/store";
import { disableVpnAccess, enableVpnAccess } from "@/lib/vpn-access";

export function isOpenSubscriptionStatus(status: string | undefined | null): boolean {
  return Boolean(status && !["canceled", "incomplete_expired"].includes(status));
}

export async function subscriptionEntitled(userId: string): Promise<boolean> {
  const row = await getSubscription(userId);
  if (!row) return false;

  return (
    row.stripe_price_id === requiredEnv("STRIPE_PRICE_ID") &&
    entitledStatuses().has(row.status)
  );
}

async function resolveUserId(subscription: Stripe.Subscription): Promise<string> {
  const metadataId = subscription.metadata?.supabase_user_id;
  if (metadataId) return metadataId;

  const customerId =
    typeof subscription.customer === "string"
      ? subscription.customer
      : subscription.customer.id;

  const userId = await userIdForCustomer(customerId);
  if (!userId) {
    throw new Error("Stripe subscription is not linked to a Supabase user");
  }
  return userId;
}

export async function syncStripeSubscription(subscription: Stripe.Subscription) {
  const userId = await resolveUserId(subscription);
  const item = subscription.items.data[0];
  const priceId = item?.price?.id ?? null;
  const currentPeriodEnd = item?.current_period_end
    ? new Date(item.current_period_end * 1000).toISOString()
    : null;

  await saveSubscription({
    user_id: userId,
    stripe_subscription_id: subscription.id,
    stripe_price_id: priceId,
    status: subscription.status,
    current_period_end: currentPeriodEnd,
    cancel_at_period_end: subscription.cancel_at_period_end
  });

  const entitled =
    priceId === requiredEnv("STRIPE_PRICE_ID") &&
    entitledStatuses().has(subscription.status);

  if (entitled) {
    await enableVpnAccess(userId);
  } else {
    await disableVpnAccess(userId);
  }

  return { userId, entitled };
}
