#!/usr/bin/env python3
"""Measure a command: wall time, peak RSS (ru_maxrss), CPU%. GNU time is not
available in this environment, so getrusage(RUSAGE_CHILDREN) does the job.
On a nonzero exit code, the tool's stderr tail is printed for diagnosis."""
import resource
import subprocess
import sys
import time

cmd = sys.argv[1:]
t0 = time.monotonic()
r = subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
wall = time.monotonic() - t0
ru = resource.getrusage(resource.RUSAGE_CHILDREN)
cpu = (ru.ru_utime + ru.ru_stime) / wall * 100 if wall > 0 else 0.0
tail = ""
if r.returncode != 0 and r.stderr:
    err = r.stderr.decode(errors="replace").strip().splitlines()
    tail = " | stderr: " + (err[-1] if err else "")
print(f"wall={wall:.2f}s rss={ru.ru_maxrss}kB cpu={cpu:.0f}% rc={r.returncode}{tail}")
