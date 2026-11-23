// Pure decision logic of the frontend queue orchestrator — no DOM, no engine,
// no store imports, so it runs under `node --test` (tests/queue-plan.test.mjs).
//
// A queue may only advance items that are waiting BECAUSE OF THE QUEUE. An
// item the user paused by hand is recorded in its queue's `held` list and
// stays paused until the user resumes it. "paused" used to be treated as
// "waiting", so the next completion event — or simply the next app start —
// silently restarted manually paused members.

// rows that occupy a queue slot (resolving/merging are real active work)
export const QUEUE_RUNNING = ['downloading', 'retrying', 'resolving', 'merging'];
// rows the orchestrator may start
export const QUEUE_WAITING = ['queued', 'paused', 'created'];

/**
 * Queue items to start right now, in order, without duplicates.
 *
 * @param {Array<{active?: boolean, itemIds?: number[], held?: number[], maxConcurrent?: number}>} queues
 * @param {{ get(id: number): any }} rows  id -> { id, state } (the store's row Map)
 * @param {{ blocked?: boolean }} [opts]  `blocked`: start nothing (an update
 *   install is in flight — rows were paused on purpose for the exit)
 * @returns {number[]} item ids
 */
export function planQueueStarts(queues, rows, opts = {}) {
  if (opts.blocked) return [];
  const starts = [];
  const claimed = new Set(); // a row member of two active queues is started once
  for (const q of queues || []) {
    if (!q.active) continue;
    const held = Array.isArray(q.held) ? q.held : [];
    const items = (q.itemIds || []).map((id) => rows.get(id)).filter(Boolean);
    const running = items.filter((r) => QUEUE_RUNNING.includes(r.state)).length;
    let slots = Math.max(0, (q.maxConcurrent || 2) - running);
    for (const r of items) {
      if (slots <= 0) break;
      if (!QUEUE_WAITING.includes(r.state) || held.includes(r.id) || claimed.has(r.id)) continue;
      starts.push(r.id);
      claimed.add(r.id);
      slots--;
    }
  }
  return starts;
}

/**
 * Record (held=true) or clear (held=false) a hand-pause of `id` on EVERY queue
 * that contains the item — an item can be a member of several queues, and each
 * one's orchestrator must respect the hold. Queues saved before this existed
 * have no `held` field and get one on first use.
 *
 * @returns {boolean} true when a queue changed
 */
export function setQueueHold(queues, id, held) {
  let changed = false;
  for (const q of queues || []) {
    if (!(q.itemIds || []).includes(id)) continue;
    const cur = Array.isArray(q.held) ? q.held : [];
    if (held && !cur.includes(id)) {
      q.held = [...cur, id];
      changed = true;
    } else if (!held && cur.includes(id)) {
      q.held = cur.filter((x) => x !== id);
      changed = true;
    }
  }
  return changed;
}
