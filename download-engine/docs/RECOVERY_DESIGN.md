# RECOVERY DESIGN — Persistent state, crash safety, and resume validation

This is the reliability heart of the engine. Design authority: sqlite.org (wal.html, pragma.html,
atomiccommit.html), man7.org fsync(2)/rename(2)/write(2)/ftruncate(2), LWN 457667 — all verified 2026-11-19.

## 1. Persistent state model (SQLite, engine-state)

```sql
downloads(id PK, url, resolved_url, dest_path, temp_path, final_name,
          total_size NULLABLE, etag, etag_is_weak, last_modified,
          mime, content_encoding, accept_ranges,          -- yes|no|unknown
          mode,                                            -- segmented|single|stream
          state,                                           -- created|inspecting|downloading|paused|
                                                           -- completed|failed|cancelled|verifying
          received, max_connections, config_json,
          error_code, error_message, sha256, blake3, verified,
          created_at, updated_at, completed_at)
segments(download_id FK, idx PK, start_off, end_off NULLABLE,  -- NULL = unbounded stream
         pos,        -- durable checkpoint: bytes of THIS segment known written
         completed)
errors(id PK, download_id FK, ts, code, message, class, segment_idx)  -- bounded ring (last 50)
```

Single writer at a time (WAL); all multi-row updates in `BEGIN IMMEDIATE` transactions; `busy_timeout=5000`.
`downloads.received` is always the sum of segment positions, updated in the same transaction — no drift.
DB location: data-dir `state.db` (ARCHITECTURE §2; `DXDL_HOME` override).

## 2. Segment lifecycle

```
planned → active(pos advancing) → checkpointed(pos durable) ─┐
                    │                                        ├─→ complete(sync_data → DB commit)
                    └─→ reclaimed(tail stolen by scheduler) ─┘
any state ─→ reset(pos=0)   [version change / file reconciliation]
```

A segment's `pos` is a **durable lower bound** on correctly-written bytes (see §4). `completed` is set only
after that segment's data reached `sync_data()` **and** the DB commit returned.

**Cross-process exclusion (session 2):** `engine-storage::DownloadLock` takes a `<part>.lock` file recording
the owning pid before any task opens a part file; a lock whose pid no longer exists (SIGKILL, power loss)
is stolen on the next resume. Two engine processes can therefore never interleave writes into one part
file — a corruption vector found by the P5 audit and closed. Boundary normalization additionally
completes any segment found fully written but unmarked (pause/kill at the exact boundary), so a resume
can never synthesize an invalid `start > end` range request.

## 3. Checkpoint policy (crash windows)

Worker loop: read ≤64 KiB → positional write → `advance(n)` (in-memory). Every `checkpoint_interval`
(default 4 MiB) or 3 s, whichever first, the worker's dirty position is flushed:

```
File::sync_data()            -- data now durable
BEGIN IMMEDIATE; UPDATE segments SET pos=?; UPDATE downloads SET received=?; COMMIT   (synchronous=FULL)
```

Crash windows and outcomes:
- crash between write and sync_data: DB pos is older → resume re-downloads the tail. Safe.
- crash between sync_data and commit: same — pos older than disk. Safe (extra bytes get overwritten).
- crash after commit: pos ≤ durable bytes **because fsync precedes commit** — the load-bearing ordering.
  The reverse order would let the DB claim bytes that power loss destroyed; it is forbidden.

SQLite durability basis: `journal_mode=WAL` + `synchronous=FULL` makes commits durable against power loss
(pragma.html matrix); NORMAL would only survive *application* crashes — insufficient here because the DB row
is the resume manifest. WAL is local-disk-only by design (wal.html).

## 4. Completion / finalization order (strict, non-negotiable)

```
1. all segments complete (each already sync_data'd at completion)
2. optional whole-file hash (sha256/blake3) computed on the part file
3. File::sync_all()
4. rename(temp → final)                       -- atomic same-volume replace (rename(2))
5. fsync(destination directory)               -- fsync(2): "does not necessarily ensure that the entry in the
                                              -- directory ... has also reached disk"; Unix; no-op on Windows MVP
6. BEGIN IMMEDIATE; UPDATE downloads SET state='completed', final_path, hashes; COMMIT
```

Invariant: **state=completed in the DB ⇒ final file exists and is durable.** Crash anywhere before step 6
leaves state<completed → recovery path (§5). A completed row can therefore never reference missing/corrupt
data, except through external interference (user deleting files) — detected by `verify`.

## 5. Resume algorithm

### 5.1 Resource identity
1. Re-inspect with the stored validator: `GET Range: bytes=0-0` + `If-Range: <strong-etag>` (or HTTP-date form
   only when it is a strong validator per RFC 9110 §8.8.2.2/§13.1.5 — weak ETags are never used in If-Range).
2. `206` → unchanged: resume. `200` → the server either ignored Range or the resource changed: fall through to
   5.4 (conservative restart). `416` → adopt new length from `Content-Range: */N` and re-evaluate.
3. If no validator existed originally: compare Last-Modified/length when present; if the server never offered
   any validator, resume is allowed but flagged `resource_identity = Unverified` (nothing better is knowable;
   documented rather than faked — "conditionally resumable").
4. Length mismatch vs manifest → treat as changed resource (restart) unless 416/N says otherwise.

### 5.2 Local file reconciliation (manifest vs disk)
Let `max_written = max(start_off + pos)` over non-completed segments; `F` = part-file length.
- `F < max_written` → durable bytes are missing (e.g., rollback beyond checkpoints, tampering):
  any *completed* segment with `end_off > F` ⇒ the file is untrustworthy ⇒ **restart whole download**;
  otherwise clamp each in-progress segment `pos = min(pos, F.saturating_sub(start_off))` and re-download the
  remainder (positional writes overwrite any garbage).
- `F > expected` regions → data beyond checkpoints is un-checkpointed residue; it is simply overwritten later.
- `F > total` → truncate to `total`.
- Missing part file + manifest says in-progress → restart (nothing to trust).

### 5.3 Policy: `resume_policy = RequireValidatorIfAvailable` (default)
If the server originally provided ETag/Last-Modified, resume demands they still match (via If-Range/304);
only validator-less servers get `Unverified` resume. This is configurable for users who prefer IDM-style
size-only resume, at their own documented risk.

### 5.4 Validator change mid-download
If any range request receives `200` (If-Range failed ⇒ new version) or a Content-Range start that mismatches
the requested start, the task aborts all workers, resets segments/pos, truncates the part file, records the new
validator, and restarts planning. We never splice old-version bytes with new-version bytes (RFC 9110 §15.3.7.3).

## 6. Crash matrix

| Crash point | DB says | Disk has | Recovery |
|---|---|---|---|
| mid-write, before sync | pos=N | ≤N valid | resume from N (overwrite tail) |
| after sync, before commit | pos=N-δ | N valid | resume from N-δ (re-download δ) |
| after segment commit | seg complete | seg durable (order §4) | skip segment |
| after rename, before DB commit | state=downloading, temp_path missing (renamed away) | final file exists | **implemented behavior:** the orphan is never adopted; the engine re-downloads into a collision-safe name and never exposes unverified bytes as complete (verified by `rename_orphan_final_file_is_never_adopted_without_evidence`). An adopt-if-verified path (size + available hash) is a planned optimization |
| after DB completed commit | completed | durable | nothing to do |
| process kill mid-flight (no handlers) | last checkpoint | last checkpoint +residue | resume (§5.2) — **verified out-of-process with SIGKILL**: mid-download kill, kill during resume, kill before first checkpoint; manifest-vs-file invariant (positions ≤ file length) asserted after every kill |
| power loss during checkpoint | WAL-rolled-back | older | resume (§5.2). *Scope note:* this claim covers process kill + SQLite WAL/FULL commit durability; the engine does not claim hardware-level power-loss safety beyond what fsync+SQLite provide on the target filesystem |

The "rename orphan" case is handled by an explicit reconciliation step at startup: if `temp_path` is absent but
`final_name` exists with matching size/validators, verify-then-adopt or restart; never silently mark complete
without evidence (hash when available, else size+validators).

## 7. Platform notes

- Directory fsync is required on POSIX (fsync(2)) and is skipped on Windows in MVP (NTFS metadata journaling
  caveat: file *data* durability still guaranteed by sync_all before rename; rename-entry durability on NTFS
  without dir-handle flush is **unverified** — documented as an MVP limitation, platform adapter planned).
- `set_len` preallocation creates a sparse file (ftruncate(2)); unallocated regions read as zeros — harmless
  because every byte range is written before completion claims it.
- Android (future): process death is the common crash — handled by the same checkpoint/resume path; storage
  adapter will handle SAF volumes separately (ROADMAP P7).

## 8. Non-guarantees (stated plainly)

- Un-checkpointed tail bytes (≤ checkpoint_interval) are re-downloaded after a crash — by design.
- Without any validator and without a checksum, "the same file" is unverifiable; we say so in status.
- A completed download's integrity claim is only as strong as its verification: checksummed ⇒ strong;
  validator-consistent ⇒ conditional; nothing ⇒ size-only.
