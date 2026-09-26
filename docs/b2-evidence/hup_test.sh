# Does SIGHUP (sing-box's in-process reload) keep open connections? It does
# not restart the process, but it re-creates every inbound.
set -u; . /root/b2/lib.sh
P0=$(sbpid)
python3 stream.py current 1 10830 40 &
S=$!
sleep 12; echo "$(ts) kill -HUP sing-box (pid $P0)"; kill -HUP "$P0"; sleep 2
echo "$(ts) pid after HUP=$(sbpid) active=$(systemctl is-active sing-box)"
wait $S
echo "--- control run, no signal:"
python3 stream.py current 1 10831 40
