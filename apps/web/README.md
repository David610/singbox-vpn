# Customer web app

Minimal customer-facing control plane for `singbox-vpn`.

It intentionally does four things only:

1. Supabase email/password signup, login, verification and password reset.
2. Stripe-hosted subscription Checkout and Customer Portal.
3. Stripe webhook -> one VPN identity per Supabase user.
4. Authenticated config delivery in Hiddify/share-link, URI, native sing-box JSON and provisioning-contract formats.

It does **not** replace the existing `vpn-admin`, subscription renderer, or
protocol logic. The web app calls a narrow root helper which in turn uses the
existing admin CLI, so there is still one source of truth for VPN credentials.

## Architecture

```text
browser
  |
  | Supabase auth cookie
  v
Next.js web app :3000
  |                 \
  |                  \ Stripe Checkout / Portal / signed webhook
  |
  | sudo: provision|enable|disable + verified Supabase UUID only
  v
vpn-web-provision
  |
  v
vpn-admin -> users.json -> validated sing-box config -> reload
  |
  v
existing /sub/{token} and /v1/provision/{token}
```

The raw VPN subscription/provisioning URLs are persisted twice because the
server intentionally stores only their hashes after creation:

- root-only recovery mapping: `/var/lib/singbox-vpn/web-users/<uuid>.json`
- Supabase `vpn_access`: AES-256-GCM encrypted with
  `VPN_CONFIG_ENCRYPTION_KEY`

## 1. Supabase

Create a project and run the SQL migration in
`supabase/migrations/202609200001_web_billing.sql`.

In Auth settings:

- enable email/password auth;
- require email confirmation for production;
- set the Site URL to `NEXT_PUBLIC_APP_URL`;
- allow `https://YOUR_DOMAIN/auth/callback` as a redirect URL;
- configure production SMTP before inviting real users.

The four application tables have RLS enabled and all `anon` /
`authenticated` table grants revoked. Browser code never receives the service
role key.

## 2. Stripe

Create exactly one recurring Price and set its ID as `STRIPE_PRICE_ID`.

Configure the Customer Portal. For this MVP, keep plan switching and quantity
changes disabled. Enable cancellation, payment-method updates and invoice
history as needed.

Create a webhook destination:

```text
POST https://YOUR_DOMAIN/api/stripe/webhook
```

Subscribe to:

- `checkout.session.completed`
- `customer.subscription.created`
- `customer.subscription.updated`
- `customer.subscription.deleted`

Copy the signing secret to `STRIPE_WEBHOOK_SECRET`.

Access is granted only when both conditions hold:

- subscription Price equals `STRIPE_PRICE_ID`;
- status is in `VPN_ENTITLED_STATUSES` (default `active,trialing`).

That means `past_due` is suspended by default. Change the environment value if
you intentionally want a billing grace period.

Do not grant access from the Checkout success redirect. The signed webhook (or
the authenticated “Refresh access” reconciliation endpoint) is authoritative.

## 3. Install the provisioning helper on the VPN host

The web process must **not** receive general root or `vpn-admin` access.

```bash
sudo install -o root -g root -m 0755 \
  deploy/almalinux/vpn-web-provision \
  /usr/local/sbin/vpn-web-provision

sudo useradd --system --home /nonexistent --shell /usr/sbin/nologin vpn-web \
  2>/dev/null || true

sudo install -o root -g root -m 0440 \
  deploy/almalinux/vpn-web-sudoers \
  /etc/sudoers.d/vpn-web

sudo visudo -cf /etc/sudoers.d/vpn-web
```

The helper accepts only:

```text
provision <Supabase UUID>
enable    <Supabase UUID>
disable   <Supabase UUID>
```

It never accepts a VPN username or arbitrary command from the web request.

## 4. Web environment

Copy `.env.example` to a root-owned environment file, for example
`/etc/singbox-vpn/web.env`.

Generate the config encryption key once:

```bash
openssl rand -base64 32
```

Keep this key backed up. Losing it makes the encrypted database copies of the
subscription URLs unreadable, although the root-only helper mappings remain a
recovery source.

## 5. Build and run

Generate and commit a lockfile from a networked development machine before the
production merge:

```bash
cd apps/web
npm install
npm run typecheck
npm run build
```

Deploy `apps/web` to `/opt/singbox-vpn-web`, install dependencies, then install
and enable the provided service:

```bash
sudo install -o root -g root -m 0644 \
  apps/web/deploy/vpn-web.service \
  /etc/systemd/system/vpn-web.service

sudo systemctl daemon-reload
sudo systemctl enable --now vpn-web
curl -fsS http://127.0.0.1:3000/api/health
```

Put the app behind your HTTPS reverse proxy and proxy the public web domain to
`127.0.0.1:3000`. Do not expose port 3000 publicly.

## 6. Required launch checks

Before accepting real money:

- real domain, DNS and HTTPS are active;
- Supabase email confirmation, redirect URLs and SMTP are tested;
- Stripe live Price, Checkout, Portal and webhook are tested end-to-end;
- Stripe tax settings match your business/tax registrations;
- `LEGAL_NAME`, `LEGAL_ADDRESS`, `LEGAL_EMAIL`, support contact, terms,
  privacy notice, cancellation/refund process and consumer disclosures are
  reviewed for the jurisdictions you sell into;
- the actual VPN/reverse-proxy/hosting logs are audited before making any
  “no-logs” claim;
- a test user can subscribe, receive access, download each format, cancel, and
  is disabled when the subscription becomes non-entitled;
- backups include both the existing VPN state and
  `/var/lib/singbox-vpn/web-users`.

## Design

The visual system intentionally follows `david610.github.io`: system UI font
stack, white background, `#333` text, `#666` muted text, `#0070f3` links,
thin `#eaeaea` borders, restrained radius and compact cards. No separate design
system or stock VPN graphics were introduced.
