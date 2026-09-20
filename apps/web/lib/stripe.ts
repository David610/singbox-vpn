import Stripe from "stripe";
import { requiredEnv } from "@/lib/env";

let client: Stripe | undefined;

export function stripe(): Stripe {
  if (!client) {
    client = new Stripe(requiredEnv("STRIPE_SECRET_KEY"), {
      appInfo: {
        name: "singbox-vpn-web",
        version: "0.1.0"
      }
    });
  }
  return client;
}
