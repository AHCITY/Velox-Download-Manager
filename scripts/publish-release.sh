#!/bin/bash
# Publish v0.5.0 — the one command that ships the new setup files.
#
# Pushes main + the v0.5.0 tag. GitHub CI (windows.yml) then builds the signed
# NSIS installer, generates latest.json (updater manifest), zips the browser
# extension and PUBLISHES the release automatically (release job on v* tags).
#
# Token: GITHUB_TOKEN env or .secrets/github-token (0600, gitignored) — the
# sandbox recycle wiped .secrets/, so the user runs this with their own token.
#
# Usage: GITHUB_TOKEN=ghp_... scripts/publish-release.sh
set -euo pipefail
cd /home/z/my-project

TOKEN="${GITHUB_TOKEN:-}"
if [ -z "$TOKEN" ] && [ -r .secrets/github-token ]; then
  TOKEN="$(cat .secrets/github-token)"
fi
: "${TOKEN:?set GITHUB_TOKEN (env) or provide .secrets/github-token (0600)}"

URL=$(git remote get-url origin)
case "$URL" in https://github.com/*) ;; *) echo "unexpected remote: $URL"; exit 1 ;; esac
AUTH_URL="${URL/https:\/\//https:\/\/AHCITY:${TOKEN}@}"

TAG="v0.5.0"
git rev-parse -q --verify "refs/tags/$TAG" >/dev/null || git tag -a "$TAG" -m "Velox 0.5.0 — signed CI releases, updater pipeline repaired"

echo "== pushing main (HEAD $(git rev-parse --short HEAD)) =="
git push "$AUTH_URL" main 2>&1 | sed "s/${TOKEN}//g"
echo "== pushing $TAG =="
git push "$AUTH_URL" "$TAG" 2>&1 | sed "s/${TOKEN}//g"

echo "== VERIFIED: remote main =="
git ls-remote "$AUTH_URL" refs/heads/main "refs/tags/$TAG" | sed "s/${TOKEN}//g"

cat <<'NEXT'

Next (automatic): windows.yml runs on the tag push:
  windows gate -> desktop job builds + signs Velox-Setup-x64.exe
  -> release job publishes the GitHub release with:
       Velox-Setup-x64.exe (+ .sig), latest.json, Velox-Extension.zip, SHA256SUMS.txt
Watch: https://github.com/AHCITY/Velox-Download-Manager/actions
Preview: https://github.com/AHCITY/Velox-Download-Manager/releases/tag/v0.5.0
NOTE: existing 0.4.0 installs pin the OLD updater key (lost in a recycle), so
they cannot auto-update to 0.5.0 — install Velox-Setup-x64.exe manually once.
NEXT
