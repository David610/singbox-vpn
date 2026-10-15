import socketserver, time
from http.server import BaseHTTPRequestHandler, HTTPServer

class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def do_GET(self):
        n = 50
        self.send_response(200)
        self.send_header("Content-Length", str(n))
        self.send_header("Content-Type", "application/octet-stream")
        self.end_headers()
        for _ in range(n):
            self.wfile.write(b"x"); self.wfile.flush(); time.sleep(0.5)
    def log_message(self, *a): pass

class S(socketserver.ThreadingMixIn, HTTPServer):
    daemon_threads = True
    allow_reuse_address = True

S(("0.0.0.0", 9555), H).serve_forever()
