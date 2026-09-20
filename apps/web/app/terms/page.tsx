import { productName, supportEmail } from "@/lib/env";

export const dynamic = "force-dynamic";

export default function TermsPage() {
  const email = supportEmail();

  return (
    <article className="prose">
      <h1>Terms of Service</h1>
      <p>
        These are minimal launch terms for {productName()} and are not a substitute
        for jurisdiction-specific legal review.
      </p>

      <h2>Service</h2>
      <p>
        A paid subscription provides credentials for the supported VPN service
        and the configuration formats shown in the account dashboard. Compatible
        third-party client applications are separate products.
      </p>

      <h2>Billing and cancellation</h2>
      <p>
        Subscriptions renew according to the interval and price shown in Stripe
        Checkout until canceled. Billing details and cancellation are managed
        through Stripe’s customer portal. Access remains tied to the subscription
        status received from Stripe.
      </p>

      <h2>Account and credential security</h2>
      <p>
        Keep account credentials and VPN configuration URLs private. You are
        responsible for activity performed with credentials issued to your account.
        Contact support if you believe a credential has been exposed.
      </p>

      <h2>Acceptable use</h2>
      <p>
        Do not use the service to violate applicable law, attack systems, send
        abuse or spam, distribute malware, or interfere with other users or
        infrastructure. Access may be suspended when necessary to protect the
        service or comply with legal obligations.
      </p>

      <h2>Availability</h2>
      <p>
        Internet routes, hosting networks, third-party applications, and external
        services can change. The service does not guarantee access to every site,
        country, or network.
      </p>

      <h2>Consumer rights</h2>
      <p>
        Statutory consumer rights are not excluded by these terms. Before selling
        to consumers, configure the refund policy and any required withdrawal,
        pre-contract, tax, and recurring-payment disclosures for your jurisdiction.
      </p>

      {email && <p>Support: <a href={`mailto:${email}`}>{email}</a></p>}
    </article>
  );
}
