#!/usr/bin/env python3
"""Loopback-only, no-auth TCP SOCKS front for controlled literal-family probes.

Resolves domains on the probe machine and sends an IPv4/IPv6 literal to an
existing LOOPBACK SOCKS tunnel. TLS bytes/SNI remain unchanged. This tests
VPS destination egress family, not client-to-VPS transport, native-app retry,
UDP/QUIC or DNS privacy. Never use it as a public proxy or production client.
"""
import contextlib
import ipaddress
import selectors
import socket
import socketserver
import struct
import threading
import urllib.parse


def exact(sock, size):
    data = b""
    while len(data) < size:
        part = sock.recv(size - len(data))
        if not part:
            raise ConnectionError("closed SOCKS stream")
        data += part
    return data


def destination(host, port, family):
    try:
        addr = ipaddress.ip_address(host)
        if addr.version != family or getattr(addr, "ipv4_mapped", None):
            raise ValueError("literal destination does not match requested family")
    except ValueError as error:
        if ":" in host or host.replace(".", "").isdigit():
            raise ValueError("invalid or mismatched literal family") from error
        answers = socket.getaddrinfo(host, port, socket.AF_INET if family == 4 else socket.AF_INET6, socket.SOCK_STREAM)
        if not answers:
            raise ValueError("no address in requested family")
        addr = ipaddress.ip_address(answers[0][4][0])
    return bytes([1 if family == 4 else 4]) + addr.packed + struct.pack("!H", port)


class Handler(socketserver.BaseRequestHandler):
    def handle(self):
        client = self.request
        client.settimeout(10)
        try:
            version, count = exact(client, 2)
            methods = exact(client, count)
            if version != 5 or 0 not in methods:
                client.sendall(b"\x05\xff")
                return
            client.sendall(b"\x05\x00")
            version, command, reserved, kind = exact(client, 4)
            if version != 5 or command != 1 or reserved != 0:
                raise ValueError("TCP CONNECT required")
            if kind == 3:
                host = exact(client, exact(client, 1)[0]).decode("ascii")
            elif kind in (1, 4):
                host = str(ipaddress.ip_address(exact(client, 4 if kind == 1 else 16)))
            else:
                raise ValueError("invalid SOCKS address")
            port = struct.unpack("!H", exact(client, 2))[0]
            encoded = destination(host, port, self.server.family)
            with socket.create_connection(self.server.upstream, timeout=10) as upstream:
                upstream.sendall(b"\x05\x01\x00")
                if exact(upstream, 2) != b"\x05\x00":
                    raise ValueError("upstream requires unsupported authentication")
                upstream.sendall(b"\x05\x01\x00" + encoded)
                header = exact(upstream, 4)
                if header[3] == 3:
                    size = exact(upstream, 1)
                    reply = header + size + exact(upstream, size[0] + 2)
                elif header[3] in (1, 4):
                    reply = header + exact(upstream, (4 if header[3] == 1 else 16) + 2)
                else:
                    raise ValueError("invalid upstream reply")
                client.sendall(reply)
                if header[1] != 0:
                    return
                with selectors.DefaultSelector() as selector:
                    selector.register(client, selectors.EVENT_READ, upstream)
                    selector.register(upstream, selectors.EVENT_READ, client)
                    while True:
                        ready = selector.select(timeout=30)
                        if not ready:
                            return
                        for key, _ in ready:
                            data = key.fileobj.recv(65536)
                            if not data:
                                return
                            key.data.sendall(data)
        except (OSError, ValueError, UnicodeError):
            # No destination, credential, request or exception text is logged.
            with contextlib.suppress(OSError):
                client.sendall(b"\x05\x04\x00\x01\x00\x00\x00\x00\x00\x00")


class Server(socketserver.ThreadingTCPServer):
    daemon_threads = True
    allow_reuse_address = True
    def handle_error(self, request, client_address):
        pass  # Never log a request/destination on the operator's machine.


@contextlib.contextmanager
def literal_front(proxy, family):
    url = urllib.parse.urlsplit(proxy)
    if url.scheme not in ("socks5", "socks5h") or url.username or url.password or not url.port or family not in (4, 6):
        raise ValueError("use a credential-free loopback socks5 URL and family 4 or 6")
    if url.hostname != "localhost" and not ipaddress.ip_address(url.hostname).is_loopback:
        raise ValueError("upstream must be loopback")
    with Server(("127.0.0.1", 0), Handler) as server:
        server.upstream = (url.hostname, url.port)
        server.family = family
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            yield f"socks5://127.0.0.1:{server.server_address[1]}"
        finally:
            server.shutdown()
            thread.join(timeout=2)
