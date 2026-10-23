import socket,ssl,time,sys
ctx=ssl.create_default_context();ctx.check_hostname=False;ctx.verify_mode=ssl.CERT_NONE
def hs(ip,sni):
    t=time.time()
    try:
        s=socket.create_connection((ip,443),4);s.settimeout(5)
        w=ctx.wrap_socket(s,server_hostname=sni);w.close();return 'OK(%dms)'%((time.time()-t)*1000)
    except socket.timeout:return 'TIMEOUT-no-ServerHello'
    except ssl.SSLError as e:return 'SSLERR'
    except ConnectionResetError:return 'RST'
    except Exception as e:return type(e).__name__
ips=['142.251.145.6','108.177.14.101','209.85.233.132','173.194.160.162','188.43.61.144']
snis=['www.youtube.com','i.ytimg.com','rr1---sn-aaaa.googlevideo.com','www.google.com','example.com',None]
print('%-26s'%'SNI \\ IP'+''.join('%-26s'%i for i in ips))
for sn in snis:
    print('%-26s'%(sn or '(no SNI)')[:25]+''.join('%-26s'%hs(i,sn) for i in ips))
