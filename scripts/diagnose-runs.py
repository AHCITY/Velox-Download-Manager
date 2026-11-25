#!/usr/bin/env python3
"""Diagnose failed v0.7.0 release runs (#69/#71/#73/#75) via GitHub API."""
import json
import os
import re
import sys
import urllib.request

TOKEN = open('/home/z/my-project/.secrets/github-token').read().strip()
REPO = 'AHCITY/Velox-Download-Manager'
RUNS = {
    '45622266476': '#69 (766712e)',
    '45625186177': '#71 (913c906)',
    '45628692779': '#73 (b7d9e78)',
    '45631597093': '#75 (7373ea4)',
}


def api(path):
    req = urllib.request.Request(
        f'https://api.github.com{path}',
        headers={'Authorization': f'Bearer {TOKEN}',
                 'Accept': 'application/vnd.github+json'})
    return urllib.request.urlopen(req)


def job_logs(job_id):
    try:
        with api(f'/repos/{REPO}/actions/jobs/{job_id}/logs') as r:
            return r.read().decode('utf-8', 'replace')
    except urllib.error.HTTPError as e:
        return f'<logs unavailable: HTTP {e.code}>'


for run_id, label in RUNS.items():
    print('=' * 78)
    print(f'RUN {label} id={run_id}')
    with api(f'/repos/{REPO}/actions/runs/{run_id}/jobs?per_page=20') as r:
        jobs = json.load(r)['jobs']
    for j in jobs:
        print(f"  job: {j['name']:22s} status={j['status']:10s} conclusion={j['conclusion']}")
        if j['conclusion'] == 'failure':
            for s in j.get('steps', []):
                if s['conclusion'] not in ('success', 'skipped', None):
                    print(f"    FAILED STEP: {s['name']} -> {s['conclusion']}")
            log = job_logs(j['id'])
            lines = log.splitlines()
            # error-relevant lines: ##[error], errors, panics, last 25 lines
            interesting = [l for l in lines
                           if '##[error]' in l or 'ERROR:' in l or 'error:' in l.lower()[:120]]
            print(f'    log lines: {len(lines)}, error-ish: {len(interesting)}')
            for l in interesting[:8]:
                clean = re.sub(r'\x1b\[[0-9;]*m', '', l)
                print('    ERR|', clean[:220])
            print('    --- tail ---')
            for l in lines[-18:]:
                clean = re.sub(r'\x1b\[[0-9;]*m', '', l)
                print('    |', clean[:220])
