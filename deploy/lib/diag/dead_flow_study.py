import socket,os,time,sys,threading,json,random
IPS=['142.251.155.119','142.251.150.4','142.250.74.206','142.251.151.4']
def mkvn(size):
    v=bytes([0xc0])+(0x1a2a3a4a).to_bytes(4,'big')+bytes([8])+os.urandom(8)+bytes([8])+os.urandom(8);return v+b'\0'*(size-len(v))
def flow(ip,size,port=0):
    u=socket.socket(socket.AF_INET,socket.SOCK_DGRAM)
    try:u.bind(('',port))
    except OSError:u.close();return None
    p=u.getsockname()[1];vn=mkvn(size)
    for gap in (1,2,4,8):
        u.sendto(vn,(ip,443));u.settimeout(gap)
        try:u.recvfrom(2000);u.close();return dict(ip=ip,port=p,size=size,ok=True)
        except Exception:pass
    u.close();return dict(ip=ip,port=p,size=size,ok=False)
def batch(n,workers,size,ips=IPS):
    res=[];lk=threading.Lock()
    def w(k):
        for j in range(n):
            r=flow(ips[(j+k)%len(ips)],size)
            if r:
                with lk:res.append(r)
    ts=[threading.Thread(target=w,args=(k,)) for k in range(workers)];[t.start() for t in ts];[t.join() for t in ts];return res
def rep(label,res):
    bad=[r for r in res if not r['ok']];by={}
    for r in res:by.setdefault(r['ip'],[0,0]);by[r['ip']][0]+=1;by[r['ip']][1]+=(not r['ok'])
    print('%-34s flows=%4d dead=%3d (%.2f%%) per-ip=%s'%(label,len(res),len(bad),100*len(bad)/max(1,len(res)),{k:'%d/%d'%(v[1],v[0]) for k,v in by.items()}));return bad
out={}
out['serial_1200']=rep('serial 1 worker, 1200B',batch(300,1,1200))
out['par12_1200']=rep('12 workers, 1200B',batch(60,12,1200))
out['par12_1350']=rep('12 workers, 1350B',batch(60,12,1350))
out['par12_100']=rep('12 workers, 100B (small)',batch(60,12,100))
# replay dead source ports: same 5-tuple again
dead=out['par12_1200']+out['serial_1200']+out['par12_1350']+out['par12_100']
time.sleep(2);again=[]
for r in dead:
    x=flow(r['ip'],1200,r['port'])
    if x:again.append(x)
print('replay of %d previously-dead (ip,port) pairs: still dead=%d  recovered=%d'%(len(again),sum(1 for x in again if not x['ok']),sum(1 for x in again if x['ok'])))
# control: same ports with a different destination IP
alt=[];
for r in dead:
    ip2=[i for i in IPS if i!=r['ip']][0];x=flow(ip2,1200,r['port'])
    if x:alt.append(x)
print('same dead source ports -> OTHER dest IP: dead=%d of %d'%(sum(1 for x in alt if not x['ok']),len(alt)))
json.dump(out,open('/tmp/dead_study.json','w'))
