#!/bin/bash
# Repeatable dependency licence report for Velox.
# Scans the cargo registry source of every crate in Cargo.lock and prints the
# licence field; flags copyleft licences for review. No new dependencies.
# Output: docs/dependency-licence-report.txt
set -u
cd /home/z/my-project/download-engine || exit 1
REG=/home/z/my-project/.cargo/registry/src
OUT=docs/dependency-licence-report.txt
LOCKED=$(python3 - <<'EOF'
import re
s = open('Cargo.lock').read()
pkgs = re.findall(r'\[\[package\]\]\nname = "([^"]+)"\nversion = "([^"]+)"', s)
print('\n'.join(f'{n} {v}' for n, v in pkgs))
EOF
)
{
echo "# Velox dependency licence report — generated $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "# source: Cargo.lock ($(echo "$LOCKED" | wc -l) crates) × cargo registry licence fields"
echo "# FLAG lines require review before any redistribution."
echo "# MISSING-REGISTRY = archive downloaded but not yet unpacked (licence read next run)."
echo
echo "$LOCKED" | while read -r name ver; do
  f=$(find "$REG" -maxdepth 2 -type d -name "${name}-${ver}" 2>/dev/null | head -1)
  if [ -z "$f" ]; then
    echo "MISSING-REGISTRY $name $ver"
    continue
  fi
  lic=$(grep -m1 -E '^\s*license\s*=' "$f/Cargo.toml" 2>/dev/null | sed 's/.*=\s*"\(.*\)"/\1/')
  [ -z "$lic" ] && lic=$(grep -m1 -E '^\s*license-file\s*=' "$f/Cargo.toml" 2>/dev/null | sed 's/.*=\s*"\(.*\)"/file:\1/')
  [ -z "$lic" ] && lic="UNSPECIFIED (check $f/Cargo.toml)"
  if echo "$lic" | grep -qiE 'GPL|AGPL|LGPL'; then
    echo "FLAG $name $ver $lic"
  else
    echo "OK $name $ver $lic"
  fi
done
} > "$OUT"
echo "wrote $OUT: $(grep -c '^OK' "$OUT") OK, $(grep -c '^FLAG' "$OUT") flagged, $(grep -c '^MISSING' "$OUT") missing-from-registry"
