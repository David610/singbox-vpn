# Native iPhone (Hiddify) acceptance procedure — status: UNVERIFIED (not yet performed)

Do **one step at a time**; after each, send back what the step asks for. Nothing is PASS until it is recorded in
`docs/release-acceptance/<tag>.evidence.json` (see `docs/DEVICE_ACCEPTANCE_TESTS.md`). The staging node's own IP is
refused by YouTube for Shorts (I-38), so Shorts steps (S) stay BLOCKED until a hairpin target is authorized; every
other step can run now. Never send the subscription URL, token or QR through any channel that is stored; the
maintainer hands you a dedicated test profile out of band.

| Step | Action | Expected | Send back |
|---|---|---|---|
| 0 | Settings > General > About: model + iOS version. Hiddify: Settings > About: version/build. Note whether it is App Store or TestFlight. | - | the three values as text |
| 1 | On the VPS (your own SSH terminal): `sudo bash /opt/singbox-vpn/deploy/lib/qa_profile_pack.sh <qa-user-id>` (see `vpn user list`; use a dedicated QA user). It prints three QR codes **to your terminal only** and writes nothing. Scan **1. REALITY only** with Hiddify (Add profile > scan). | the profile appears with **one** route and **no error dialog** | screenshot of the profile list (not of the QR); any error text verbatim |
| 2 | Connect. Open Safari > a "what is my IP" page. | the iOS VPN indicator shows; the page shows **62.238.46.190** | screenshot; the IP shown |
| 3 | Disconnect. Scan/paste **2. Hysteria2 only** (a single share link: Hiddify > Add from clipboard or scan). Connect; repeat the IP check. | same address | screenshot; note if connecting took > 10 s |
| 3b | Optionally import **3. Both transports** the way a normal user would. | two routes; Hiddify balances new connections across them (it ignores the profile's own selector), so this cell does not isolate a transport | screenshot |
| 4 | With either transport: open a DNS-leak test page and a "test-ipv6" page. | resolvers belong to the VPN node's network, not your carrier/ISP; IPv6 shown is the node's (2a01:4f9:...) or none, **never your carrier's** | screenshots (this is the only valid DNS/IPv6-leak evidence; server tests cannot give it) |
| 5 | Browse 3-4 ordinary sites and download a ~50 MB file over Wi-Fi. | normal speed, no stalls | rough speed, any failure |
| 6 | Lock the phone 2 min, unlock; then switch Wi-Fi -> cellular, then cellular -> Wi-Fi. | the VPN recovers within ~30 s each time and traffic resumes | seconds to recover per switch (Hysteria2 may take longer; record it) |
| 7 | Kill Hiddify (swipe away), reopen it. Then in Hiddify refresh the subscription (pull to refresh). | the profile is still there, no "Unexpected failure"; refresh succeeds | **exact text of any error**. If you ever see "open .../Library/Caches/Working/configs/<id>.json: no such file or directory": STOP, do not reinstall or delete anything, and send a screenshot plus what you did in the previous 10 minutes (this is upstream hiddify-app#2372; the evidence is more valuable than a fix) |
| 8 | Leave the VPN connected for 30 minutes with the screen locked, then check it is still connected and the IP check still works. | still connected | yes/no, the time |
| 9 | Repeat steps 2-3 on **cellular only** (Wi-Fi off). | same results | screenshots |
| S1 (BLOCKED) | YouTube app: 10 consecutive Shorts + the known-failing Short + a normal video with seeking, on each transport and network. | all play | count per transport/network |

A cell of the release matrix = one transport x one network with steps S1 recorded; steps 0-9 feed the supporting
notes. If any step fails, record it as FAIL with the text/screenshot; do not retry silently.

Why separate imports: Hiddify rebuilds its own groups and balances new connections across all routes it finds, so a profile with
both transports cannot tell you which one carried your traffic. The pinned REALITY profile and the single Hysteria2 link each
contain exactly one route (verified server-side by `deploy/lib/client_profile_check.py`, which does **not** prove the app works).

After the test, on the VPS: `vpn user remove <qa-user-id>`. Never paste the QR contents, the subscription URL or the links anywhere.
