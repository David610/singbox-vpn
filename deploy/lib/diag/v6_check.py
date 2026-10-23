import socket,ssl,time
ctx=ssl.create_default_context();ctx.check_hostname=False;ctx.verify_mode=ssl.CERT_NONE
for h in ('www.youtube.com','i.ytimg.com','www.google.com','youtubei.googleapis.com'):
    try:ips=sorted({a[4][0] for a in socket.getaddrinfo(h,443,socket.AF_INET6)})[:2]
    except Exception:print('v6 %-26s no-AAAA/resolve-fail'%h);continue
    for ip in ips:
        try:
            t=time.time();s=socket.create_connection((ip,443),4);w=ctx.wrap_socket(s,server_hostname=h);w.close();r='ok(%dms)'%((time.time()-t)*1000)
        except socket.timeout:r='noreply'
        except Exception as e:r=type(e).__name__
        print('v6 %-26s %-28s %s'%(h,ip,r))
