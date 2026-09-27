import socket, struct, time, sys
port=int(sys.argv[1]); n=int(sys.argv[2])
def req():
    s=socket.create_connection(('127.0.0.1',port)); s.sendall(b'\x05\x01\x00'); s.recv(2)
    s.sendall(b'\x05\x01\x00\x01'+socket.inet_aton('10.0.4.2')+struct.pack('>H',80)); r=s.recv(10)
    s.sendall(b'GET / HTTP/1.0\r\nHost: t\r\n\r\n'); d=b''
    while True:
        c=s.recv(4096)
        if not c: break
        d+=c
    s.close(); return b'TARGET-OK' in d
lat=[]; ok=0; t0=time.time()
for i in range(n):
    a=time.time(); ok+=req(); lat.append((time.time()-a)*1000)
el=time.time()-t0; lat.sort()
print(f"port {port}: {n} sequential new connections, ok={ok}, {n/el:.0f} conn/s, p50={lat[n//2]:.1f}ms p95={lat[int(n*.95)]:.1f}ms")
