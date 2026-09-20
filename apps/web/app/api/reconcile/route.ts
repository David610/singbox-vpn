import { NextResponse, type NextRequest } from "next/server";
import { requireUser } from "@/lib/auth";
import {
  isOpenSubscriptionStatus,
  syncStripeSubscription
} from "@/lib/billing";
import { assertSameOrigin } from "@/lib/http";
import { appUrl, requiredEnv } from "@/lib/env";
import { stripe } from "@/lib/stripe";
import { getCustomerId } from "@/lib/store";

export const runtime = "nodejs";

export async function POST(request: NextRequest) {
  try {
    assertSameOrigin(request);
    const user = await requireUser();
    const customerId = await getCustomerId(user.id);

    if (!customerId) {
      return NextResponse.redirect(new URL("/dashboard", appUrl()), 303);
    }

    const subscriptions = await stripe().subscriptions.list({
      customer: customerId,
      status: "all",
      limit: 10
    });

    const configuredPrice = requiredEnv("STRIPE_PRICE_ID");
    const candidates = subscriptions.data.filter((subscription) =>
      subscription.items.data.some((item) => item.price.id === configuredPrice)
    );
    const match =
      candidates.find((subscription) =>
        isOpenSubscriptionStatus(subscription.status)
      ) ?? candidates[0];

    if (match) {
      await syncStripeSubscription(match);
    }

    return NextResponse.redirect(new URL("/dashboard", appUrl()), 303);
  } catch (error) {
    console.error("reconcile_failed", error instanceof Error ? error.message : "unknown");
    return NextResponse.redirect(new URL("/dashboard?error=reconcile", appUrl()), 303);
  }
}
