import { NextResponse, type NextRequest } from "next/server";
import { requireUser } from "@/lib/auth";
import { assertSameOrigin } from "@/lib/http";
import { appUrl, requiredEnv } from "@/lib/env";
import { stripe } from "@/lib/stripe";
import {
  getCustomerId,
  getSubscription,
  saveCustomer
} from "@/lib/store";
import { isOpenSubscriptionStatus } from "@/lib/billing";

export const runtime = "nodejs";

export async function POST(request: NextRequest) {
  try {
    assertSameOrigin(request);
    const user = await requireUser();
    const client = stripe();

    const existingSubscription = await getSubscription(user.id);
    if (isOpenSubscriptionStatus(existingSubscription?.status)) {
      return NextResponse.redirect(new URL("/dashboard", appUrl()), 303);
    }

    let customerId = await getCustomerId(user.id);
    if (!customerId) {
      const customer = await client.customers.create(
        {
          email: user.email,
          metadata: { supabase_user_id: user.id }
        },
        { idempotencyKey: `supabase-customer-${user.id}` }
      );
      customerId = customer.id;
      await saveCustomer(user.id, customerId);
    }

    const session = await client.checkout.sessions.create({
      mode: "subscription",
      customer: customerId,
      client_reference_id: user.id,
      line_items: [{ price: requiredEnv("STRIPE_PRICE_ID"), quantity: 1 }],
      subscription_data: {
        metadata: { supabase_user_id: user.id }
      },
      metadata: { supabase_user_id: user.id },
      success_url: `${appUrl()}/dashboard?checkout=success`,
      cancel_url: `${appUrl()}/dashboard?checkout=cancelled`,
      billing_address_collection: "auto",
      automatic_tax: {
        enabled: process.env.STRIPE_AUTOMATIC_TAX === "true"
      }
    });

    if (!session.url) throw new Error("Stripe Checkout returned no URL");
    return NextResponse.redirect(session.url, 303);
  } catch (error) {
    console.error("checkout_failed", error instanceof Error ? error.message : "unknown");
    return NextResponse.redirect(new URL("/dashboard?error=checkout", appUrl()), 303);
  }
}
