import { productName, supportEmail } from "@/lib/env";

export const dynamic = "force-dynamic";

export default function PrivacyPage() {
  const legalName = process.env.LEGAL_NAME?.trim() || "the service operator";
  const email = supportEmail();

  return (
    <article className="prose">
      <h1>Privacy Policy</h1>
      <p>
        This page is a minimal privacy notice for {productName()}. Before a public
        launch, replace or review it against the law that applies to your business
        and the countries you sell into.
      </p>

      <h2>Who processes the data</h2>
      <p>
        The service is operated by {legalName}.{" "}
        {email ? <>Contact: <a href={`mailto:${email}`}>{email}</a>.</> : "Add LEGAL_EMAIL or SUPPORT_EMAIL before launch."}
      </p>

      <h2>Account data</h2>
      <p>
        Supabase is used for account authentication. The service processes your
        email address, authentication records, and the identifiers needed to link
        your account to billing and VPN access.
      </p>

      <h2>Billing</h2>
      <p>
        Stripe processes checkout, subscription, invoice, payment-method, and
        billing information. The website stores Stripe customer and subscription
        identifiers, not card numbers.
      </p>

      <h2>VPN configuration</h2>
      <p>
        The service stores the identifiers needed to create and revoke your VPN
        account. Secret configuration URLs are encrypted before they are stored
        in the application database and are only shown to the authenticated owner.
      </p>

      <h2>Server and operational data</h2>
      <p>
        Do not advertise this service as “no logs” based on this website. VPN
        server, reverse-proxy, hosting-provider, and security logs must be audited
        separately and described here according to the actual production setup.
      </p>

      <h2>Retention and deletion</h2>
      <p>
        Keep account and billing records only as long as required for service,
        security, accounting, and legal obligations. Define concrete retention
        periods before launch and provide an account-deletion/support process.
      </p>
    </article>
  );
}
