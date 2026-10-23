import socket,json
names=['www.youtube.com','m.youtube.com','youtubei.googleapis.com','i.ytimg.com','i9.ytimg.com','yt3.ggpht.com','s.youtube.com','redirector.googlevideo.com','manifest.googlevideo.com','www.google.com','www.gstatic.com','accounts.google.com','play.google.com','fonts.gstatic.com','lh3.googleusercontent.com','suggestqueries-clients6.youtube.com','jnn-pa.googleapis.com','rr1.googlevideo.com','rr2.googlevideo.com','r1---sn-4g5e6nzz.googlevideo.com']
ips=set()
for n in names:
    try:
        for a in socket.getaddrinfo(n,443,socket.AF_INET):ips.add(a[4][0])
    except Exception:pass
print(json.dumps(sorted(ips)))
