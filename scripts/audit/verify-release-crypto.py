#!/usr/bin/env python3
"""End-to-end updater round-trip verification for the published v0.6.0 release.

Replicates tauri-plugin-updater's minisign verification (minisign-verify crate)
against the pubkey embedded in the shipped app (tauri.conf.json), plus the
version cross-check the updater performs (trusted comment version binding).

minisign format (jedisct1/minisign):
  untrusted comment: <text>
  <base64: algo "Ed"(2) || keynum(8) || sig(64)>          = main sig over file bytes
  trusted comment: <text>
  <base64: algo "Ed"(2) || keynum(8) || sig(64)>          = global sig over (main sig blob || trusted comment)
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

def parse_minisign_pubkey(b64_line: str) -> bytes:
    raw = base64.b64decode(b64_line)
    assert raw[:2].upper() == b'ED', f'unexpected key algo {raw[:2]!r}'
    keynum = raw[2:10]
    pk = raw[10:42]
    assert len(pk) == 32
    return keynum, pk

def parse_sig_file(text: str):
    lines = [l for l in text.split('\n') if l.strip()]
    assert lines[0].startswith('untrusted comment:'), lines[0][:40]
    main_blob = base64.b64decode(lines[1])
    assert main_blob[:2].upper() == b'ED', f'unexpected sig algo {main_blob[:2]!r}'
    assert lines[2].startswith('trusted comment:')
    trusted_comment = lines[2][len('trusted comment:'):].lstrip(' ').rstrip('\r')
    global_blob = base64.b64decode(lines[3])
    # rust-minisign-verify stores the global sig as a bare 64-byte Ed25519 sig
    # (no algo/keynum header); older minisign files use the 74-byte "ED" blob
    assert len(global_blob) in (64, 74), f'unexpected global sig length {len(global_blob)}'
    return main_blob, trusted_comment, global_blob

def verify(pk: Ed25519PublicKey, message: bytes, sig64: bytes) -> bool:
    try:
        pk.verify(sig64, message)
        return True
    except InvalidSignature:
        return False

def main() -> int:
    # the pubkey embeds its own untrusted comment line — decode it
    decoded_pub = base64.b64decode(PUBKEY_LINE).decode('utf-8', 'replace')
    key_lines = [l for l in decoded_pub.split('\n') if l.startswith('RU') or l.startswith('RW')]
    key_b64 = key_lines[0]
    keynum, pk_bytes = parse_minisign_pubkey(key_b64)
    pk = Ed25519PublicKey.from_public_bytes(pk_bytes)
    print(f'pubkey keynum: {keynum.hex()}  (from shipped tauri.conf.json)')

    installer = (RELEASE / 'Velox-Setup-x64-unified.exe').read_bytes()
    print(f'installer: {len(installer):,} bytes')

    sig_text = base64.b64decode((RELEASE / 'Velox-Setup-x64-unified.exe.sig').read_bytes()).decode('utf-8')
    main_blob, trusted_comment, global_blob = parse_sig_file(sig_text)
    print(f'trusted comment: {trusted_comment}')

    digest = hashlib.blake2b(installer, digest_size=64).digest()
    ok_main = verify(pk, digest, main_blob[10:74])
    print(f'main signature over BLAKE2b-512(installer) [rust-minisign ED hashed variant]: {"VALID" if ok_main else "INVALID"}')

    # global sig: bare 64-byte Ed25519 over (main_sig_64 || trusted comment text)
    gsig = global_blob if len(global_blob) == 64 else global_blob[10:74]
    ok_global = verify(pk, main_blob[10:] + trusted_comment.encode(), gsig)
    variant = '(main_sig64 + comment, no prefix, no newline)'
    print(f'global signature over {variant}: {"VALID" if ok_global else "INVALID"}')

    # the updater's version cross-check: trusted comment must bind this exact file and version
    m = re.search(r'version:([\d.]+)', trusted_comment)
    f = re.search(r'file:(\S+)', trusted_comment)
    version_ok = bool(m and m.group(1) == '0.6.0')
    file_ok = bool(f and f.group(1) == 'Velox-Setup-x64-unified.exe')
    print(f'version binding 0.6.0: {"OK" if version_ok else "MISMATCH"}')
    print(f'file binding: {"OK" if file_ok else "MISMATCH"}')

    # keynum consistency between sig and key
    print(f'sig keynum matches pubkey: {main_blob[2:10] == keynum}')

    verdict = ok_main and ok_global and version_ok and file_ok and main_blob[2:10] == keynum
    print('VERDICT:', 'PASS — the published installer verifies cryptographically against the shipped pubkey' if verdict else 'FAIL')
    return 0 if verdict else 1

if __name__ == '__main__':
    sys.exit(main())
