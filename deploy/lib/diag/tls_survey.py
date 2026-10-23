import socket,ssl,time,json,sys,threading,collections,queue
ips=json.load(open(sys.argv[1]))
YT=['www.youtube.com','i.ytimg.com','youtubei.googleapis.com','rr1---sn-aaaa.googlevideo.com']
CT=['www.google.com','example.com','']
ctx=ssl.create_default_context();ctx.check_hostname=False;ctx.verify_mode=ssl.CERT_NONE
def hs(ip,sni):
    try:
        s=socket.create_connection((ip,443),3.5);s.settimeout(4)
        w=ctx.wrap_socket(s,server_hostname=sni or None);w.close();return 'ok'
    except socket.timeout:return 'noreply'
    except ConnectionResetError:return 'reset'
    except ssl.SSLError:return 'sslerr'
    except OSError:return 'noconnect'
q=queue.Queue();res=[];lk=threading.Lock()
for ip in ips:
    for s in YT+CT:q.put((ip,s))
def w():
    while True:
        try:ip,s=q.get_nowait()
        except queue.Empty:return
        r=hs(ip,s)
        with lk:res.append((ip,s,r))
ts=[threading.Thread(target=w) for _ in range(24)];[t.start() for t in ts];[t.join() for t in ts]
rng=lambda ip:'.'.join(ip.split('.')[:2])
tab=collections.defaultdict(lambda:collections.Counter())
for ip,s,r in res:
    cls='YOUTUBE-SNI' if s in YT else 'control'
    tab[(rng(ip),cls)][r]+=1
print('%-10s %-12s %s'%('range','class','results'))
for k in sorted(tab):print('%-10s %-12s %s'%(k[0],k[1],dict(tab[k])))
tot=collections.Counter();[tot.update({('YT' if s in YT else 'CT',r):1}) for ip,s,r in res];print('TOTAL',dict(tot))
json.dump(res,open('/tmp/tls_survey_%s.json'%sys.argv[2],'w'))
