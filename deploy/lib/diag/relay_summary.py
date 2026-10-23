import struct,sys,collections,socket
import os
RU=os.environ['RELAY_IP'];FIN=os.environ['ENTRY_IP']  # relay (egress) and entry node addresses; never hardcode
def rd(p):
    f=open(p,'rb');g=f.read(24)
    if len(g)<24:return
    e='<' if struct.unpack('<I',g[:4])[0] in(0xa1b2c3d4,0xa1b23c4d) else '>';nano=struct.unpack(e+'I',g[:4])[0]==0xa1b23c4d
    while True:
        h=f.read(16)
        if len(h)<16:return
        ts,tf,i,o=struct.unpack(e+'IIII',h);d=f.read(i);yield ts+tf/(1e9 if nano else 1e6),d
udp=collections.OrderedDict();tcp=collections.OrderedDict();t0=None;sec=collections.defaultdict(lambda:[0,0,0,0])
for ts,d in rd(sys.argv[1]):
    t0=t0 or ts;ip=d[14:]
    if not ip or ip[0]>>4!=4:continue
    ihl=(ip[0]&15)*4;tl=struct.unpack('>H',ip[2:4])[0];pr=ip[9];s=socket.inet_ntoa(ip[12:16]);dd=socket.inet_ntoa(ip[16:20]);t=ip[ihl:]
    if FIN in (s,dd):continue
    sp,dp=struct.unpack('>HH',t[:4]);out=(s==RU);r=int(ts-t0)
    if pr==17:
        pl=tl-ihl-8;k=sp if out else dp;x=udp.setdefault(k,dict(dst=dd if out else s,t0=ts,out=0,inn=0,op=0,ip=0,last=ts));x['last']=ts
        if out:x['out']+=pl;x['op']+=1;sec[r][0]+=pl
        else:x['inn']+=pl;x['ip']+=1;sec[r][1]+=pl
    elif pr==6:
        fl=struct.unpack('>H',t[12:14])[0];doff=(fl>>12)*4;pl=tl-ihl-doff;seq=struct.unpack('>I',t[4:8])[0];flg=fl&0x3f
        k=sp if out else dp;x=tcp.setdefault(k,dict(dst=dd if out else s,t0=ts,out=0,inn=0,rst=0,syn=0,synack=0,seqs=set(),rtx=0,last=ts));x['last']=ts
        if flg&2 and not flg&16:x['syn']+=1
        if flg&18==18:x['synack']+=1
        if flg&4:x['rst']+=1
        if pl>0:
            if out:
                x['out']+=pl
                if (1,seq) in x['seqs']:x['rtx']+=1
                x['seqs'].add((1,seq));sec[r][2]+=pl
            else:x['inn']+=pl;sec[r][3]+=pl
n=len(udp);nr=sum(1 for x in udp.values() if x['inn']>0)
print('ru1 UDP flows: %d  with reply: %d  NO reply: %d'%(n,nr,n-nr))
for k,x in udp.items():
    if x['inn']==0 and x['op']>0:print('   no-reply udp port %s -> %s sends=%d bytes=%d at +%.1fs'%(k,x['dst'],x['op'],x['out'],x['t0']-t0))
print('ru1 TCP egress flows to servers: %d  SYN-no-SYNACK: %d  RST seen: %d  retransmitted data segs: %d'%(len(tcp),sum(1 for x in tcp.values() if x['syn'] and not x['synack']),sum(1 for x in tcp.values() if x['rst']),sum(x['rtx'] for x in tcp.values())))
if t0:
    print('per-5s (bytes)  udp_out udp_in tcp_out tcp_in')
    for b in range(0,int(max(sec)+1) if sec else 0,5):
        a=[sum(sec[i][j] for i in range(b,b+5)) for j in range(4)];print('  +%3ds  %8d %8d %8d %8d %s'%(b,*a,'<-- quiet' if sum(a)==0 else ''))
