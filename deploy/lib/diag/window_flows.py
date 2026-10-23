import struct,sys,socket,collections
sys.path.insert(0,'/root/work/analysis')
path,a,b=sys.argv[1],float(sys.argv[2]),float(sys.argv[3])
def rd(p):
    f=open(p,'rb');g=f.read(24);e='<' if struct.unpack('<I',g[:4])[0] in(0xa1b2c3d4,0xa1b23c4d) else '>'
    nano=struct.unpack(e+'I',g[:4])[0]==0xa1b23c4d
    while True:
        h=f.read(16)
        if len(h)<16:return
        ts,tf,i,o=struct.unpack(e+'IIII',h);d=f.read(i)
        yield ts+tf/(1e9 if nano else 1e6),d
pk=list(rd(path));t0=pk[0][0]
W=collections.defaultdict(lambda:[0,0,0,0,None,None])
for ts,d in pk:
    r=ts-t0
    ip=d[14:];ihl=(ip[0]&15)*4;tl=struct.unpack('>H',ip[2:4])[0];t=ip[ihl:]
    sp,dp=struct.unpack('>HH',t[:4]);fl=struct.unpack('>H',t[12:14])[0];doff=(fl>>12)*4;pl=tl-ihl-doff
    port=dp if sp==443 else sp;up=(sp!=443)
    w=W[port]
    if a<=r<=b:
        if up:w[0]+=pl;w[2]+=1
        else:w[1]+=pl;w[3]+=1
        if w[4] is None:w[4]=r
        w[5]=r
for port,w in sorted(W.items(),key=lambda x:-x[1][0]):
    if w[0] or w[1]:print('fin1:%d up %6dB/%3dp down %7dB/%3dp seen %.1f-%.1fs'%(port,w[0],w[2],w[1],w[3],w[4],w[5]))
