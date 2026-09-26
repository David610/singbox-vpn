#!/usr/bin/env python3
"""Server-paced stream for connection-survival tests: GET /drip?n=N sends
one byte per second for N seconds (cannot be pre-buffered by the client)."""
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_GET(self):
        n = int(self.path.split("n=")[-1]) if "n=" in self.path else 60
        self.send_response(200); self.send_header("Content-Length", str(n)); self.end_headers()
        for _ in range(n):
            self.wfile.write(b"x"); self.wfile.flush(); time.sleep(1)
ThreadingHTTPServer(("0.0.0.0", 9555), H).serve_forever()
