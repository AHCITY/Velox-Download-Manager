#!/usr/bin/env python3
"""Velox mock engine server — serves the real web/ frontend against a fake
in-memory engine over the browser transport (POST /rpc + SSE /events).

Used for UI screenshots and manual preview: the real app is Windows-only,
this lets the exact production frontend run in a headless browser with
realistic rows (YouTube thumbnail, failed row with friendly error, merging
row, completed row) and live progress events.
"""
import json, random, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

WEB = Path('/home/z/my-project/download-engine/web')

NOW = time.time()
def iso(secs_ago=0): return time.strftime('%Y-%m-%dT%H:%M:%S', time.gmtime(NOW - secs_ago)) + 'Z'

# id, name, url, state, received, total, bps, extra
ROWS = [
    dict(id=1, url='https://www.youtube.com/watch?v=dQw4w9WgXcQ',
         final_name='Never Gonna Give You Up.mp4', state='downloading',
         received=48_331_520, total_size=92_730_368, bps=2_621_440, bps_smoothed=2_400_000,
         dest_dir='C:\\Users\\Demo\\Downloads\\Velox', mode='segments', resumable='yes',
         accept_ranges='yes', connections_active=8, segments=[dict(idx=i, start=i*10_000_000, pos=i*10_000_000+ (1_000_000 if i<4 else 0), end=(i+1)*10_000_000-1, completed=False) for i in range(9)],
         sha256=None, blake3=None, verified=False, etag='"a83f-61d2"', last_modified=None,
         error_message=None, error_code=None, recent_errors=[], file_missing=False,
         created_at=iso(3600), updated_at=iso(5), completed_at=None, bps_avg=1_900_000, bps_peak=3_400_000,
         resolved_url=None, bandwidth_limit_bps=None),
    dict(id=2, url='https://www.youtube.com/watch?v=aqz-KE-bpKQ',
         final_name='Big Buck Bunny 1080p.mp4', state='completed',
         received=158_334_976, total_size=158_334_976, bps=None, bps_smoothed=None,
         dest_dir='C:\\Users\\Demo\\Downloads\\Velox', mode='segments', resumable='yes',
         accept_ranges='yes', connections_active=0, segments=[],
         sha256='9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08', blake3=None, verified=True,
         etag='"c19d-52fa"', last_modified=None, error_message=None, error_code=None,
         recent_errors=[], file_missing=False, created_at=iso(7200), updated_at=iso(1800),
         completed_at=iso(1800), bps_avg=2_100_000, bps_peak=3_900_000,
         resolved_url=None, bandwidth_limit_bps=None),
    dict(id=3, url='https://cdn.example.com/torrent-client-guide.pdf',
         final_name='torrent-client-guide.pdf', state='failed',
         received=1_048_576, total_size=5_242_880, bps=None,
         dest_dir='C:\\Users\\Demo\\Downloads\\Velox', mode='segments', resumable='yes',
         accept_ranges='yes', connections_active=0, segments=[],
         sha256=None, blake3=None, verified=False, etag=None, last_modified=None,
         error_message='HTTP 403 Forbidden: server refused the request', error_code='HTTP',
         recent_errors=['[12:04:31] attempt 1: connect: connection refused by peer',
                        '[12:05:02] attempt 2: HTTP 403 Forbidden: server refused the request'],
         file_missing=False, created_at=iso(5400), updated_at=iso(60), completed_at=None,
         bps_avg=None, bps_peak=None, resolved_url=None, bandwidth_limit_bps=None),
    dict(id=4, url='https://www.youtube.com/watch?v=9bZkp7q19f0',
         final_name='PSY - GANGNAM STYLE.mp4', state='merging',
         received=88_080_384, total_size=88_080_384, bps=None,
         dest_dir='C:\\Users\\Demo\\Downloads\\Velox', mode='media', resumable='yes',
         accept_ranges='yes', connections_active=0, segments=[],
         sha256=None, blake3=None, verified=False, etmtag=None, etag='"77aa-11bb"', last_modified=None,         error_message=None, error_code=None, recent_errors=[], file_missing=False,
         created_at=iso(900), updated_at=iso(3), completed_at=None,
         bps_avg=1_600_000, bps_peak=2_800_000, resolved_url=None, bandwidth_limit_bps=None),
    dict(id=5, url='https://releases.example.com/app-setup-2.1.exe',
         final_name='app-setup-2.1.exe', state='paused',
         received=34_603_008, total_size=115_343_360, bps=None,
         dest_dir='C:\\Users\\Demo\\Downloads\\Velox', mode='segments', resumable='user',
         accept_ranges='yes', connections_active=0, segments=[],
         sha256=None, blake3=None, verified=False, etag='"1f3a-9c2d"', last_modified=None,
         error_message=None, error_code=None, recent_errors=[], file_missing=False,
         created_at=iso(2400), updated_at=iso(120), completed_at=None,
         bps_avg=900_000, bps_peak=2_100_000, resolved_url=None, bandwidth_limit_bps=None),
    dict(id=6, url='https://www.youtube.com/watch?v=jNQXAC9IVRw',
         final_name='Me at the zoo.mp4', state='queued',
         received=0, total_size=19_749_472, bps=None,
         dest_dir='C:\\Users\\Demo\\Downloads\\Velox', mode='media', resumable='yes',
         accept_ranges='yes', connections_active=0, segments=[],
         sha256=None, blake3=None, verified=False, etag=None, last_modified=None,
         error_message=None, error_code=None, recent_errors=[], file_missing=False,
         created_at=iso(600), updated_at=iso(60), completed_at=None,
         bps_avg=None, bps_peak=None, resolved_url=None, bandwidth_limit_bps=None),
    dict(id=7, url='https://speed.hetzner.de/100MB.bin',
         final_name='100MB.bin', state='downloading',
         received=12_582_912, total_size=104_857_600, bps=1_823_104, bps_smoothed=1_700_000,
         dest_dir='C:\\Users\\Demo\\Downloads\\Velox',
         mode='segments', resumable='yes', accept_ranges='yes', connections_active=6, segments=[],
         sha256=None, blake3=None, verified=False, etag='"100mb"', last_modified=None,
         error_message=None, error_code=None, recent_errors=[], file_missing=False,
         created_at=iso(300), updated_at=iso(2), completed_at=None,
         bps_avg=1_500_000, bps_peak=2_000_000, resolved_url=None, bandwidth_limit_bps=None),
]
# scrub accidental duplicate keys with typos (kept harmless for the mock)
for r in ROWS:
    for bad in ('etmtag',):
        r.pop(bad, None)

INFO = dict(name='velox', version='0.6.0', protocol=2, features=['segments', 'media', 'verify'])
PREFS_DEFAULT = dict(notifyCompleted=True, notifyFailed=True, openFolderOnComplete=False, sidebarExpanded=True)

handlers = {}  # method -> fn(params) -> result
def method(name):
    def deco(fn): handlers[name] = fn; return fn
    return deco

@method('engine.info')
def _info(p): return INFO
@method('download.list')
def _list(p): return list(ROWS)
@method('download.status')
def _status(p):
    r = next((x for x in ROWS if x['id'] == p.get('id')), None)
    return r or dict(error=dict(code=-32602, message='NOT_FOUND: no such download'))
@method('download.inspect')
def _inspect(p):
    return dict(name='preview.bin', size=1024, accept_ranges='yes', resumable='yes')
@method('media.formats')
def _media_formats(p):
    url = p.get('url', '')
    vid = 'dQw4w9WgXcQ' if 'youtu' in url else 'mock'
    return dict(
        title='Mock Media — Format Selection Demo',
        qualities=[
            dict(label='2160p', height=2160, fps=None),
            dict(label='1440p', height=1440, fps=None),
            dict(label='1080p', height=1080, fps=None),
            dict(label='1080p60', height=1080, fps=60),
            dict(label='720p', height=720, fps=None),
            dict(label='480p', height=480, fps=None),
        ],
        has_audio_only=True,
        has_progressive=True,
        sizesByHeight={'2160': 412000000, '1440': 198000000, '1080': 134000000, '720': 72000000, '480': 38000000},
        video_id=vid,
    )
@method('download.create')
def _create(p): return dict(id=99)
@method('download.start')
def _noop(p): return True
@method('download.pause')
def _noop2(p): return True
@method('download.cancel')
def _noop3(p): return True
@method('download.remove')
def _noop4(p): return True
@method('download.verify')
def _noop5(p): return dict(ok=True)
@method('download.merge')
def _noop6(p): return True
@method('settings.get')
def _settings_get(p): return json.dumps(PREFS_DEFAULT)
@method('settings.set')
def _settings_set(p): return True

# ---------- SSE ----------
clients = []  # (wfile, lock)
clients_lock = threading.Lock()

def broadcast(event, data):
    dead = []
    with clients_lock:
        for w, lock in clients:
            try:
                with lock:
                    w.write(f"event: {event}\ndata: {json.dumps(data)}\n\n".encode())
                    w.flush()
            except Exception:
                dead.append((w, lock))
        for d in dead: clients.remove(d)

def progress_thread():
    """Advance the two downloading rows and emit progress + one completion
    during the screenshot session so the UI shows live data + the burst
    notification feature."""
    t0 = time.time()
    while True:
        time.sleep(1.0)
        for r in ROWS:
            if r['state'] == 'downloading' and r.get('bps'):
                r['received'] = min(r['total_size'], r['received'] + int(r['bps'] * random.uniform(0.7, 1.2)))
                broadcast('progress', dict(id=r['id'], received=r['received'], total=r['total_size'],
                                           bps=int(r['bps'] * random.uniform(0.75, 1.25)), segments=r['connections_active']))
                if r['received'] >= r['total_size']:
                    r['state'] = 'completed'; r['bps'] = None
                    broadcast('completed', dict(id=r['id']))
                    broadcast('removed', dict(id=r['id'] + 50))  # noise row nobody has
threading.Thread(target=progress_thread, daemon=True).start()

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def _send(self, code, body, ctype='application/json'):
        self.send_response(code)
        self.send_header('Content-Type', ctype)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def do_POST(self):
        if self.path == '/rpc':
            n = int(self.headers.get('Content-Length', 0))
            req = json.loads(self.rfile.read(n) or b'{}')
            fn = handlers.get(req.get('method'))
            if fn:
                try: result = fn(req.get('params') or {})
                except Exception as e: result = dict(error=dict(code=-32000, message=f'ENGINE: {e}'))
                resp = dict(jsonrpc='2.0', id=req.get('id'), result=result)
            else:
                resp = dict(jsonrpc='2.0', id=req.get('id'), error=dict(code=-32601, message=f'NOT_FOUND: method {req.get("method")}'))
            self._send(200, json.dumps(resp).encode())
        else:
            self._send(404, b'{}')
    def do_GET(self):
        path = self.path.split('?', 1)[0]  # browser previews append cache-busting queries
        if path == '/events':
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Cache-Control', 'no-cache')
            self.send_header('Connection', 'keep-alive')
            self.end_headers()
            lock = threading.Lock()
            entry = (self.wfile, lock)
            with clients_lock: clients.append(entry)
            try:
                self.wfile.write(b"event: hello\ndata: {}\n\n"); self.wfile.flush()
                while True:
                    time.sleep(30)
                    with lock:
                        self.wfile.write(b": ping\n\n"); self.wfile.flush()
            except Exception:
                pass
            finally:
                with clients_lock:
                    if entry in clients: clients.remove(entry)
        elif path == '/' or path == '/index.html':
            self._send(200, (WEB / 'index.html').read_bytes(), 'text/html')
        else:
            f = WEB / path.lstrip('/')
            if f.is_file() and WEB in f.resolve().parents:
                ctype = 'application/javascript' if f.suffix == '.js' else 'text/css' if f.suffix == '.css' else 'text/plain'
                self._send(200, f.read_bytes(), ctype + '; charset=utf-8')
            else:
                self._send(404, b'not found')

if __name__ == '__main__':
    srv = ThreadingHTTPServer(('127.0.6.1', 3917), Handler)
    print('mock engine on http://127.0.6.1:3917')
    srv.serve_forever()
