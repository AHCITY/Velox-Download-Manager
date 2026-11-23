#!/usr/bin/env python3
"""Read-only GitHub status report for the Velox repo (repo connectivity check).

Token comes from the GH_TOKEN / GITHUB_TOKEN environment ONLY (never a file,
never printed). Every request below is a GET — nothing is modified on GitHub.

Usage:  GH_TOKEN=... python3 scripts/gh-status.py
"""
import json
import os
import pathlib
import subprocess
import sys
import urllib.error
import urllib.request

REPO = "AHCITY/Velox-Download-Manager"
API = f"https://api.github.com/repos/{REPO}"
PROJECT = pathlib.Path(__file__).resolve().parent.parent


def get(url, token):
    req = urllib.request.Request(url, headers={
        "Authorization": f"Bearer {token}",
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "velox-status-check",
    })
    try:
        with urllib.request.urlopen(req, timeout=40) as r:
            return r.status, json.loads(r.read() or b"null"), dict(r.headers)
    except urllib.error.HTTPError as e:
        try:
            body = json.loads(e.read() or b"null")
        except Exception:
            body = None
        return e.code, body, dict(e.headers or {})


def git(*args):
    return subprocess.run(["git", "-C", str(PROJECT), *args], capture_output=True, text=True).stdout.strip()


def main():
    token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    if not token:
        print("GH_TOKEN not set in the environment")
        return 2

    code, me, hdr = get("https://api.github.com/user", token)
    scopes = hdr.get("X-OAuth-Scopes", "(fine-grained PAT: scopes not listed)")
    print(f"[auth]    GET /user -> HTTP {code}; login={me.get('login') if isinstance(me, dict) else me}; "
          f"scopes={scopes}")
    if code != 200:
        return 3

    code, repo, _ = get(API, token)
    if code != 200:
        print(f"[repo]    GET {API} -> HTTP {code}: {repo.get('message') if isinstance(repo, dict) else repo}")
        return 4
    perm = repo.get("permissions", {})
    print(f"[repo]    {repo['full_name']} private={repo['private']} visibility={repo.get('visibility')} "
          f"default={repo['default_branch']} license={(repo.get('license') or {}).get('spdx_id')} "
          f"size={repo['size']/1024:.1f} MB (git storage)")
    print(f"[repo]    pushed_at={repo['pushed_at']} updated_at={repo['updated_at']} "
          f"open_issues+PRs={repo['open_issues_count']} archived={repo['archived']}")
    print(f"[access]  admin={perm.get('admin')} maintain={perm.get('maintain')} push={perm.get('push')} "
          f"pull={perm.get('pull')}")

    local_head = git("rev-parse", "HEAD")
    local_branch = git("rev-parse", "--abbrev-ref", "HEAD")
    code, branch, _ = get(f"{API}/branches/main", token)
    remote_head = branch.get("commit", {}).get("sha") if isinstance(branch, dict) else None
    print(f"[main]    local {local_branch}={local_head[:12]} remote main={str(remote_head)[:12]} "
          f"in_sync={local_head == remote_head} protected={branch.get('protected') if isinstance(branch, dict) else '?'}")

    code, branches, _ = get(f"{API}/branches?per_page=100", token)
    print(f"[branches] {[b['name'] for b in branches] if isinstance(branches, list) else branches}")

    code, refs, _ = get(f"{API}/git/matching-refs/tags", token)
    tags = []
    for r in refs or []:
        sha = r["object"]["sha"]
        if r["object"]["type"] == "tag":  # annotated tag -> resolve to the commit
            _, obj, _ = get(r["object"]["url"], token)
            sha = (obj or {}).get("object", {}).get("sha", sha)
        tags.append((r["ref"].replace("refs/tags/", ""), sha))
    local_tags = dict(
        line.split()[::-1] for line in git("for-each-ref", "--format=%(objectname) %(refname:short)",
                                           "refs/tags").splitlines())
    print("[tags]    " + ", ".join(f"{t}@{s[:7]}" for t, s in tags))
    print(f"[tags]    local==remote: {sorted(local_tags.items()) == sorted(tags)}")

    code, releases, _ = get(f"{API}/releases?per_page=20", token)
    for rel in releases or []:
        print(f"[release] tag={rel['tag_name']} name={rel['name']!r} draft={rel['draft']} prerelease={rel['prerelease']} "
              f"published={rel['published_at']} target={str(rel['target_commitish'])[:7]}")
        for a in rel.get("assets", []):
            print(f"    asset  {a['name']:42s} {a['size']/1048576:8.1f} MB  downloads={a['download_count']:5d}  "
                  f"updated={a['updated_at']}  state={a['state']}")

    code, runs, _ = get(f"{API}/actions/runs?branch=main&per_page=10", token)
    for r in runs.get("workflow_runs", []) if isinstance(runs, dict) else []:
        print(f"[actions] {r['name']:8s} run#{r['run_number']:<4d} sha={r['head_sha'][:7]} event={r['event']:8s} "
              f"status={r['status']:12s} conclusion={r['conclusion']} created={r['created_at']}")
    if isinstance(runs, dict) and runs.get("total_count") is not None:
        print(f"[actions] total runs on main: {runs['total_count']}")

    code, pulls, _ = get(f"{API}/pulls?state=open&per_page=50", token)
    code2, issues, _ = get(f"{API}/issues?state=open&per_page=50", token)
    n_pr = len(pulls) if isinstance(pulls, list) else pulls
    n_iss = len([i for i in issues if "pull_request" not in i]) if isinstance(issues, list) else issues
    print(f"[open]    pull_requests={n_pr} issues={n_iss}")

    code, rate, _ = get("https://api.github.com/rate_limit", token)
    core = (rate or {}).get("resources", {}).get("core", {})
    print(f"[quota]   core remaining={core.get('remaining')}/{core.get('limit')}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
