import Link from "next/link";

const priceDisplay = process.env.NEXT_PUBLIC_PRICE_DISPLAY || "Monthly subscription";

export default function HomePage() {
  return (
    <>
      <section className="hero">
        <p className="eyebrow">Simple VPN access</p>
        <h1>VPN access without a custom app.</h1>
        <p className="lead">
          Create an account, subscribe, and import your configuration into a
          compatible client. No separate client application is required.
        </p>
        <div className="actions">
          <Link className="button" href="/auth/signup">Create account</Link>
          <Link className="button secondary" href="/auth/login">Log in</Link>
        </div>
        <p className="mono" style={{ color: "#666", fontSize: "0.82rem" }}>{priceDisplay}</p>
      </section>

      <section className="grid" aria-label="What you get">
        <article className="card">
          <h3>One account</h3>
          <p>Sign up with email, manage the subscription, and keep VPN access in one dashboard.</p>
        </article>
        <article className="card">
          <h3>Useful formats</h3>
          <p>Copy a Hiddify/share-link subscription or download native sing-box and provisioning JSON.</p>
        </article>
        <article className="card">
          <h3>Existing server stack</h3>
          <p>The website provisions the same VLESS + REALITY and Hysteria2 credentials already produced by the server.</p>
        </article>
      </section>

      <section className="section">
        <div className="section-heading">
          <div>
            <p className="eyebrow">Flow</p>
            <h2>Account → payment → configuration</h2>
          </div>
        </div>
        <div className="grid" style={{ paddingBottom: 0 }}>
          <article className="card"><h3>1. Sign up</h3><p>Create and verify your account.</p></article>
          <article className="card"><h3>2. Subscribe</h3><p>Checkout and billing are handled by Stripe.</p></article>
          <article className="card"><h3>3. Connect</h3><p>Choose a config format in the dashboard and import it into your client.</p></article>
        </div>
      </section>
    </>
  );
}
