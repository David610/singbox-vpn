import { CopyField } from "@/components/CopyField";
import { logout } from "@/app/auth/actions";
import { requireUser } from "@/lib/auth";
import { subscriptionEntitled } from "@/lib/billing";
import { getSubscription } from "@/lib/store";
import { accessLinks } from "@/lib/vpn-access";

export const dynamic = "force-dynamic";

type Props = {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
};

function first(value: string | string[] | undefined): string | undefined {
  return Array.isArray(value) ? value[0] : value;
}

function dateLabel(value: string | null): string | null {
  if (!value) return null;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return null;
  return new Intl.DateTimeFormat("en", {
    year: "numeric",
    month: "short",
    day: "numeric"
  }).format(date);
}

export default async function DashboardPage({ searchParams }: Props) {
  const user = await requireUser();
  const params = await searchParams;
  const subscription = await getSubscription(user.id);
  const entitled = await subscriptionEntitled(user.id);
  const links = entitled ? await accessLinks(user.id) : null;
  const periodEnd = dateLabel(subscription?.current_period_end ?? null);

  return (
    <section className="dashboard">
      <div className="dashboard-head">
        <div>
          <p className="eyebrow">Account</p>
          <h1 style={{ fontSize: "2.4rem" }}>Dashboard</h1>
          <p style={{ margin: "0.5rem 0 0", color: "#666" }}>{user.email}</p>
        </div>
        <form action={logout}>
          <button className="secondary" type="submit">Log out</button>
        </form>
      </div>

      {first(params.checkout) === "success" && (
        <div className="notice success">
          Checkout completed. Stripe is confirming the subscription. If access
          is not visible yet, use “Refresh access” below.
        </div>
      )}
      {first(params.checkout) === "cancelled" && (
        <div className="notice">Checkout was cancelled. No access change was made.</div>
      )}
      {first(params.error) && (
        <div className="notice error">
          That action did not complete. Retry once; if it repeats, contact support.
        </div>
      )}

      <article className="card">
        <div className="section-heading" style={{ marginBottom: 0 }}>
          <div>
            <p className="eyebrow">Subscription</p>
            <h2 style={{ marginTop: "0.3rem" }}>VPN plan</h2>
          </div>
          <span className={`status ${entitled ? "active" : ""}`}>
            {subscription?.status || "not subscribed"}
          </span>
        </div>

        {periodEnd && (
          <p>
            {subscription?.cancel_at_period_end ? "Access scheduled through" : "Current billing period ends"}{" "}
            <strong>{periodEnd}</strong>.
          </p>
        )}

        <div className="actions" style={{ marginTop: "1rem" }}>
          {!subscription || !entitled ? (
            <form method="post" action="/api/checkout">
              <button type="submit">Subscribe with Stripe</button>
            </form>
          ) : (
            <form method="post" action="/api/billing">
              <button className="secondary" type="submit">Manage billing</button>
            </form>
          )}
          <form method="post" action="/api/reconcile">
            <button className="secondary" type="submit">Refresh access</button>
          </form>
        </div>
      </article>

      <article className="card">
        <p className="eyebrow">Configurations</p>
        <h2 style={{ marginTop: "0.3rem" }}>Connect</h2>

        {!entitled && (
          <p>An active subscription is required before VPN credentials are issued.</p>
        )}

        {entitled && !links && (
          <div className="notice">
            Your subscription is active, but VPN credentials are not ready. Use
            “Refresh access”. If it still fails, the provisioning service needs
            operator attention.
          </div>
        )}

        {links && (
          <div className="config-list" style={{ marginTop: "1rem" }}>
            <div className="config-row">
              <div>
                <h3>Hiddify / share-link subscription</h3>
                <p>Copy this URL into a compatible subscription importer.</p>
              </div>
              <CopyField value={links.hiddify} label="Hiddify subscription URL" />
              <a href="/api/config/hiddify">Download share links</a>
            </div>

            <div className="config-row">
              <div>
                <h3>URI list</h3>
                <p>Plain VLESS/Hysteria2 share-link representation.</p>
              </div>
              <CopyField value={links.uri} label="URI subscription URL" />
              <a href="/api/config/uri">Download URI list</a>
            </div>

            <div className="config-row">
              <div>
                <h3>Native sing-box JSON</h3>
                <p>For clients that consume the server’s native sing-box subscription.</p>
              </div>
              <CopyField value={links.singbox} label="sing-box subscription URL" />
              <a href="/api/config/singbox">Download JSON</a>
            </div>

            <div className="config-row">
              <div>
                <h3>Provisioning JSON</h3>
                <p>The versioned provisioning contract exposed by the VPN server.</p>
              </div>
              <CopyField value={links.provision} label="Provisioning URL" />
              <a href="/api/config/provision">Download provisioning JSON</a>
            </div>

            <div className="notice">
              Treat these URLs like passwords. Anyone holding one can retrieve
              configuration material until the VPN account is revoked or rotated.
            </div>
          </div>
        )}
      </article>
    </section>
  );
}
