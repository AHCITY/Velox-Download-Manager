// Version comparison for the Velox updater (Phase 3 repair, RC8/RC9).
// Pure functions only — no DOM, no Tauri — so they are unit-testable with node.
//
// Strict semver: a release is offered ONLY when it is strictly NEWER than the
// running app. Unparseable versions never announce (honest default).

/** Parse "v1.2.3", "1.2.3-beta" etc. → [major, minor, patch] or null. */
export function parseVersion(v) {
  const m = String(v ?? '')
    .trim()
    .replace(/^v/i, '')
    .match(/^(\d+)\.(\d+)\.(\d+)/);
  if (!m) return null;
  return [parseInt(m[1], 10), parseInt(m[2], 10), parseInt(m[3], 10)];
}

/** true when `remote` is strictly newer than `current`. */
export function isNewerVersion(remote, current) {
  const r = parseVersion(remote);
  const c = parseVersion(current);
  if (!r || !c) return false;
  for (let i = 0; i < 3; i++) {
    if (r[i] !== c[i]) return r[i] > c[i];
  }
  return false;
}
