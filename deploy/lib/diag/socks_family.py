#!/usr/bin/env python3
"""Minimal SOCKS5 (CONNECT only) that resolves + connects with ONE address family. usage: socks_family.py 4|6 PORT"""
import asyncio,socket,struct,sys
FAM={'4':socket.AF_INET,'6':socket.AF_INET6}[sys.argv[1]];PORT=int(sys.argv[2])
async def pipe(r,w):
    try:
        while (d:=await r.read(65536)):w.write(d);await w.drain()
    except Exception:pass
    finally:
        try:w.close()
        except Exception:pass
async def h(r,w):
    try:
        await r.readexactly(2+(await r.readexactly(1))[0]) if False else None
    except Exception:pass
async def handle(r,w):
    try:
        v,n=await r.readexactly(2);await r.readexactly(n);w.write(b'\x05\x00')
        v,cmd,_,at=await r.readexactly(4)
        if at==1:host=socket.inet_ntoa(await r.readexactly(4))
        elif at==3:host=(await r.readexactly((await r.readexactly(1))[0])).decode()
        else:host=socket.inet_ntop(socket.AF_INET6,await r.readexactly(16))
        port=struct.unpack('>H',await r.readexactly(2))[0]
        infos=await asyncio.get_running_loop().getaddrinfo(host,port,family=FAM,type=socket.SOCK_STREAM)
        rr,ww=await asyncio.wait_for(asyncio.open_connection(infos[0][4][0],port),8)
        w.write(b'\x05\x00\x00\x01'+b'\0'*6);await w.drain()
        await asyncio.gather(pipe(r,ww),pipe(rr,w))
    except Exception:
        try:w.write(b'\x05\x04\x00\x01'+b'\0'*6);w.close()
        except Exception:pass
async def main():
    s=await asyncio.start_server(handle,'127.0.0.1',PORT);await s.serve_forever()
asyncio.run(main())
