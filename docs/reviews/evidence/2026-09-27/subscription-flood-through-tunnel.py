import socket, struct, time, threading
# minimal SOCKS5 client through the local sing-box socks inbound -> exit tunnel -> 127.0.0.1:9100 on the exit
def one():
    s=socket.create_connection(('127.0.0.1',1080)); s.sendall(b'\x05\x01\x00'); s.recv(2)
    s.sendall(b'\x05\x01\x00\x01'+socket.inet_aton('127.0.0.1')+struct.pack('>H',9100)); s.recv(10)
    return s
N=0; stop=time.time()+6
def worker():
    global N
    s=one()
    while time.time()<stop:
        try:
            s.sendall(b'GET /sub/xxxxxxxxxxxxxxxxxxxxxxxxxxx HTTP/1.1\r\nHost: x\r\n\r\n')
            d=s.recv(4096)
            if not d: s=one()
            N+=1
        except Exception:
            s=one()
ts=[threading.Thread(target=worker) for _ in range(16)]
[t.start() for t in ts]; [t.join() for t in ts]
print("flood requests sent through tunnel:", N)
