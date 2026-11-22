#!/usr/bin/env python3
"""B2 development-cache tooling for the Velox project (Phase 1).

Native B2 API v2/v3 via stdlib only. Credentials come from /home/z/my-project/.secrets/b2.env
(B2_KEY_ID, B2_APPLICATION_KEY, B2_BUCKET) — never printed, logged or committed.
This is DEVELOPMENT storage only; Velox the product never touches it.

Commands:
  verify          authorize + exact-bucket-name check + upload/download/checksum selftest
  push DIR PREFIX upload DIR/** as PREFIX/... (sha256 recorded)
  pull PREFIX DIR restore files under PREFIX into DIR (skips existing identical)
  manifest        write cache-manifest.json (local index of PREFIX with sha256)
"""
import base64
import hashlib
import json
import os
import pathlib
import sys
import time
import urllib.request
import urllib.error

SECRETS = pathlib.Path("/home/z/my-project/.secrets")
ENV_FILE = SECRETS / "b2.env"
STATE_FILE = SECRETS / "b2-state.json"  # auth cache (short-lived), still 0600, git-ignored
API = "https://api.backblazeb2.com/b2api/v3"


def load_env():
    env = {}
    for line in ENV_FILE.read_text().splitlines():
        line = line.strip()
        if line and not line.startswith("#") and "=" in line:
            k, v = line.split("=", 1)
            env[k] = v
    return env


def save_state(auth):
    STATE_FILE.write_text(json.dumps(auth))
    os.chmod(STATE_FILE, 0o600)


def load_state():
    if STATE_FILE.exists():
        try:
            st = json.loads(STATE_FILE.read_text())
            if st.get("expiresAt", 0) > time.time() + 60:
                return st
        except Exception:
            pass
    return None


def api(auth, name, body=None):
    req = urllib.request.Request(
        f"{auth['apiUrl']}/b2api/v3/{name}" if auth else f"{API}/{name}",
        data=json.dumps(body or {}).encode(),
        headers={"Authorization": auth["authorizationToken"]} if auth
        else {"Authorization": "Basic " + base64.b64encode(
            f"{os.environ['B2_KEY_ID']}:{os.environ['B2_APPLICATION_KEY']}".encode()).decode()},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return json.loads(r.read())
    except urllib.error.HTTPError as e:
        detail = e.read().decode(errors="replace")[:300]
        raise RuntimeError(f"b2 {name} → HTTP {e.code}: {detail}") from None


def authorize():
    st = load_state()
    if st:
        return st
    auth = api(None, "b2_authorize_account")
    # v3 nests the storage endpoints under apiInfo.storageApi — flatten for v2-style use
    storage = auth.get("apiInfo", {}).get("storageApi", {})
    auth = {**auth, **storage}
    if "region" not in auth:
        host = auth.get("s3ApiUrl", "")
        # https://s3.us-west-004.backblazeb2.com → us-west-004
        parts = host.replace("https://", "").split(".")
        if len(parts) > 2 and parts[0] == "s3":
            auth["region"] = parts[1]
    save_state(auth)
    return auth


def bucket(auth, name):
    # This application key is bucket-scoped: b2_list_buckets returns 401 for it, but
    # b2_authorize_account pre-resolves the allowed bucket (bucketId/bucketName).
    pre_id, pre_name = auth.get("bucketId"), auth.get("bucketName")
    if pre_id and pre_name:
        match = {"bucketId": pre_id, "bucketName": pre_name, "bucketType": "pre-resolved"}
        return (match if pre_name == name else None), [pre_name] if pre_name else []
    buckets = api(auth, "b2_list_buckets", {"accountId": auth["accountId"]})["buckets"]
    exact = [b for b in buckets if b["bucketName"] == name]
    near = [b["bucketName"] for b in buckets if name.lower() in b["bucketName"].lower()]
    return (exact[0] if exact else None), near


def sha256(p):
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def upload_file(auth, bucket_id, key, path):
    path = pathlib.Path(path)
    size = path.stat().st_size
    if size <= 100_000_000:  # single-shot upload (B2 native limit is 200 MB+; 100 MB is a safe proxy cut)
        return _upload_small(auth, bucket_id, key, path)
    return _upload_large(auth, bucket_id, key, path)


def _upload_small(auth, bucket_id, key, path):
    with open(path, "rb") as f:
        data = f.read()
    sha1 = hashlib.sha1(data).hexdigest()
    up = api(auth, "b2_get_upload_url", {"bucketId": bucket_id})

    def _send():
        req = urllib.request.Request(
            up["uploadUrl"], data=data,
            headers={
                "Authorization": up["authorizationToken"],
                "X-Bz-File-Name": urllib.request.quote(key),
                "Content-Type": "b2/x-auto",
                "X-Bz-Content-Sha1": sha1,
            }, method="POST")
        with urllib.request.urlopen(req, timeout=600) as r:
            return json.loads(r.read())

    return _retry(_send)


def _retry(fn, tries=4):
    for i in range(tries):
        try:
            return fn()
        except (urllib.error.HTTPError, urllib.error.URLError, OSError) as e:
            code = getattr(e, "code", None)
            if code is not None and code not in (401, 400):
                time.sleep(2 ** i * 2)
                continue
            raise
    raise RuntimeError("retries exhausted")


def _upload_large(auth, bucket_id, key, path):
    """b2_start_large_file → upload 100 MB parts → b2_finish_large_file."""
    part_size = 100_000_000
    start = api(auth, "b2_start_large_file", {
        "bucketId": bucket_id,
        "fileName": key,
        "contentType": "b2/x-auto",
        "fileInfo": {"sha256": sha256(path)},
    })
    file_id, sha1s, off, part_no = start["fileId"], [], 0, 1
    with open(path, "rb") as f:
        while off < path.stat().st_size:
            f.seek(off)
            chunk = f.read(part_size)
            part = api(auth, "b2_get_upload_part_url", {"fileId": file_id})
            sha1 = hashlib.sha1(chunk).hexdigest()
            resp = _retry(lambda: (lambda req: json.loads(
                urllib.request.urlopen(req, timeout=900).read()
            ))(urllib.request.Request(
                part["uploadUrl"], data=chunk,
                headers={
                    "Authorization": part["authorizationToken"],
                    "X-Bz-Part-Number": str(part_no),
                    "X-Bz-Content-Sha1": sha1,
                }, method="POST")))
            sha1s.append(resp["contentSha1"])
            off += len(chunk)
            part_no += 1
    return api(auth, "b2_finish_large_file", {"fileId": file_id, "partSha1Array": sha1s})


def download_file(auth, bucket_name, key, dest):
    url = f"{auth['downloadUrl']}/file/{urllib.request.quote(bucket_name)}/{urllib.request.quote(key)}"
    req = urllib.request.Request(url, headers={"Authorization": auth["authorizationToken"]})
    with urllib.request.urlopen(req, timeout=300) as r:
        data = r.read()
    dest = pathlib.Path(dest)
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_bytes(data)
    return dest


def list_files(auth, bucket_id, prefix):
    out, start = [], None
    while True:
        body = {"bucketId": bucket_id, "prefix": prefix, "maxFileCount": 1000}
        if start:
            body["startFileName"] = start
        resp = api(auth, "b2_list_file_names", body)
        out += resp["files"]
        if resp.get("nextFileName"):
            start = resp["nextFileName"]
        else:
            return out


def cmd_verify(env):
    auth = authorize()
    print("authorize: OK (apiurl=%s, s3=%s)" % (auth["apiUrl"], auth.get("s3ApiUrl", "n/a")))
    name = env["B2_BUCKET"]
    b, near = bucket(auth, name)
    if not b:
        print(f"BUCKET MISMATCH: exact name '{name}' not found; near matches: {near}")
        return 2
    print(f"bucket: exact match '{b['bucketName']}' id={b['bucketId']} type={b['bucketType']}")
    # round-trip: upload -> download -> both checksums -> delete
    key = f"selftest/roundtrip-{int(time.time())}.bin"
    payload = f"velox-b2-selftest {time.time()}\n".encode() * 100
    tmp = SECRETS / "_selftest.bin"
    tmp.write_bytes(payload)
    want_sha256 = hashlib.sha256(payload).hexdigest()
    up = upload_file(auth, b["bucketId"], key, tmp)
    got = download_file(auth, name, key, SECRETS / "_selftest-dl.bin")
    got_sha256 = sha256(got)
    ok = got_sha256 == want_sha256 and hashlib.sha1(payload).hexdigest() == up["contentSha1"]
    api(auth, "b2_delete_file_version", {"fileName": key, "fileId": up["fileId"]})
    tmp.unlink()
    got.unlink()
    print(f"round-trip: {'OK' if ok else 'FAILED'} ({len(payload)}B, sha256 {want_sha256[:12]}…, deleted)")
    # persist discovered s3 endpoint/region back into b2.env blanks
    s3 = auth.get("s3ApiUrl", "")
    region = auth.get("region", "")
    txt = ENV_FILE.read_text()
    if s3:
        txt = txt.replace("B2_ENDPOINT=", f"B2_ENDPOINT={s3}", 1)
    if region:
        txt = txt.replace("B2_REGION=", f"B2_REGION={region}", 1)
    ENV_FILE.write_text(txt)
    return 0 if ok else 3


def cmd_push(env, src, prefix):
    auth = authorize()
    b, _ = bucket(auth, env["B2_BUCKET"])
    if not b:
        print("bucket not found"); return 2
    src, n, failed = pathlib.Path(src), 0, []
    # idempotency: skip files already present with the same sha1
    existing = {f["fileName"]: f["contentSha1"] for f in list_files(auth, b["bucketId"], prefix)}
    for p in sorted(src.rglob("*")):
        if p.is_file() and p.stat().st_size > 0:
            key = f"{prefix}/{p.relative_to(src)}"
            local_sha1 = hashlib.sha1(p.read_bytes()).hexdigest()
            if existing.get(key) == local_sha1:
                print(f"  = {key} (already cached, skipped)")
                continue
            try:
                upload_file(auth, b["bucketId"], key, p)
                n += 1
                print(f"  + {key} ({p.stat().st_size}B)")
            except Exception as e:
                failed.append(key)
                print(f"  ! {key} FAILED: {str(e)[:120]}")
    print(f"pushed {n} files under {prefix}/" + (f" — {len(failed)} FAILED: {failed}" if failed else ""))
    return 1 if failed else 0


def cmd_pull(env, prefix, dest):
    auth = authorize()
    b, _ = bucket(auth, env["B2_BUCKET"])
    if not b:
        print("bucket not found"); return 2
    dest = pathlib.Path(dest)
    n = 0
    for f in list_files(auth, b["bucketId"], prefix):
        rel = f["fileName"][len(prefix):].lstrip("/")
        d = dest / rel
        if d.exists() and sha256(d) == hashlib.sha256(b"").hexdigest():
            pass  # empty-marker skip; real compare below
        if d.exists() and d.stat().st_size == f["contentLength"]:
            n += 1  # already present, same size → skip (fast restore)
            continue
        download_file(auth, env["B2_BUCKET"], f["fileName"], d)
        n += 1
        print(f"  ↓ {rel} ({f['contentLength']}B)")
    print(f"restored {n} files to {dest}")
    return 0


def cmd_manifest(env, prefix):
    auth = authorize()
    b, _ = bucket(auth, env["B2_BUCKET"])
    files = [{"key": f["fileName"], "size": f["contentLength"], "sha1": f["contentSha1"],
              "uploaded": f["uploadTimestamp"]}
             for f in list_files(auth, b["bucketId"], prefix)]
    out = {"bucket": env["B2_BUCKET"], "prefix": prefix, "generated": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
           "files": files}
    path = pathlib.Path("cache-manifest.json")
    path.write_text(json.dumps(out, indent=2))
    print(f"cache-manifest.json: {len(files)} objects, {sum(f['size'] for f in files)/1e6:.1f} MB")


if __name__ == "__main__":
    env = load_env()
    os.environ.setdefault("B2_KEY_ID", env["B2_KEY_ID"])
    os.environ.setdefault("B2_APPLICATION_KEY", env["B2_APPLICATION_KEY"])
    cmd = sys.argv[1]
    if cmd == "verify":
        sys.exit(cmd_verify(env))
    elif cmd == "push":
        sys.exit(cmd_push(env, sys.argv[2], sys.argv[3]))
    elif cmd == "pull":
        sys.exit(cmd_pull(env, sys.argv[2], sys.argv[3]))
    elif cmd == "manifest":
        sys.exit(cmd_manifest(env, sys.argv[2] if len(sys.argv) > 2 else ""))
    else:
        print(__doc__); sys.exit(64)
