#!/usr/bin/env python3
"""Empirical minisign variant resolution for the tauri 'ED' signature format.

rust-minisign defines two signature algorithms:
  "Ed" — Ed25519 over the raw content
  "ED" — Ed25519 over BLAKE2b-512(content) (streaming/large-file variant)
This script brute-forces the structured candidate space against the shipped
pubkey and reports which interpretation validates the published installer.
"""
import base64
import hashlib
import json
import re
import sys
from pathlib import Path

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

RELEASE = Path('/home/z/my-project/release-verify')
PUBKEY_LINE = json.loads(
    Path('/home/z/my-project/download-engine/app/src-tauri/tauri.conf.json').read_text()
)['plugins']['updater']['pubkey']

def vfy(pk, msg, sig):
    try:
        pk.verify(sig, msg)
        return True
    except InvalidSignature:
        return False

sig_text = base64.b64decode((RELEASE / 'Velox-Setup-x64-unified.exe.sig').read_bytes()).decode('utf-8')
lines = [l for l in sig_text.split('\n') if l.strip()]
main_blob = base64.b64decode(lines[1])
tc_full = lines[2]                       # 'trusted comment: timestamp:...\tfile:...\tversion:0.6.0'
tc = tc_full[len('trusted comment: '):]  # comment text only
global_sig = base64.b64decode(lines[3])  # 64-byte bare sig

decoded_pub = base64.b64decode(PUBKEY_LINE).decode('utf-8', 'replace')
key_b64 = [l for l in decoded_pub.split('\n') if l.startswith('RU') or l.startswith('RW')][0]
pkey_raw = base64.b64decode(key_b64)
pk = Ed25519PublicKey.from_public_bytes(pkey_raw[10:42])

installer = (RELEASE / 'Velox-Setup-x64-unified.exe').read_bytes()
msig = main_blob[10:74]
print(f'algo prefix: {main_blob[:2]!r}, global sig {len(global_sig)} bytes, keynum ok: {main_blob[2:10] == pkey_raw[2:10]}')

print('\n== main signature message candidates ==')
candidates = {
    'raw file bytes': installer,
    'blake2b-512(file)': hashlib.blake2b(installer, digest_size=64).digest(),
    'sha512(file)': hashlib.sha512(installer).digest(),
}
for name, msg in candidates.items():
    print(f'  {"VALID" if vfy(pk, msg, msig) else "invalid":7}  {name}')

print('\n== global signature message candidates (sig over comment) ==')
gcands = {
    'blob74+tc': main_blob + tc.encode(),
    'blob74+prefix+tc': main_blob + ('trusted comment: ' + tc).encode(),
    'blob74+tc+\\n': main_blob + (tc + '\n').encode(),
    'blob74+prefix+tc+\\n': main_blob + ('trusted comment: ' + tc + '\n').encode(),
    'blob72+tc': main_blob[2:] + tc.encode(),
    'blob64+tc': main_blob[10:] + tc.encode(),
    'tc only': tc.encode(),
    'prefix+tc only': ('trusted comment: ' + tc).encode(),
}
for name, msg in gcands.items():
    print(f'  {"VALID" if vfy(pk, msg, global_sig) else "invalid":7}  {name}')
