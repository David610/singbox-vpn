#!/usr/bin/env bash
# usage: wrongcert.sh <label>   -- swap the Hysteria2 cert for a valid-but-wrong-hostname one, run doctor, ALWAYS restore.
set -u
L="$1"; H=/etc/vpn/compat/hysteria; B=/root/rc2-eval/hy-backup-$L
mkdir -p $B; cp -a $H/cert.pem $H/key.pem $B/
restore() { cp -a $B/cert.pem $B/key.pem $H/; systemctl restart sing-box; sleep 3; echo "restored original certificate; sing-box $(systemctl is-active sing-box)"; }
trap restore EXIT
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 30 -subj "/CN=wrong-host.example.invalid" -addext "subjectAltName=DNS:wrong-host.example.invalid" -keyout $H/key.pem -out $H/cert.pem 2>/dev/null
chown root:sing-box $H/cert.pem $H/key.pem; chmod 0640 $H/cert.pem $H/key.pem
systemctl restart sing-box; sleep 4
echo "sing-box with WRONG-HOSTNAME Hysteria2 certificate: $(systemctl is-active sing-box)"
vpn doctor --protocol > /root/rc2-eval/doctor-wrongcert-$L.txt 2>&1; rc=$?
echo "doctor exit=$rc  OK=$(grep -c '^\[OK\]' /root/rc2-eval/doctor-wrongcert-$L.txt) FAIL=$(grep -c '^\[FAIL\]' /root/rc2-eval/doctor-wrongcert-$L.txt) WARN=$(grep -c '^\[WARN\]' /root/rc2-eval/doctor-wrongcert-$L.txt)"
grep -E '^\[(FAIL|WARN)\]|L5-6' /root/rc2-eval/doctor-wrongcert-$L.txt | cut -c1-330
