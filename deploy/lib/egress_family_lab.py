#!/usr/bin/env python3
"""Local real-core policy lab; proxy outcomes are NOT native iPhone acceptance.

Input configs are production-renderer outputs in a private temporary directory.
Only the lab appends destination overrides AFTER the production policy, mapping
public test addresses to loopback responders; C-16 is retained and tested.
No public packets, credentials in argv, per-destination logs or host changes.
"""
import argparse
import contextlib
import ipaddress
import json
import os
from pathlib import Path
import socket
import ssl
import struct
import subprocess
import threading
import time


def write_json(path, value):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as handle:
        json.dump(value, handle)


def port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def fixture(family, context=None, udp=False):
    sock = socket.socket(family, socket.SOCK_DGRAM if udp else socket.SOCK_STREAM)
    sock.bind(("::1" if family == socket.AF_INET6 else "127.0.0.1", 0))
    if not udp:
        sock.listen()
    count = [0]

    def serve():
        while True:
            try:
                if udp:
                    data, peer = sock.recvfrom(65535)
                    count[0] += 1
                    sock.sendto(b"ECHO", peer)
                else:
                    conn, _ = sock.accept()
                    count[0] += 1

                    def respond(conn):
                        with contextlib.suppress(OSError, ssl.SSLError):
                            with conn:
                                conn.settimeout(2)
                                if context:
                                    conn = context.wrap_socket(conn, server_side=True)
                                    with conn:
                                        conn.recv(4096)
                                        conn.sendall(b"OK")
                                else:
                                    conn.sendall(b"BANNER")
                    threading.Thread(target=respond, args=(conn,), daemon=True).start()
            except OSError:
                return
    threading.Thread(target=serve, daemon=True).start()
    return sock, count


def encoded_address(host):
    try:
        addr = ipaddress.ip_address(host)
        return bytes([1 if addr.version == 4 else 4]) + addr.packed
    except ValueError:
        data = host.encode("ascii")
        return bytes([3, len(data)]) + data


def read_exact(sock, count):
    data = b""
    while len(data) < count:
        chunk = sock.recv(count - len(data))
        if not chunk:
            raise ConnectionError("proxy closed")
        data += chunk
    return data


def socks_connect(proxy_port, host, target_port=443, command=1):
    sock = socket.create_connection(("127.0.0.1", proxy_port), timeout=2)
    try:
        sock.sendall(b"\x05\x01\x00")
        assert read_exact(sock, 2) == b"\x05\x00"
        sock.sendall(b"\x05" + bytes([command]) + b"\x00" + encoded_address(host) + struct.pack("!H", target_port))
        reply = read_exact(sock, 4)
        if reply[1]:
            raise ConnectionError("SOCKS failure")
        if reply[3] == 1:
            bound = socket.inet_ntop(socket.AF_INET, read_exact(sock, 4))
        elif reply[3] == 4:
            bound = socket.inet_ntop(socket.AF_INET6, read_exact(sock, 16))
        else:
            bound = read_exact(sock, read_exact(sock, 1)[0]).decode("ascii")
        bound_port = struct.unpack("!H", read_exact(sock, 2))[0]
        return sock, bound, bound_port
    except BaseException:
        sock.close()
        raise


def tcp_probe(proxy, host, tls_context, sni, target_port=443):
    started = time.monotonic()
    try:
        sock, _, _ = socks_connect(proxy, host, target_port)
        with sock:
            if target_port != 443:
                success = sock.recv(6) == b"BANNER"
            else:
                with tls_context.wrap_socket(sock, server_hostname=sni) as tls:
                    tls.sendall(b"probe")
                    success = tls.recv(2) == b"OK"
        return success, time.monotonic() - started
    except (OSError, ConnectionError):
        return False, time.monotonic() - started


def udp_probe(proxy, host, payload):
    control, bound, bound_port = socks_connect(proxy, "0.0.0.0", 0, command=3)
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.settimeout(1.5)
            sock.sendto(b"\x00\x00\x00" + encoded_address(host) + struct.pack("!H", 443) + payload, (bound, bound_port))
            try:
                data, _ = sock.recvfrom(65535)
                return data.endswith(b"ECHO")
            except TimeoutError:
                return False
    finally:
        control.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sing-box", required=True)
    parser.add_argument("--configs-dir", type=Path, required=True)
    args = parser.parse_args()
    root = args.configs_dir
    os.chmod(root, 0o700)
    # Ephemeral fixture key material is held only in this private directory.
    cert, key = root / "cert.pem", root / "key.pem"
    subprocess.run(["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-days", "1", "-keyout", str(key), "-out", str(cert), "-subj", "/CN=localhost", "-addext", "subjectAltName=DNS:localhost,DNS:www.youtube.com,DNS:example.test"], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    os.chmod(key, 0o600)
    server_tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    server_tls.minimum_version = ssl.TLSVersion.TLSv1_3
    server_tls.load_cert_chain(cert, key)
    client_tls = ssl.create_default_context(cafile=str(cert))
    absent_sni_tls = ssl.create_default_context(cafile=str(cert))
    absent_sni_tls.check_hostname = False  # Intentional no-SNI control; chain still verified.
    sockets = []
    processes = []
    try:
        endpoints = {}
        for label, family, tls, udp in [
            ("decoy", socket.AF_INET, server_tls, False),
            ("v4", socket.AF_INET, server_tls, False),
            ("v6", socket.AF_INET6, server_tls, False),
            ("banner", socket.AF_INET6, None, False),
            ("udp", socket.AF_INET6, None, True),
        ]:
            sock, count = fixture(family, tls, udp)
            sockets.append(sock)
            endpoints[label] = (sock.getsockname()[1], count)
        output = subprocess.check_output([args.sing_box, "generate", "reality-keypair"], text=True)
        keys = dict(line.split(":", 1) for line in output.splitlines() if ":" in line)
        v6 = "2606:4700:4700::1111"  # Never dialed: post-policy lab override only.
        v4 = "8.8.8.8"
        checks = 0

        def start(config, name):
            path = root / (name + ".json")
            write_json(path, config)
            subprocess.run([args.sing_box, "check", "-c", str(path)], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            process = subprocess.Popen([args.sing_box, "run", "-c", str(path)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            processes.append(process)
            return process

        for policy in ["allow", "reject_google", "reject_all"]:
            config = json.loads((root / (policy + ".json")).read_text())
            # Keep the security/policy prefix; local routing overrides run only AFTER it.
            config["dns"] = {"servers": [{"type": "hosts", "tag": "lab-hosts", "predefined": {"localhost": "127.0.0.1", "fixture.test": v4, "fixture6.test": v6, "dual.test": [v4, v6]}}], "final": "lab-hosts"}
            config["route"]["default_domain_resolver"] = "lab-hosts"
            config["route"]["rules"] += [
                {"network": "udp", "action": "route-options", "override_address": "::1", "override_port": endpoints["udp"][0]},
                {"network": "tcp", "port": 12345, "action": "route-options", "override_address": "::1", "override_port": endpoints["banner"][0]},
                {"network": "tcp", "ip_version": 6, "port": 443, "action": "route-options", "override_address": "::1", "override_port": endpoints["v6"][0]},
                {"network": "tcp", "port": 443, "action": "route-options", "override_address": "127.0.0.1", "override_port": endpoints["v4"][0]},
            ]
            reality_port, hy2_port = port(), port()
            reality, hy2 = config["inbounds"]
            reality["listen"], reality["listen_port"] = "127.0.0.1", reality_port
            reality["tls"]["reality"]["private_key"] = keys["PrivateKey"].strip()
            reality["tls"]["reality"]["handshake"] = {"server": "localhost", "server_port": endpoints["decoy"][0]}
            hy2["listen"], hy2["listen_port"] = "127.0.0.1", hy2_port
            hy2["tls"]["certificate_path"], hy2["tls"]["key_path"] = str(cert), str(key)
            start(config, "server-" + policy)
            banner_config = json.loads(json.dumps(config))
            banner_reality_port, banner_hy2_port = port(), port()
            banner_config["inbounds"][0]["listen_port"] = banner_reality_port
            banner_config["inbounds"][1]["listen_port"] = banner_hy2_port
            banner_config["route"]["rules"].append({"network": "tcp", "action": "route-options", "override_address": "::1", "override_port": endpoints["banner"][0]})
            start(banner_config, "banner-server-" + policy)
            for transport in ["reality", "hysteria2"]:
                proxy = port()
                if transport == "reality":
                    outbound = {"type": "vless", "tag": "proxy", "server": "127.0.0.1", "server_port": reality_port, "uuid": reality["users"][0]["uuid"], "flow": "xtls-rprx-vision", "tls": {"enabled": True, "server_name": "localhost", "utls": {"enabled": True, "fingerprint": "chrome"}, "reality": {"enabled": True, "public_key": keys["PublicKey"].strip(), "short_id": reality["tls"]["reality"]["short_id"][0]}}}
                else:
                    outbound = {"type": "hysteria2", "tag": "proxy", "server": "127.0.0.1", "server_port": hy2_port, "password": hy2["users"][0]["password"], "tls": {"enabled": True, "server_name": "localhost", "certificate_path": str(cert)}}
                child = start({"log": {"level": "fatal"}, "inbounds": [{"type": "mixed", "listen": "127.0.0.1", "listen_port": proxy}], "outbounds": [outbound], "route": {"final": "proxy"}}, "client-" + policy + "-" + transport)
                for attempt in range(50):
                    assert child.poll() is None, "client failed to start"
                    try:
                        with socket.create_connection(("127.0.0.1", proxy), timeout=0.1):
                            break
                    except OSError:
                        time.sleep(0.02)
                cases = [(v6, "www.youtube.com", policy == "allow"), (v6, "example.test", policy != "reject_all"), (v4, "www.youtube.com", True), ("::ffff:8.8.8.8", "www.youtube.com", True), ("fixture.test", "www.youtube.com", True), ("fixture6.test", "www.youtube.com", True), ("dual.test", "www.youtube.com", True)]
                timings = {}
                for host, sni, expected in cases:
                    result, elapsed = tcp_probe(proxy, host, client_tls, sni)
                    assert result == expected, f"{policy}/{transport}: TCP family/form/SNI control failed"
                    if not expected:
                        assert elapsed < 2, "TCP proxy close exceeded bound"
                    if host == v6:
                        timings["non_google_tls" if sni == "example.test" else "google_tls"] = round(elapsed, 4)
                    checks += 1
                result, _ = tcp_probe(proxy, v6, absent_sni_tls, None)
                assert result == (policy != "reject_all"), "no-SNI policy boundary changed"
                checks += 1
                result, elapsed = tcp_probe(proxy, v6, client_tls, None, 12345)
                assert result == (policy != "reject_all"), "server-first port control failed"
                if result:
                    assert elapsed < 0.3, "non-443 server-first traffic gained sniff delay"
                timings["server_first_non443"] = round(elapsed, 4)
                checks += 1
                before = endpoints["v4"][1][0]
                rejected, _ = tcp_probe(proxy, v6, client_tls, "www.youtube.com")
                if policy != "allow":
                    assert not rejected and endpoints["v4"][1][0] == before, "reject unexpectedly steered to IPv4"
                    assert tcp_probe(proxy, v4, client_tls, "www.youtube.com")[0], "explicit simulated IPv4 retry failed"
                    checks += 1
                assert not tcp_probe(proxy, "::1", client_tls, "example.test")[0], "C-16 loopback isolation regressed"
                checks += 1
                assert udp_probe(proxy, v6, b"not-a-QUIC-initial") == (policy != "reject_all"), "UDP no-SNI control failed"
                checks += 1
                try:
                    from aioquic.quic.configuration import QuicConfiguration
                    from aioquic.quic.connection import QuicConnection
                except ImportError:
                    raise RuntimeError("aioquic==1.3.0 is required for real QUIC Initial controls; install test requirements") from None
                for sni in ["www.youtube.com", "example.test"]:
                    quic = QuicConnection(configuration=QuicConfiguration(is_client=True, server_name=sni))
                    quic.connect((v6, 443), now=time.monotonic())
                    packet = quic.datagrams_to_send(now=time.monotonic())[0][0]
                    echoed = udp_probe(proxy, v6, packet)
                    expected = policy == "allow" or (policy == "reject_google" and sni != "www.youtube.com")
                    assert echoed == expected, "QUIC SNI/literal policy control failed"
                    checks += 1
                print(f"PASS {policy}/{transport}: family/form/SNI, absent SNI, C-16, bounded TCP close, explicit retry, UDP/QUIC; no native-device claim")
                banner_proxy = port()
                banner_outbound = json.loads(json.dumps(outbound))
                banner_outbound["server_port"] = banner_reality_port if transport == "reality" else banner_hy2_port
                banner_child = start({"log": {"level": "fatal"}, "inbounds": [{"type": "mixed", "listen": "127.0.0.1", "listen_port": banner_proxy}], "outbounds": [banner_outbound], "route": {"final": "proxy"}}, "banner-client-" + policy + "-" + transport)
                for attempt in range(50):
                    assert banner_child.poll() is None, "banner client startup failed"
                    try:
                        with socket.create_connection(("127.0.0.1", banner_proxy), timeout=0.1):
                            break
                    except OSError:
                        time.sleep(0.02)
                started = time.monotonic()
                try:
                    banner_sock, _, _ = socks_connect(banner_proxy, v6, 443)
                    with banner_sock:
                        got_banner = banner_sock.recv(6) == b"BANNER"
                except (OSError, ConnectionError):
                    got_banner = False
                elapsed = time.monotonic() - started
                assert got_banner == (policy != "reject_all"), "non-TLS server-first port443 control failed"
                assert elapsed < 0.3, "port443 server-first wait exceeded sniff budget plus local transport margin"
                timings["server_first_443"] = round(elapsed, 4)
                checks += 1
                banner_child.terminate()
                banner_child.wait(timeout=5)
                print("MEASURED seconds " + json.dumps(timings, sort_keys=True))
                child.terminate()
                child.wait(timeout=5)
        print(f"LOCAL LAB PASS: {checks} assertions; IPv6 loopback exercised. UDP rejection was response absence, not demonstrated client-visible ICMP or automatic recovery.")
        return 0
    finally:
        for process in reversed(processes):
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
        for sock in sockets:
            sock.close()


if __name__ == "__main__":
    raise SystemExit(main())
