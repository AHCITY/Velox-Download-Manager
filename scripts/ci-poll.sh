#!/bin/bash
# Poll recent GitHub Actions runs for AHCITY/Velox-Download-Manager.
# Usage: bash scripts/ci-poll.sh
set -euo pipefail
TOKEN=$(cat /home/z/my-project/.secrets/github-token)
REPO=AHCITY/Velox-Download-Manager
curl -s -H "Authorization: Bearer $TOKEN" -H "Accept: application/vnd.github+json" \
  "https://api.github.com/repos/$REPO/actions/runs?per_page=8" \
| python3 -c '
import json, sys
d = json.load(sys.stdin)
for r in d.get("workflow_runs", []):
    rid = r["id"]; st = r["status"]; cc = r.get("conclusion")
    hb = r["head_branch"]; ev = r["event"]; ca = r["created_at"]; url = r["html_url"]
    print(f"{rid}  {st:<12} {str(cc):<10} {hb:<12} {ev:<8} {ca}  {url}")
'
