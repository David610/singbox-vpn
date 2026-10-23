import socket,ssl,time,struct,select,sys
ctx=ssl.create_default_context();ctx.check_hostname=False;ctx.verify_mode=ssl.CERT_NONE
def ch(sni):
    a,b=ssl.MemoryBIO(),ssl.MemoryBIO();o=ctx.wrap_bio(a,b,server_hostname=sni)
    try:o.do_handshake()
    except ssl.SSLWantReadError:pass
    return b.read()
icmp=socket.socket(socket.AF_INET,socket.SOCK_RAW,socket.IPPROTO_ICMP);icmp.setblocking(False)
def probe(ip,sni,ttl):
    while True:
        try:icmp.recvfrom(4096)
        except BlockingIOError:break
    s=socket.create_connection((ip,443),4);s.setsockopt(socket.IPPROTO_IP,socket.IP_TTL,ttl);s.settimeout(1.2)
    lport=s.getsockname()[1];s.sendall(ch(sni));hop=None;got=None;t=time.time()
    while time.time()-t<1.4:
        r,_,_=select.select([icmp,s],[],[],0.2)
        if icmp in r:
            d,a=icmp.recvfrom(4096);ihl=(d[0]&15)*4;ty=d[ihl]
            inner=d[ihl+8:];
            if ty==11 and len(inner)>=24:
                iihl=(inner[0]&15)*4
                if struct.unpack('>H',inner[iihl:iihl+2])[0]==lport:hop=a[0]
        if s in r:
            try:
                x=s.recv(5);got=x[:1].hex() if x else 'closed'
            except Exception as e:got=type(e).__name__
            break
    s.close();return hop,got
ip=sys.argv[1]
for sni in ('www.youtube.com','www.google.com'):
    print('SNI',sni,'-> dest',ip);row=[]
    for ttl in range(1,17):
        hop,got=probe(ip,sni,ttl)
        row.append('%2d:%s'%(ttl,('ICMP '+hop) if hop else ('SERVERHELLO' if got=='16' else (got or 'silence'))))
        if got=='16':break
    print('  '+'\n  '.join(row))
