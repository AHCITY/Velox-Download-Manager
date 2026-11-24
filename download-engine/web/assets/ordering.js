// Velox list ordering — PURE, node-testable comparator for the Downloads list.
// Active downloads (transferring, retrying, resolving, merging) always sort
// above everything else, ordered by their most recent activity; the user's
// chosen sort governs the non-active rows. Nothing in here touches the DOM —
// store.visibleRows() applies it and the Downloads screen re-sequences only
// when the computed order actually changed. No imports: store.js passes the
// active-state set in, keeping the dependency graph acyclic.

/**
 * @param {Array} rows engine StatusView rows (any order)
 * @param {{activeStates: string[], lastActive: Map<number,number>, sortKey: string, sortDir: string}} ctx
 * @returns {Array} ordered copy — active group first (newest activity first),
 *   then the rest under the user's sort key.
 */
export function orderRows(rows, ctx) {
  const { activeStates, lastActive, sortKey, sortDir } = ctx;
  const isActive = (r) => activeStates.includes(r.state);
  const mul = sortDir === 'asc' ? 1 : -1;
  const byUserSort = (a, b) => {
    let va = a[sortKey], vb = b[sortKey];
    if (sortKey === 'final_name') { va = (va || '').toLowerCase(); vb = (vb || '').toLowerCase(); }
    if (va == null) va = sortKey === 'total_size' ? -1 : '';
    if (vb == null) vb = sortKey === 'total_size' ? -1 : '';
    if (va < vb) return -1 * mul;
    if (va > vb) return 1 * mul;
    return b.id - a.id;
  };
  return [...rows].sort((a, b) => {
    const aa = isActive(a) ? 1 : 0;
    const ab = isActive(b) ? 1 : 0;
    if (aa !== ab) return ab - aa; // active work first
    if (aa === 1) {
      const ta = lastActive.get(a.id) || 0;
      const tb = lastActive.get(b.id) || 0;
      if (ta !== tb) return tb - ta; // most recently (re)started activity on top
      return b.id - a.id; // stable fallback for rows never seen active
    }
    return byUserSort(a, b);
  });
}

/** True when a row ENTERED an active state (used to stamp activity time). */
export function becameActive(prevState, nextState, activeStates) {
  return activeStates.includes(nextState) && !activeStates.includes(prevState);
}
