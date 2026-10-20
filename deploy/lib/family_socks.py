#!/usr/bin/env python3
"""Loopback SOCKS5 (no auth, CONNECT only) that dials every destination with ONE
address family, so a dual-stack machine's IPv4 and IPv6 egress can be probed
separately with youtube_shorts_probe.py (domain form: pass this proxy directly;
literal forms: add --literal-family N). Binds 127.0.0.1 only. Staging tool."""
from __future__ import annotations

import argparse
import asyncio
import socket
import struct


async def pipe(r: asyncio.StreamReader, w: asyncio.StreamWriter) -> None:
    try:
        while data := await r.read(65536):
            w.write(data)
            await w.drain()
    except (ConnectionError, asyncio.CancelledError):
        pass
    finally:
        w.close()


def handler(family: int):
    async def handle(r: asyncio.StreamReader, w: asyncio.StreamWriter) -> None:
        try:
            ver, n = await r.readexactly(2)
            await r.readexactly(n)
            w.write(b"\x05\x00")
            _, cmd, _, atyp = await r.readexactly(4)
            if atyp == 1:
                host = socket.inet_ntoa(await r.readexactly(4))
            elif atyp == 3:
                host = (await r.readexactly((await r.readexactly(1))[0])).decode()
            else:
                host = socket.inet_ntop(socket.AF_INET6, await r.readexactly(16))
            (port,) = struct.unpack("!H", await r.readexactly(2))
            fam = socket.AF_INET if family == 4 else socket.AF_INET6
            info = await asyncio.get_running_loop().getaddrinfo(host, port, family=fam, type=socket.SOCK_STREAM)
            ur, uw = await asyncio.open_connection(*info[0][4][:2])
            w.write(b"\x05\x00\x00\x01\x00\x00\x00\x00\x00\x00")
            await asyncio.gather(pipe(r, uw), pipe(ur, w))
        except Exception:
            try:
                w.write(b"\x05\x04\x00\x01\x00\x00\x00\x00\x00\x00")
            except Exception:
                pass
        finally:
            w.close()
    return handle


async def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("--family", type=int, choices=(4, 6), required=True)
    p.add_argument("--port", type=int, default=1081)
    a = p.parse_args()
    srv = await asyncio.start_server(handler(a.family), "127.0.0.1", a.port)
    async with srv:
        await srv.serve_forever()


if __name__ == "__main__":
    asyncio.run(main())
