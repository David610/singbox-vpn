begin;

create table if not exists public.billing_customers (
  user_id uuid primary key references auth.users(id) on delete cascade,
  stripe_customer_id text not null unique,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists public.subscriptions (
  user_id uuid primary key references auth.users(id) on delete cascade,
  stripe_subscription_id text not null unique,
  stripe_price_id text,
  status text not null,
  current_period_end timestamptz,
  cancel_at_period_end boolean not null default false,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists public.vpn_access (
  user_id uuid primary key references auth.users(id) on delete cascade,
  vpn_user_id text not null unique,
  subscription_url_encrypted text not null,
  provisioning_url_encrypted text not null,
  enabled boolean not null default false,
  provisioned_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists public.stripe_events (
  event_id text primary key,
  event_type text not null,
  created_at timestamptz not null default now(),
  processed_at timestamptz,
  last_error text
);

alter table public.billing_customers enable row level security;
alter table public.subscriptions enable row level security;
alter table public.vpn_access enable row level security;
alter table public.stripe_events enable row level security;

revoke all on table public.billing_customers from anon, authenticated;
revoke all on table public.subscriptions from anon, authenticated;
revoke all on table public.vpn_access from anon, authenticated;
revoke all on table public.stripe_events from anon, authenticated;

comment on table public.vpn_access is
  'Private server-side mapping from Supabase accounts to VPN credentials. Secret URLs are AES-256-GCM encrypted by the web service before storage.';

commit;
