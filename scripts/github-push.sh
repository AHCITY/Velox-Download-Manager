#!/bin/bash
# Push the Velox project to GitHub using a token supplied via ENV ONLY.
# The token is NEVER written to any file, config, or the remote URL.
#
# Usage (run in the sandbox shell):
#   GITHUB_TOKEN=ghp_xxx... scripts/github-push.sh
#
# Token requirements: fine-grained PAT on account AHCITY with
#   - Repository access: the target repo (create it first if it doesn't exist)
#   - Contents: Read and Write
# (A classic PAT with the "repo" scope also works.)
set -eu
# env var first; fallback: gitignored local token file (0600) for unattended cycles
[ -n "${GITHUB_TOKEN:-}" ] && [ -r /home/z/my-project/.secrets/github-token ] && \
  GITHUB_TOKEN="$(cat /home/z/my-project/.secrets/github-token)"
: "${GITHUB_TOKEN:?Set GITHUB_TOKEN (env) or provide .secrets/github-token (0600, gitignored)}"
cd /home/z/my-project

URL=$(git remote get-url origin)
case "$URL" in
  https://github.com/*) ;;
  *) echo "unexpected remote: $URL"; exit 1 ;;
esac
AUTH_URL="${URL/https:\/\//https:\/\/AHCITY:${GITHUB_TOKEN}@}"

echo "== pushing $(git rev-parse --short HEAD) ($(git log --oneline -1 | cut -c9-)) =="
# push main first; tolerate an empty upstream
if git push "$AUTH_URL" main 2>&1 | sed "s/${GITHUB_TOKEN}//g"; then
  echo "== main pushed =="
else
  echo "== push FAILED (see message above). If the error is 'Repository not found', create the repo on github.com first. ==" >&2
  exit 1
fi
# tags exist after releases; tolerate none
git push "$AUTH_URL" --tags 2>&1 | sed "s/${GITHUB_TOKEN}//g" || true

# verify the remote really has the commit (never claim success without proof)
REMOTE_HEAD=$(git ls-remote "$AUTH_URL" refs/heads/main | cut -f1)
LOCAL_HEAD=$(git rev-parse HEAD)
if [ "$REMOTE_HEAD" = "$LOCAL_HEAD" ]; then
  echo "== VERIFIED: remote main == local $LOCAL_HEAD =="
else
  echo "== WARNING: remote head $REMOTE_HEAD != local $LOCAL_HEAD — investigate ==" >&2
  exit 1
fi
