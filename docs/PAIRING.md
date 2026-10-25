# Pairing a Russian-entry relay with a European exit (`vpn-admin pair`)

Target: client -> relay (entry, Russia) -> exit (Europe) -> Internet, with the optional Google/YouTube
exception (those domains leave through the relay, because YouTube refuses the exit's address).

Nothing here edits a generated sing-box config. Every step changes `deployment.toml` or the user store
through a validated command, and the relay/exit documents are re-rendered from them (`render-config`,
reconcile, `update.sh`, `--repair`).

## Secrets

A pairing step moves ONE protected document (a "bundle") between the hosts. It holds the other node's public
parameters and one dedicated credential. Rules the tools enforce:

- A bundle is written to a new `0600` file (`--out FILE`, never overwritten) or to stdout **only when stdout is
  not a terminal** (a pipe). A bundle file that is readable by group/others is refused when read.
- Credentials are never accepted as command-line arguments and never printed. Progress goes to stderr; `pair status` prints names and counts only.
- A credential is dedicated: one exit user per relay user (`peer-<label>`), one hairpin principal per exit
  (`hairpin-<label>`). Nothing is shared with an ordinary client, so revoking one affects only that route.

## Inputs (declared once)

| Name | Meaning |
|---|---|
| `EXIT_HOST`, `ENTRY_HOST` | DNS names (A records) of the exit and the entry. Real names are recommended (see baseline D5). |
| `RELEASE` | the published candidate tag installed on both hosts (same version on both) |
| decoy SNIs | `REALITY_HANDSHAKE_SERVER` for each host (a TLS 1.3 site reachable from that host) |
| `SPOOF_SNI` | optional: a name the entry's network allows (only where YouTube handshakes are filtered) |
| friend labels | one relay user per friend |

## Ordered procedure

Automated = a command below. **Owner** = cannot be automated.

0. **Owner**: create the two VPSs, DNS A records, provider firewall (22/tcp, 443/tcp+udp, 8443/tcp), an SSH key
   with a verified recovery path. See `docs/qualification/2026-10-25/NEXT_SESSION_HANDOFF.md`.
1. Exit install (on the exit): the published installer for `RELEASE` with `PUBLIC_HOST=$EXIT_HOST` and
   `REALITY_HANDSHAKE_SERVER=<decoy>`. `vpn-admin doctor` must report 0 FAIL.
2. Entry install (on the entry): the same installer with `--role relay --node-id ru1` and
   `PUBLIC_HOST=$ENTRY_HOST`. An unpaired relay forwards nothing; subscriptions answer 503 `no_selectable_route`.
3. Per friend, on the entry: `vpn-admin user create --name friend1 --json` (the JSON holds the personal links; the
   one to hand out is `subscription_url_hiddify_pinned`). Note the user id.
4. Pair the route, exit -> entry, one pipe per friend (the bundle never touches disk or the terminal):

       ssh exit  'vpn-admin pair issue-exit --label friend1 --flow vision' \
         | ssh entry 'vpn-admin pair accept-exit --user <friend-user-id> --endpoint-id <exit>-via-<entry> --bundle-stdin'

   - `--flow none` renders that friend's exit credential Vision-less (only if the nested-Vision hang of
     2026-10-25 reproduces; `docs/RELAY_EXIT_VISION.md`). All users of one declared endpoint share its flow, so
     use a different `--endpoint-id` for a Vision-less variant.
   - The first `accept-exit` declares the route in the entry's `deployment.toml` and applies it (rolled back if the
     apply fails); a later bundle for the same exit only adds that friend's credential. A conflicting
     declaration under the same id is refused, never merged.
5. Optional Google/YouTube exception:

       ssh entry 'vpn-admin pair issue-hairpin --label <exit>' \
         | ssh exit 'vpn-admin pair accept-hairpin --bundle-stdin'
       ssh entry 'vpn-admin tls-spoof set --sni <SPOOF_SNI>'       # only if the entry's network filters YouTube SNI

   (`tls-spoof set` also installs the capability drop-in; `docs/TLS_SPOOF.md`.) Without step 5 every destination
   leaves from the exit.
6. Verify: `vpn-admin pair status` on both hosts; `vpn-admin doctor` on both (0 FAIL; spoof capability OK when
   enabled); client check below.
7. **Owner**: send the friend the `?format=singbox&compat=hiddify-pinned` link, privately.

## Interrupted or wrong pairing

- Between "route declared" and "credential stored" the entry forwards only to the declared exit and nobody holds
  a credential for it, so the state is fail-closed (the friend's profile has no selectable route). Re-running
  `accept-exit` finishes it (`--replace-credential` if a credential was already stored).
- A failing apply restores `deployment.toml` (message: `deployment.toml restored`); no credential is stored.
- Wrong role (`issue-exit` on a relay, `accept-exit` on an exit, ...) and malformed bundles (wrong schema,
  bad UUID/key/short id, unknown fields) are refused before anything changes, without echoing the secret.
- Revoke a friend's route: disable/remove the exit user `peer-<label>` on the exit (the route fails closed), and/or the entry user.

## Client link

Only `?format=singbox&compat=hiddify-pinned` keeps the Russian first hop in Hiddify's engine. The share-link
form (`?format=hiddify`) cannot express the relay and connects straight to the exit; the unpinned sing-box form
becomes a Hiddify balancer. Mobile-app behaviour is unverified until a device test.

## What the Google exception means for egress

`vpn-admin pair status` states it per node: on the exit, "Google/YouTube egress: hairpinned through <entry>";
on the entry, "leaves THIS relay for its N hairpin principal(s)". Owner decision of 2026-10-25: for the private
pilot, Google/YouTube may exit through Russia while ordinary traffic exits through Europe.
