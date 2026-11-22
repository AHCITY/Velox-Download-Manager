#!/usr/bin/env python3
"""Throttled file server for UI e2e: serves /slow.bin (60MB) at ~2 MiB/s."""
import http.server, socketserver, os, time, sys

SIZE = 60 * 1024 * 1024
CHUNK = 512 * 1024
DELAY = 1.0

# self-contained fixture: guarantee the backing file exists (older sessions
# pre-created it at /tmp/vlx-e2e/files; the sandbox recycle wiped it, which made
# every request die right after the headers and broke the slow-download E2E)
FIXTURE = "/tmp/vlx-e2e/files/slow.bin"
os.makedirs(os.path.dirname(FIXTURE), exist_ok=True)
if not os.path.exists(FIXTURE) or os.path.getsize(FIXTURE) != SIZE:
    with open(FIXTURE, "wb") as f:
        f.write(b"\x00" * SIZE)

HITS = {}

class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        # /switch.bin: probe request (1st) succeeds; the real GET (2nd+) is 410 GONE
        # (a permanent HTTP error — the engine fails the row fast, no retries).
        if self.path == "/switch.bin":
            n = HITS.get(self.path, 0) + 1
            HITS[self.path] = n
            if n == 1:
                self.send_response(200)
                self.send_header("Content-Type", "application/octet-stream")
                self.send_header("Content-Length", str(4 * 1024 * 1024))
                self.send_header("Accept-Ranges", "bytes")
                self.end_headers()
                self.wfile.write(b"x" * 1024)
                return
            self.send_response(410)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        if self.path != "/slow.bin":
            self.send_error(404)
            return
        rng = self.headers.get("Range")
        start, end = 0, SIZE - 1
        partial = False
        if rng and rng.startswith("bytes="):
            a, _, b = rng[6:].partition("-")
            start = int(a) if a else 0
            end = int(b) if b else SIZE - 1
            partial = True
        length = end - start + 1
        self.send_response(206 if partial else 200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(length))
        self.send_header("Accept-Ranges", "bytes")
        if partial:
            self.send_header("Content-Range", f"bytes {start}-{end}/{SIZE}")
        self.end_headers()
        pos = start
        with open("/tmp/vlx-e2e/files/slow.bin", "rb") as f:
            f.seek(pos)
            try:
                while pos <= end:
                    n = min(CHUNK, end - pos + 1)
                    self.wfile.write(f.read(n))
                    self.wfile.flush()
                    pos += n
                    time.sleep(DELAY)
            except (BrokenPipeError, ConnectionResetError):
                pass

    def log_message(self, *a):
        pass

with socketserver.ThreadingTCPServer(("127.0.0.1", 8392), H) as srv:
    srv.allow_reuse_address = True
    print("slow server on 8392")
    srv.serve_forever()
