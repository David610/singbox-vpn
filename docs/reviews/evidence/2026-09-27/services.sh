#!/usr/bin/env bash
set -euo pipefail
L="$(pwd)"
mkdir -p www meta localsecret
echo "TARGET-OK" > www/index.html
echo "SIMULATED-CLOUD-METADATA-SERVICE" > meta/index.html
echo "EXIT-LOOPBACK-ONLY-SERVICE" > localsecret/index.html
openssl req -x509 -newkey rsa:2048 -days 2 -nodes -keyout decoy.key -out decoy.crt -subj "/CN=decoy.test" -addext "subjectAltName=DNS:decoy.test" 2>/dev/null
ip netns exec target dnsmasq --no-resolv --no-hosts --listen-address=10.0.4.2,127.0.0.1 --bind-interfaces --address=/decoy.test/10.0.4.2 --address=/target.test/10.0.4.2 --address=/example.test/10.0.4.2 --log-queries --log-facility=$L/dnsmasq-target.log --pid-file=$L/dnsmasq-target.pid --user=root
ip netns exec target nohup python3 -m http.server 80 --bind 10.0.4.2 --directory www >/dev/null 2>&1 &
ip netns exec target nohup openssl s_server -accept 10.0.4.2:443 -cert decoy.crt -key decoy.key -tls1_3 -quiet -www >/dev/null 2>&1 &
ip netns exec inet nohup python3 -m http.server 80 --bind 169.254.169.254 --directory meta >/dev/null 2>&1 &
ip netns exec exit nohup python3 -m http.server 8081 --bind 127.0.0.1 --directory localsecret >/dev/null 2>&1 &
sleep 1
echo services-up
