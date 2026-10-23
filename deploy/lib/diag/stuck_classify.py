import struct,sys,socket,collections
def rd(p):
    f=open(p,'rb');g=f.read(24);e='<' if struct.unpack('<I',g[:4])[0] in(0xa1b2c3d4,0xa1b23c4d) else '>'
    nano=struct.unpack(e+'I',g[:4])[0]==0xa1b23c4d
    while True:
        h=f.read(16)
        if len(h)<16:return
        ts,tf,i,o=struct.unpack(e+'IIII',h);d=f.read(i);yield ts+tf/(1e9 if nano else 1e6),d
F=collections.OrderedDict()
for ts,d in rd(sys.argv[1]):
    ip=d[14:];ihl=(ip[0]&15)*4;tl=struct.unpack('>H',ip[2:4])[0];t=ip[ihl:]
    sp,dp=struct.unpack('>HH',t[:4]);seq=struct.unpack('>I',t[4:8])[0];fl=struct.unpack('>H',t[12:14])[0];doff=(fl>>12)*4;pl=tl-ihl-doff;flg=fl&0x3f
    up=sp!=443;port=dp if sp==443 else sp
    x=F.setdefault(port,dict(t0=ts,ev=[],seqs=set(),rtx=0,end=ts,fin_by=None))
    x['end']=ts
    if flg&5: x['fin_by']=('fin1' if up else 'ru1')+('-RST' if flg&4 else '-FIN') if x['fin_by'] is None else x['fin_by']
    if pl>0:
        k=(up,seq)
        if k in x['seqs']:x['rtx']+=1
        x['seqs'].add(k);x['ev'].append((ts-x['t0'],'U' if up else 'D',pl))
cls=collections.Counter();rtx=sum(x['rtx'] for x in F.values())
doubling=0;ex=[]
for port,x in F.items():
    ev=x['ev'];down=sum(p for _,d,p in ev if d=='D');ups=[(t,p) for t,d,p in ev if d=='U']
    # app data after the 3-record REALITY/Vision preamble: ignore first 4 upstream records (ClientHello,64,59,first data)
    post=[t for t,p in ups[3:]]
    gaps=[round(b-a,1) for a,b in zip(post,post[1:])]
    life=x['end']-x['t0']
    if down<=2300 and sum(p for _,p in ups)>2500:
        # stuck candidate
        g=[round(b-a,2) for a,b in zip(post,post[1:])]
        dbl=len(g)>=2 and all(1.5<=g[i+1]/max(g[i],0.01)<=2.6 for i in range(len(g)-1))
        cls['no-reply, repeated app msgs with ~doubling gaps']+=dbl
        cls['no-reply, other pattern (idle/short/single send)']+= (not dbl)
        if dbl and len(ex)<3:ex.append((port,[round(t,1) for t in post],x['fin_by']))
    elif down>20000:cls['bulk (>20KB down)']+=1
    else:cls['normal small exchange']+=1
print(sys.argv[1],'flows',len(F),'TCP-level retransmitted data segments:',rtx)
for k,v in cls.items():print('  %4d  %s'%(v,k))
print('  examples (fin1 port, post-handshake send offsets s, closed by):',ex)
ends=collections.Counter(x['fin_by'] for x in F.values());print('  closed by:',dict(ends))
