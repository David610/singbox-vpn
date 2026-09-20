import { NextResponse, type NextRequest } from "next/server";
import { requireUser } from "@/lib/auth";
import { assertSameOrigin } from "@/lib/http";
import { appUrl } from "@/lib/env";
import { stripe } from "@/lib/stripe";
import { getCustomerId } from "@/lib/store";

export const runtime = "nodejs";

export async function POST(request: NextRequest) {
  try {
    assertSameOrigin(request);
    const user = await requireUser();
    const customerId = await getCustomerId(user.id);

    if (!customerId) {
      return NextResponse.redirect(new URL("/dashboard?error=no-billing-account", appUrl()), 303);
    }

    const session = await stripe().billingPortal.sessions.create({
      customer: customerId,
      return_url: `${appUrl()}/dashboard`
    });

    return NextResponse.redirect(session.url, 303);
  } catch (error) {
    console.error("billing_portal_failed", error instanceof Error ? error.message : "unknown");
    return NextResponse.redirect(new URL("/dashboard?error=billing", appUrl()), 303);
  }
}
