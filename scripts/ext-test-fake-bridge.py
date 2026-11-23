#!/usr/bin/env python3
"""Fake Velox loopback bridge for the extension E2E (mimics the desktop app).

Writes the real bridge.json layout (~/.local/share/io.velox.downloadmanager/
data/bridge.json), serves POST /rpc with the bearer token and answers exactly
the whitelisted methods the real bridge allows. Every request body is logged
to /tmp/exttest/bridge-requests.log so the test can assert what arrived.

Usage: python3 ext-test-fake-bridge.py [port]   (default 9350)
"""
import json
import os
import sys
import time
import urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 9350
# optional: forward whitelisted calls to a REAL velox engine (serve-http) so the
# E2E proves the whole path extension -> host -> bridge -> engine -> disk
PROXY = os.environ.get("VELOX_FAKE_BRIDGE_PROXY", "").strip() or None
BRIDGE_FILE = os.path.expanduser("~/.local/share/io.velox.downloadmanager/data/bridge.json")
LOG = "/tmp/exttest/bridge-requests.log"
TOKEN = "e2e-token-0123456789abcdef0123456789abcdef"  # 48 chars: real tokens are >=32

RESULTS = {
    "engine.info": {"name": "velox", "version": "0.4.0-test"},
    "download.list": [
        {"id": 1, "final_name": "notes-readme.txt", "state": "completed",
         "received": 409600, "total_size": 409600},
        {"id": 2, "final_name": "big.bin", "state": "downloading",
         "received": 123456, "total_size": 2097152, "bps": 8192},
    ],
    "download.create": {"id": 42, "final_name": "big.bin", "total_size": 2097152},
    "download.status": {"id": 42, "state": "downloading"},
    "download.pause": {},
    "download.cancel": {},
}


class H(BaseHTTPRequestHandler):
    def do_POST(self):
        n = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(n).decode()
        auth = self.headers.get("Authorization", "")
        ok = auth == f"Bearer {TOKEN}"
        with open(LOG, "a") as f:
            f.write(json.dumps({"ts": round(time.time(), 3), "auth_ok": ok,
                                "body": json.loads(body) if body else None}) + "\n")
        if not ok:
            self.send_response(403)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        req = json.loads(body)
        method = req.get("method", "")
        if PROXY:
            # full pass-through to the REAL engine: the E2E then shows a real
            # engine download landing on disk, not a canned row. The engine
            # creates rows paused, so download.start follows the create.
            params = dict(req.get("params") or {})
            params.pop("auto_start", None)
            payload = json.dumps({"jsonrpc": "2.0", "id": req.get("id"),
                                  "method": method, "params": params}).encode()
            try:
                r = urllib.request.Request(PROXY, data=payload,
                                           headers={"Content-Type": "application/json"})
                with urllib.request.urlopen(r, timeout=30) as res:
                    resp = json.loads(res.read().decode())
                if method == "download.create" and resp.get("result", {}).get("id"):
                    start = json.dumps({"jsonrpc": "2.0", "id": req.get("id"),
                                        "method": "download.start",
                                        "params": {"id": resp["result"]["id"]}}).encode()
                    urllib.request.urlopen(urllib.request.Request(
                        PROXY, data=start, headers={"Content-Type": "application/json"}),
                        timeout=10)
            except Exception as e:
                resp = {"jsonrpc": "2.0", "id": req.get("id"),
                        "error": {"code": -32000, "message": f"engine proxy failed: {e}"}}
            with open(LOG, "a") as f:
                f.write(json.dumps({"ts": round(time.time(), 3), "proxy": method,
                                    "resp": resp}) + "\n")
        else:
            result = RESULTS.get(method)
            resp = (
                {"jsonrpc": "2.0", "id": req.get("id"), "result": result}
                if result is not None
                else {"jsonrpc": "2.0", "id": req.get("id"),
                      "error": {"code": -32601, "message": "method not allowed over the browser bridge"}}
            )
        data = json.dumps(resp).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *a):  # keep stdout quiet
        pass


if __name__ == "__main__":
    os.makedirs(os.path.dirname(BRIDGE_FILE), exist_ok=True)
    with open(BRIDGE_FILE, "w") as f:
        json.dump({"port": PORT, "token": TOKEN, "app_version": "0.4.0-test",
                   "pid": os.getpid()}, f)
    os.chmod(BRIDGE_FILE, 0o600)
    print(f"fake bridge on 127.0.0.1:{PORT}, bridge.json at {BRIDGE_FILE}", flush=True)
    HTTPServer(("127.0.0.1", PORT), H).serve_forever()
