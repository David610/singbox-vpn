import socket,os,sys,time
host=sys.argv[1];fam=socket.AF_INET6 if ':' in host else socket.AF_INET
res=[]
for i in range(3):
    s=socket.socket(fam,socket.SOCK_DGRAM);s.settimeout(3)
    dcid=os.urandom(8);scid=os.urandom(8)
    pkt=bytes([0xc0])+(0x1a2a3a4a).to_bytes(4,'big')+bytes([8])+dcid+bytes([8])+scid
    pkt+=b'\x00'*(1200-len(pkt))
    t=time.time()
    try:
        s.sendto(pkt,(host,443));d,a=s.recvfrom(2000)
        ok=len(d)>5 and d[1:5]==b'\x00\x00\x00\x00'
        res.append('VN-reply(%dB,%.0fms)'%(len(d),(time.time()-t)*1000) if ok else 'other-reply')
    except Exception as e:res.append('NO-REPLY(%s)'%type(e).__name__)
print(host,res)
