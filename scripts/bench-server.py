#!/usr/bin/env python3
"""Minimal Range-capable static file server for Velox benchmarks.
Serves GET with Content-Length and honors single Range: bytes=a-b / a- (206).
Usage: bench-server.py <file> <port>"""
import os
import re
import socketserver
import sys
from http.server import BaseHTTPRequestHandler

FILE = sys.argv[1]
PORT = int(sys.argv[2])
SIZE = os.path.getsize(FILE)


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self):
        start, end = 0, SIZE - 1
        rng = self.headers.get("Range")
        partial = False
        if rng:
            m = re.match(r"bytes=(\d*)-(\d*)$", rng.strip())
            if m:
                if m.group(1):
                    start = int(m.group(1))
                    end = int(m.group(2)) if m.group(2) else SIZE - 1
                else:
                    start = SIZE - int(m.group(2))
                    end = SIZE - 1
                partial = True
        end = min(end, SIZE - 1)
        length = end - start + 1
        if partial:
            self.send_response(206)
            self.send_header("Content-Range", f"bytes {start}-{end}/{SIZE}")
        else:
            self.send_response(200)
        self.send_header("Content-Length", str(length))
        self.send_header("Content-Type", "application/octet-stream")
        self.end_headers()
        with open(FILE, "rb") as f:
            f.seek(start)
            remaining = length
            while remaining > 0:
                chunk = f.read(min(1 << 20, remaining))
                if not chunk:
                    break
                self.wfile.write(chunk)
                remaining -= len(chunk)

    def log_message(self, *a):
        pass


class S(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


if __name__ == "__main__":
    S(("127.0.0.1", PORT), H).serve_forever()
