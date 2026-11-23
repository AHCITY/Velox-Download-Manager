#!/usr/bin/env python3
"""Secret / credential scan of the Velox git repository (audit helper).

Scans (1) every file tracked at HEAD and (2) every text blob that exists
anywhere in history. Only file paths, line numbers, rule names and MASKED
values are printed — a real secret is never written to stdout.

Usage:  python3 scripts/audit-secret-scan.py [--history] [--max-mb 30]
"""
import collections
import pathlib
import re
import subprocess
import sys

PROJECT = pathlib.Path(__file__).resolve().parent.parent

RULES = [
    ("github_token", re.compile(r"\b(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{40,})\b")),
    ("aws_access_key", re.compile(r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b")),
    ("b2_key_id", re.compile(r"\b00[0-9a-f]{23}\b")),
    ("private_key_block", re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----")),
    ("url_with_password", re.compile(
        r"[a-z][a-z0-9+.-]*://[^/\s:@'\"<>]{1,64}:([^@\s'\"/<>]{3,})@[A-Za-z0-9.\-\[]+")),
    ("jwt", re.compile(r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b")),
    ("slack_token", re.compile(r"\bxox[baprs]-[A-Za-z0-9-]{10,}\b")),
    ("keyed_assignment", re.compile(
        r"(?i)\b(password|passwd|secret|api[_-]?key|access[_-]?token|auth[_-]?token|"
        r"client[_-]?secret|private[_-]?key|signing[_-]?key|application[_-]?key|key[_-]?id|token)\b"
        r"\s*[:=]\s*['\"]([^'\"\s]{8,})['\"]")),
]
PLACEHOLDER = re.compile(
    r"(?i)(example|placeholder|dummy|your[_-]|<|>|\$\{|%\(|\{\{|xxxx|todo|changeme|redacted|not-needed|"
    r"test[_-]?token|token[_-]?here|n/a|\.\.\.|:token:)")


def mask(value):
    if len(value) <= 6:
        return "*" * len(value)
    return f"{value[:4]}…{value[-2:]} (len {len(value)})"


def is_text(data):
    if b"\x00" in data[:8192]:
        return False
    return True


def scan(data):
    hits = []
    text = data.decode("utf-8", errors="replace")
    for name, rx in RULES:
        for m in rx.finditer(text):
            value = next((g for g in m.groups() if g), m.group(0))
            if name == "keyed_assignment" and PLACEHOLDER.search(value):
                continue
            line = text.count("\n", 0, m.start()) + 1
            hits.append((name, line, value))
    return hits


def git(*args):
    return subprocess.run(["git", "-C", str(PROJECT), *args], capture_output=True, check=True).stdout


def main():
    args = sys.argv[1:]
    with_history = "--history" in args
    max_mb = 30
    if "--max-mb" in args:
        max_mb = int(args[args.index("--max-mb") + 1])

    tracked = git("ls-files", "-z").decode().split("\0")
    print(f"== HEAD: scanning {len([t for t in tracked if t])} tracked files")
    head_hits = 0
    for path in filter(None, tracked):
        blob = git("show", f"HEAD:{path}")
        if not is_text(blob):
            continue
        for name, line, value in scan(blob):
            head_hits += 1
            print(f"  HEAD  {path}:{line}  [{name}]  {mask(value)}")
    print(f"== HEAD: {head_hits} finding(s)")

    if not with_history:
        return 0

    print(f"== HISTORY: every text blob <= {max_mb} MB reachable from any ref")
    listing = git("rev-list", "--objects", "--all").decode(errors="replace").splitlines()
    shas, names = [], {}
    for row in listing:
        parts = row.split(" ", 1)
        shas.append(parts[0])
        if len(parts) == 2:
            names[parts[0]] = parts[1]
    meta = subprocess.run(["git", "-C", str(PROJECT), "cat-file", "--batch-check=%(objectname) %(objecttype) %(objectsize)"],
                          input="\n".join(shas).encode(), capture_output=True, check=True).stdout.decode().splitlines()
    blobs = []
    for row in meta:
        sha, typ, size = row.split()
        if typ == "blob" and int(size) <= max_mb * 1024 * 1024:
            blobs.append(sha)
    cat = subprocess.Popen(["git", "-C", str(PROJECT), "cat-file", "--batch"],
                           stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    findings = collections.OrderedDict()
    scanned = 0
    for sha in blobs:
        cat.stdin.write((sha + "\n").encode())
        cat.stdin.flush()
        header = cat.stdout.readline().split()
        size = int(header[2])
        data = cat.stdout.read(size)
        cat.stdout.read(1)  # trailing newline
        if not is_text(data):
            continue
        scanned += 1
        for name, line, value in scan(data):
            key = (name, value)
            findings.setdefault(key, {"paths": set(), "lines": set()})
            findings[key]["paths"].add(names.get(sha, f"(blob {sha[:10]})"))
            findings[key]["lines"].add(line)
    cat.stdin.close()
    cat.wait()
    print(f"== HISTORY: scanned {scanned} text blobs of {len(blobs)} small blobs ({len(shas)} objects total)")
    gone = 0
    for (name, value), info in sorted(findings.items()):
        paths = sorted(info["paths"])
        still = [p for p in paths if p in set(tracked)]
        flag = "also-at-HEAD" if still else "HISTORY-ONLY"
        if not still:
            gone += 1
        shown = ", ".join(paths[:6]) + (f" … +{len(paths) - 6} paths" if len(paths) > 6 else "")
        print(f"  [{name}] {mask(value)}  {flag}  lines={sorted(info['lines'])[:8]}\n        {shown}")
    print(f"== HISTORY: {len(findings)} distinct finding(s), {gone} only in history")
    return 0


if __name__ == "__main__":
    sys.exit(main())
