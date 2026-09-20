import { NextResponse, type NextRequest } from "next/server";
import type Stripe from "stripe";
import { requiredEnv } from "@/lib/env";
import { stripe } from "@/lib/stripe";
import {
  beginStripeEvent,
  failStripeEvent,
  finishStripeEvent
} from "@/lib/store";
import { syncStripeSubscription } from "@/lib/billing";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

async function syncCurrentSubscription(subscriptionId: string) {
  const current = await stripe().subscriptions.retrieve(subscriptionId);
  await syncStripeSubscription(current);
}

async function processEvent(event: Stripe.Event) {
  switch (event.type) {
    case "checkout.session.completed": {
      const session = event.data.object as Stripe.Checkout.Session;
      const subscriptionId =
        typeof session.subscription === "string"
          ? session.subscription
          : session.subscription?.id;

      if (subscriptionId) {
        await syncCurrentSubscription(subscriptionId);
      }
      break;
    }

    case "customer.subscription.created":
    case "customer.subscription.updated":
    case "customer.subscription.deleted": {
      const snapshot = event.data.object as Stripe.Subscription;

      // Stripe does not guarantee webhook delivery order. Re-read the
      // subscription so an older delivered event cannot overwrite newer
      // entitlement state, such as re-enabling an already-canceled account.
      await syncCurrentSubscription(snapshot.id);
      break;
    }

    default:
      break;
  }
}

export async function POST(request: NextRequest) {
  const signature = request.headers.get("stripe-signature");
  if (!signature) {
    return NextResponse.json({ error: "missing signature" }, { status: 400 });
  }

  let event: Stripe.Event;
  try {
    const body = await request.text();
    event = stripe().webhooks.constructEvent(
      body,
      signature,
      requiredEnv("STRIPE_WEBHOOK_SECRET")
    );
  } catch {
    return NextResponse.json({ error: "invalid signature" }, { status: 400 });
  }

  const shouldProcess = await beginStripeEvent(event.id, event.type);
  if (!shouldProcess) {
    return NextResponse.json({ received: true, duplicate: true });
  }

  try {
    await processEvent(event);
    await finishStripeEvent(event.id);
    return NextResponse.json({ received: true });
  } catch (error) {
    const message = error instanceof Error ? error.message : "unknown";
    await failStripeEvent(event.id, message);
    console.error("stripe_webhook_failed", event.id, event.type, message);
    return NextResponse.json({ error: "processing failed" }, { status: 500 });
  }
}
