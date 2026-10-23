import struct,sys,socket,collections
import os
RU=os.environ['RELAY_IP'];FIN=os.environ['ENTRY_IP']  # relay (egress) and entry node addresses; never hardcode
def rd(p):
    f=open(p,'rb');g=f.read(24);e='<' if struct.unpack('<I',g[:4])[0] in(0xa1b2c3d4,0xa1b23c4d) else '>';nano=struct.unpack(e+'I',g[:4])[0]==0xa1b23c4d
    while True:
        h=f.read(16)
        if len(h)<16:return
        ts,tf,i,o=struct.unpack(e+'IIII',h);d=f.read(i);yield ts+tf/(1e9 if nano else 1e6),d
F={};t0=None
for ts,d in rd(sys.argv[1]):
    t0=t0 or ts;ip=d[14:]
    if not ip or ip[0]>>4!=4 or ip[9]!=6:continue
    ihl=(ip[0]&15)*4;tl=struct.unpack('>H',ip[2:4])[0];s=socket.inet_ntoa(ip[12:16]);dd=socket.inet_ntoa(ip[16:20]);t=ip[ihl:]
    if FIN in (s,dd):continue
    sp,dp=struct.unpack('>HH',t[:4]);seq=struct.unpack('>I',t[4:8])[0];fl=struct.unpack('>H',t[12:14])[0];doff=(fl>>12)*4;pl=tl-ihl-doff;fg=fl&0x3f
    out=(s==RU);k=sp if out else dp;x=F.setdefault(k,dict(dst=dd if out else s,dport=dp if out else sp,t0=ts-t0,syn=0,synack=0,rst=0,out=0,inn=0,seqs=set(),rtx=0,last_in=None,last=ts-t0))
    x['last']=ts-t0
    if fg&2 and not fg&16:x['syn']+=1
    if fg&18==18:x['synack']+=1
    if fg&4:x['rst']+=1
    if pl>0:
        if out:
            x['out']+=pl
            if seq in x['seqs']:x['rtx']+=1
            x['seqs'].add(seq)
        else:x['inn']+=pl;x['last_in']=ts-t0
n=len(F);rt=[x for x in F.values() if x['rtx']>0]
print('TCP egress flows (excl. tunnel):',n,'| flows with data retransmissions:',len(rt),'| total rtx segs:',sum(x['rtx'] for x in rt))
print('  of those, flows that NEVER received any data back:',sum(1 for x in rt if x['inn']==0),'| received some:',sum(1 for x in rt if x['inn']>0))
print('  SYN sent but no SYN-ACK:',sum(1 for x in F.values() if x['syn'] and not x['synack']),' dport mix:',collections.Counter(x['dport'] for x in F.values()).most_common(3))
dst=collections.Counter(x['dst'].rsplit('.',2)[0] for x in rt);print('  rtx flows by dst /16:',dst.most_common(6))
print('top flows by rtx: (start s, dst, out B, in B, rtx, last-data-in s)')
for k,x in sorted(F.items(),key=lambda kv:-kv[1]['rtx'])[:12]:print('  +%5.1f %-16s out %6d in %7d rtx %3d lastin %s'%(x['t0'],x['dst'],x['out'],x['inn'],x['rtx'],('%.1f'%x['last_in']) if x['last_in'] else '-'))
ok=[x for x in F.values() if x['inn']>50000];print('bulk flows (>50KB in):',len(ok),[(round(x['t0']),x['dst'],x['inn']//1000,'KB rtx',x['rtx']) for x in ok][:8])
