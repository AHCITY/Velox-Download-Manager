// Theme system tests: WCAG contrast of every built-in, sanitizer security,
// and CSS↔JS palette sync (app.css [data-theme] blocks must define exactly
// the same values as themes.js — a drift means a theme changes appearance
// depending on whether JS loaded yet).
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const { BUILTIN_THEMES, THEME_VARS, contrastReport, sanitizeTheme, contrastRatio, luminance, isSafeColor }
  = await import('../assets/themes.js');

/* ---------- built-ins ---------- */

test('five built-in themes exist with unique ids', () => {
  const ids = Object.keys(BUILTIN_THEMES);
  assert.deepEqual(ids.sort(), ['blue', 'dark', 'light', 'purple', 'soft-canopy']);
});

test('every built-in defines every theme variable', () => {
  for (const t of Object.values(BUILTIN_THEMES)) {
    for (const v of THEME_VARS) {
      assert.ok(t.colors[v], `${t.id} is missing ${v}`);
    }
  }
});

test('built-ins pass WCAG AA contrast (4.5:1 text, 3:1 UI colors)', () => {
  for (const t of Object.values(BUILTIN_THEMES)) {
    const fails = contrastReport(t);
    assert.deepEqual(fails, [], `${t.id}: ${fails.join('; ')}`);
  }
});

/* ---------- WCAG math ---------- */

test('contrastRatio matches known WCAG examples', () => {
  assert.equal(Math.round(contrastRatio('#000000', '#ffffff') * 100) / 100, 21);
  assert.equal(Math.round(contrastRatio('#ffffff', '#ffffff') * 100) / 100, 1);
  // WCAG own example: #767676 on #ffffff is exactly 4.54:1 (AA pass)
  assert.ok(contrastRatio('#767676', '#ffffff') >= 4.5);
  // rgba blends onto base: 50% black over white == #808080
  const a = contrastRatio('rgba(0,0,0,.5)', '#ffffff');
  const b = contrastRatio('#808080', '#ffffff');
  assert.ok(Math.abs(a - b) < 0.01, `blend mismatch: ${a} vs ${b}`);
});

test('luminance handles hex, rgb(), hsl() and alpha', () => {
  assert.ok(luminance('#ffffff') > 0.99);
  assert.ok(Math.abs(luminance('#ffffff') - luminance('rgb(255,255,255)')) < 1e-9);
  assert.ok(Math.abs(luminance('#ff0000') - luminance('hsl(0,100%,50%)')) < 1e-9);
  assert.equal(luminance('url(javascript:alert(1))'), null);
});

/* ---------- sanitizer: third-party themes are data, never code ---------- */

test('isSafeColor rejects vectors and non-colors', () => {
  for (const bad of [
    'url(https://evil.example/beacon)', 'url(#x)',
    'var(--t1)', 'calc(1px + 2px)', 'expression(alert(1))',
    'javascript:alert(1)', '</style><script>alert(1)</script>',
    'linear-gradient(red, blue)', 'element(x)', 'attr(x)',
    '', '   ', '#12', '#12345', '#1234567', 'notacolor',
  ]) {
    assert.equal(isSafeColor(bad), false, `should reject: ${bad}`);
  }
  for (const good of ['#abc', '#aabbcc', '#aabbccdd', '#ABCDEF',
    'rgb(1,2,3)', 'rgba(1, 2, 3, 0.5)', 'hsl(120, 50%, 50%)', 'hsla(120,50%,50%,.5)', 'transparent']) {
    assert.equal(isSafeColor(good), true, `should accept: ${good}`);
  }
});

test('sanitizeTheme rejects malicious or malformed manifests', () => {
  const base = { id: 'evil', name: 'Evil', type: 'dark' };
  // url() exfiltration
  assert.equal(sanitizeTheme({ ...base, colors: { '--bg0': 'url(https://evil.example/x)' } }).ok, false);
  // unknown variable injection
  assert.equal(sanitizeTheme({ ...base, colors: { '--evil': '#ff0000' } }).ok, false);
  // partial manifest: valid overrides + garbage must fail on the garbage only
  const mixed = sanitizeTheme({ ...base, colors: { '--bg0': '#101010', '--t1': 'url(x)' } });
  assert.equal(mixed.ok, false);
  // id collisions with built-ins
  assert.equal(sanitizeTheme({ ...base, id: 'dark', colors: {} }).ok, false);
  // bad types
  assert.equal(sanitizeTheme(null).ok, false);
  assert.equal(sanitizeTheme({ ...base, type: 'neon' }).ok, false);
  assert.equal(sanitizeTheme({ ...base, colors: 'red' }).ok, false);
});

test('sanitizeTheme inherits missing variables from the base theme', () => {
  const res = sanitizeTheme({
    id: 'rose', name: 'Rose', type: 'dark', base: 'dark',
    colors: { '--accent': '#ff7ac6', '--accent-h': '#ff9ad6', '--accent-bg': 'rgba(255,122,198,.14)' },
  });
  assert.equal(res.ok, true, JSON.stringify(res.errors));
  assert.equal(res.theme.colors['--bg0'], BUILTIN_THEMES.dark.colors['--bg0']);
  assert.equal(res.theme.colors['--accent'], '#ff7ac6');
});

test('sanitizeTheme rejects a theme that fails WCAG contrast', () => {
  const res = sanitizeTheme({
    id: 'invisible', name: 'Invisible', type: 'light', base: 'light',
    colors: { '--t1': '#ffffff', '--bg0': '#ffffff', '--bg1': '#ffffff', '--bg2': '#ffffff' }, // white on white
  });
  assert.equal(res.ok, false);
  assert.ok(res.errors.some((e) => e.includes('4.5:1') || e.includes('contrast')), res.errors.join('; '));
});

/* ---------- CSS ↔ JS sync ---------- */

test('app.css [data-theme] blocks match themes.js palettes exactly', () => {
  const css = readFileSync(new URL('../assets/app.css', import.meta.url), 'utf8');
  for (const t of Object.values(BUILTIN_THEMES)) {
    const m = css.match(new RegExp(`\\[data-theme="${t.id}"\\]([^}]*)\\}`, 'm'))
      ?? (t.id === 'dark'
        ? css.match(/:root, \[data-theme="dark"\]([^}]*)\}/m)
        : null);
    assert.ok(m, `app.css has no [data-theme="${t.id}"] block`);
    // blocks may pack several declarations per line — parse all of them
    const decls = {};
    for (const dm of m[1].matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) {
      decls[dm[1]] = dm[2].trim();
    }
    const norm = (v) => v.replace(/\s+/g, ''); // css allows spaces in rgba()
    for (const [k, v] of Object.entries(t.colors)) {
      assert.equal(norm(decls[k]), norm(v), `${t.id} ${k}: css="${decls[k]}" js="${v}"`);
    }
  }
});

test('app.css :root defines exactly the theme variables (geometry excluded)', () => {
  const css = readFileSync(new URL('../assets/app.css', import.meta.url), 'utf8');
  const rootBlock = css.match(/:root, \[data-theme="dark"\]([^}]*)\}/m)[1];
  const cssVars = new Set([...rootBlock.matchAll(/^\s*(--[\w-]+)\s*:/gm)].map((m) => m[1]));
  for (const v of THEME_VARS) assert.ok(cssVars.has(v), `:root missing ${v}`);
});

/* ---------- community themes ---------- */

const { COMMUNITY_SOURCES, getTheme, listCommunitySources, fetchCommunityCatalog, savedCommunityEntries,
  isCommunitySaved, saveCommunityTheme, removeSavedCommunityTheme, restoreSavedCommunityThemes }
  = await import('../assets/themes.js');

// node has no localStorage — install a map-backed shim around each storage test
async function withLocalStorage(run) {
  const mem = new Map();
  globalThis.localStorage = {
    getItem: (k) => (mem.has(k) ? mem.get(k) : null),
    setItem: (k, v) => mem.set(k, String(v)),
    removeItem: (k) => mem.delete(k),
  };
  try { await run(); } finally { delete globalThis.localStorage; }
}

// accent-only override on the dark base — inherits every WCAG-checked pair
const oceanManifest = {
  id: 'ocean', name: 'Ocean', type: 'dark', base: 'dark',
  author: 'Nova', version: '1.2', description: 'Deep sea blues for late-night downloading',
  colors: {
    '--accent': '#66c7ff', '--accent-h': '#8ad4ff',
    '--accent-bg': 'rgba(102,199,255,.13)', '--on-accent': '#04121e',
  },
};

test('community sources list ships empty (registry comes later)', () => {
  assert.deepEqual(listCommunitySources(), []);
  assert.deepEqual(COMMUNITY_SOURCES, []);
});

test('fetchCommunityCatalog parses manifests, dedupes ids and skips invalid entries', async () => {
  const manifestA = {
    themes: [
      oceanManifest,
      { ...oceanManifest, id: 'broken', colors: { '--t1': 'url(https://evil.example/x)' } },
      { ...oceanManifest, id: 'ocean' }, // duplicate within one source
    ],
  };
  const manifestB = [oceanManifest]; // bare array form from a second source
  const fetchImpl = async (url) => {
    if (url.includes('a-registry')) return { ok: true, status: 200, json: async () => manifestA };
    if (url.includes('b-registry')) return { ok: true, status: 200, json: async () => manifestB };
    return { ok: false, status: 404, json: async () => ({}) };
  };
  const res = await fetchCommunityCatalog(
    ['https://a-registry.example/themes.json', 'https://b-registry.example/themes.json', 'https://dead.example/x.json'],
    fetchImpl,
  );
  assert.equal(res.items.length, 1, 'deduped by theme id across sources');
  assert.equal(res.items[0].theme.id, 'ocean');
  assert.equal(res.items[0].theme.name, 'Ocean');
  assert.equal(res.skipped, 1, 'invalid entry silently dropped, counted honestly');
  assert.equal(res.sourceErrors.length, 1, 'dead source reported per source');
  assert.match(res.sourceErrors[0].message, /HTTP 404/);
  assert.deepEqual(res.items[0].theme.colors['--bg0'], BUILTIN_THEMES.dark.colors['--bg0'],
    'catalog theme inherits unset variables from its base');
});

test('fetchCommunityCatalog rejects a malformed manifest shape per source', async () => {
  const res = await fetchCommunityCatalog(['https://x.example/r.json'],
    async () => ({ ok: true, status: 200, json: async () => ({ nope: true }) }));
  assert.deepEqual(res.items, []);
  assert.equal(res.sourceErrors.length, 1);
  assert.match(res.sourceErrors[0].message, /manifest must be/);
});

test('save → restore → remove round-trip for downloaded community themes', async () => {
  await withLocalStorage(async () => {
    assert.equal(getTheme('ocean'), null, 'not registered yet in a fresh session');
    assert.equal(await saveCommunityTheme({ id: 'nope', colors: { '--t1': 'url(x)' } }).then((r) => r.ok), false,
      'invalid manifests are refused');
    assert.equal(getTheme('ocean'), null, 'a refused manifest never registers');
    const res = await saveCommunityTheme(oceanManifest, { description: oceanManifest.description });
    assert.equal(res.ok, true, JSON.stringify(res.errors || []));
    assert.equal(isCommunitySaved('ocean'), true);
    assert.equal(getTheme('ocean')?.name, 'Ocean', 'saving registers the theme for this session');
    // re-reads re-validate and inherit from the base
    const entries = savedCommunityEntries();
    assert.equal(entries.length, 1);
    assert.equal(entries[0].theme.id, 'ocean');
    assert.equal(entries[0].theme.colors['--bg0'], BUILTIN_THEMES.dark.colors['--bg0']);
    // boot path: re-registering from storage is idempotent
    assert.equal(restoreSavedCommunityThemes(), 1);
    assert.equal(getTheme('ocean')?.name, 'Ocean');
    // removing unregisters it again
    removeSavedCommunityTheme('ocean');
    assert.equal(isCommunitySaved('ocean'), false);
    assert.equal(getTheme('ocean'), null);
    assert.deepEqual(savedCommunityEntries(), []);
  });
});

test('re-saving a community theme id replaces the stored manifest', async () => {
  await withLocalStorage(async () => {
    await saveCommunityTheme(oceanManifest, { description: 'v1' });
    await saveCommunityTheme({ ...oceanManifest, version: '1.3' }, { description: 'v2' });
    const entries = savedCommunityEntries();
    assert.equal(entries.length, 1, 'no duplicate ids');
    assert.equal(entries[0].theme.version, '1.3');
    assert.equal(entries[0].meta.description, 'v2');
  });
});
