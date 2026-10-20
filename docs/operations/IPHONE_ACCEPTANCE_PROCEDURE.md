# Native iPhone (Hiddify) acceptance procedure — status: UNVERIFIED (not yet performed)

Do **one step at a time**; after each, send back what the step asks for. Nothing is PASS until it is recorded in
`docs/release-acceptance/<tag>.evidence.json` (see `docs/DEVICE_ACCEPTANCE_TESTS.md`). The staging node's own IP is
refused by YouTube for Shorts (I-38), so Shorts steps (S) stay BLOCKED until a hairpin target is authorized; every
other step can run now. Never send the subscription URL, token or QR through any channel that is stored; the
maintainer hands you a dedicated test profile out of band.

| Step | Action | Expected | Send back |
|---|---|---|---|
| 0 | Settings > General > About: model + iOS version. Hiddify: Settings > About: version/build. Note whether it is App Store or TestFlight. | - | the three values as text |
| 1 | Import the test subscription (QR or link) into Hiddify. | the profile appears with two routes (REALITY and Hysteria2) and **no error dialog** | screenshot of the profile list; any error text verbatim |
| 2 | Select **REALITY** and connect. Open Safari > a "what is my IP" page. | the iOS VPN indicator shows; the page shows **62.238.46.190** | screenshot; the IP shown |
| 3 | Disconnect, select **Hysteria2**, connect, repeat the IP check. | same address | screenshot; note if connecting took > 10 s |
| 4 | With either transport: open a DNS-leak test page and a "test-ipv6" page. | resolvers belong to the VPN node's network, not your carrier/ISP; IPv6 shown is the node's (2a01:4f9:...) or none, **never your carrier's** | screenshots (this is the only valid DNS/IPv6-leak evidence; server tests cannot give it) |
| 5 | Browse 3-4 ordinary sites and download a ~50 MB file over Wi-Fi. | normal speed, no stalls | rough speed, any failure |
| 6 | Lock the phone 2 min, unlock; then switch Wi-Fi -> cellular, then cellular -> Wi-Fi. | the VPN recovers within ~30 s each time and traffic resumes | seconds to recover per switch (Hysteria2 may take longer; record it) |
| 7 | Kill Hiddify (swipe away), reopen it. Then in Hiddify refresh the subscription (pull to refresh). | the profile is still there, no "Unexpected failure"; refresh succeeds | **exact text of any error**. If you ever see "open .../Library/Caches/Working/configs/<id>.json: no such file or directory": STOP, do not reinstall or delete anything, and send a screenshot plus what you did in the previous 10 minutes (this is upstream hiddify-app#2372; the evidence is more valuable than a fix) |
| 8 | Leave the VPN connected for 30 minutes with the screen locked, then check it is still connected and the IP check still works. | still connected | yes/no, the time |
| 9 | Repeat steps 2-3 on **cellular only** (Wi-Fi off). | same results | screenshots |
| S1 (BLOCKED) | YouTube app: 10 consecutive Shorts + the known-failing Short + a normal video with seeking, on each transport and network. | all play | count per transport/network |

A cell of the release matrix = one transport x one network with steps S1 recorded; steps 0-9 feed the supporting
notes. If any step fails, record it as FAIL with the text/screenshot; do not retry silently.
