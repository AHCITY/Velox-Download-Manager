// Velox theme system — built-in themes + user-installable third-party themes.
//
// Architecture (docs/THEMES.md is the user-facing guide):
//   - A theme is DATA, never code: a JSON manifest of CSS custom properties.
//     Third-party themes cannot contain selectors, url(), @import or any
//     executable content by construction — see sanitizeTheme().
//   - The 4 built-ins (dark, light, purple, blue) are ALSO defined in
//     assets/app.css as `[data-theme="…"]` blocks so the first paint is
//     always correct (no JS round-trip, no flash of the wrong theme).
//     assets/theme-boot.js applies the persisted theme attribute before
//     first paint from a localStorage cache. The node test
//     web/tests/themes.test.mjs asserts the CSS blocks and the objects
//     below stay in sync.
//   - Custom (user-installed) themes are applied at runtime by writing the
//     validated color variables to document.documentElement.style — CSP-safe
//     (CSSOM property writes are not subject to style-src).
//   - Every theme — built-in or custom — must pass the WCAG contrast check
//     (contrastReport) before it can be applied; failures are reported with
//     the exact ratio and requirement.

/* ------------------------------------------------------------------ */
/* Built-in themes                                                     */
/* ------------------------------------------------------------------ */

// Variables every theme must define. Keep in sync with app.css `:root`.
export const THEME_VARS = [
  // surfaces
  '--bg0', '--bg1', '--bg2', '--bg3', '--bg4',
  // lines
  '--line', '--line2',
  // text
  '--t1', '--t2', '--t3',
  // accent
  '--accent', '--accent-h', '--accent-bg', '--on-accent',
  // status
  '--ok', '--ok-bg', '--warn', '--warn-bg', '--err', '--err-bg',
  '--purple', '--purple-bg',
  // media/category accents
  '--cat-archive', '--cat-archive-bg',
  // controls
  '--scroll', '--scroll-h', '--spin-track', '--spin-dot',
  // depth
  '--shadow-pop',
];

const dark = {
  id: 'dark', name: 'Dark', type: 'dark', built_in: true,
  colors: {
    '--bg0': '#0b0e13', '--bg1': '#10141b', '--bg2': '#161c25', '--bg3': '#1c232e', '--bg4': '#232c38',
    '--line': '#1e2632', '--line2': '#2a3543',
    '--t1': '#e9eef5', '--t2': '#9aa7b8', '--t3': '#6b7889',
    '--accent': '#4d9fff', '--accent-h': '#6cb0ff', '--accent-bg': 'rgba(77,159,255,.13)', '--on-accent': '#06121f',
    '--ok': '#3ecf8e', '--ok-bg': 'rgba(62,207,142,.12)',
    '--warn': '#e2b53e', '--warn-bg': 'rgba(226,181,62,.12)',
    '--err': '#f4695e', '--err-bg': 'rgba(244,105,94,.12)',
    '--purple': '#b18cff', '--purple-bg': 'rgba(177,140,255,.13)',
    '--cat-archive': '#e2a55e', '--cat-archive-bg': 'rgba(226,165,94,.12)',
    '--scroll': '#2a3543', '--scroll-h': '#35404f',
    '--spin-track': 'rgba(0,0,0,.25)', '--spin-dot': '#06121f',
    '--shadow-pop': '0 8px 28px rgba(0,0,0,.45), 0 2px 8px rgba(0,0,0,.3)',
  },
};

const light = {
  id: 'light', name: 'Light', type: 'light', built_in: true,
  colors: {
    '--bg0': '#f3f5f8', '--bg1': '#ffffff', '--bg2': '#eef1f5', '--bg3': '#e7ebf1', '--bg4': '#dde2ea',
    '--line': '#e2e6ed', '--line2': '#ccd3de',
    '--t1': '#1b2430', '--t2': '#4f5b6a', '--t3': '#7c8797',
    '--accent': '#0b6dd6', '--accent-h': '#095cb4', '--accent-bg': 'rgba(11,109,214,.10)', '--on-accent': '#ffffff',
    '--ok': '#157a4d', '--ok-bg': 'rgba(21,122,77,.10)',
    '--warn': '#8f6400', '--warn-bg': 'rgba(143,100,0,.10)',
    '--err': '#c2392e', '--err-bg': 'rgba(194,57,46,.10)',
    '--purple': '#7443d6', '--purple-bg': 'rgba(116,67,214,.09)',
    '--cat-archive': '#a05f16', '--cat-archive-bg': 'rgba(160,95,22,.10)',
    '--scroll': '#c6cdd8', '--scroll-h': '#b2bac7',
    '--spin-track': 'rgba(255,255,255,.6)', '--spin-dot': '#0b6dd6',
    '--shadow-pop': '0 8px 28px rgba(23,32,46,.14), 0 2px 8px rgba(23,32,46,.10)',
  },
};

const purple = {
  id: 'purple', name: 'Purple', type: 'dark', built_in: true,
  colors: {
    '--bg0': '#0d0916', '--bg1': '#120d1f', '--bg2': '#191228', '--bg3': '#201833', '--bg4': '#2a2042',
    '--line': '#221a38', '--line2': '#32244e',
    '--t1': '#ede9f7', '--t2': '#ab9fc9', '--t3': '#756a94',
    '--accent': '#a382ff', '--accent-h': '#b99cff', '--accent-bg': 'rgba(163,130,255,.14)', '--on-accent': '#150c26',
    '--ok': '#43d99a', '--ok-bg': 'rgba(67,217,154,.12)',
    '--warn': '#e6ba4f', '--warn-bg': 'rgba(230,186,79,.12)',
    '--err': '#f77066', '--err-bg': 'rgba(247,112,102,.12)',
    '--purple': '#f0a6ff', '--purple-bg': 'rgba(240,166,255,.12)',
    '--cat-archive': '#e2a55e', '--cat-archive-bg': 'rgba(226,165,94,.12)',
    '--scroll': '#2e2248', '--scroll-h': '#3c2d5e',
    '--spin-track': 'rgba(0,0,0,.25)', '--spin-dot': '#150c26',
    '--shadow-pop': '0 8px 28px rgba(0,0,0,.5), 0 2px 8px rgba(0,0,0,.35)',
  },
};

const blue = {
  id: 'blue', name: 'Blue', type: 'dark', built_in: true,
  colors: {
    '--bg0': '#0a1020', '--bg1': '#0e1628', '--bg2': '#141f36', '--bg3': '#1a2844', '--bg4': '#233454',
    '--line': '#1b2a44', '--line2': '#2a3d5e',
    '--t1': '#e8eefb', '--t2': '#9babc9', '--t3': '#66789c',
    '--accent': '#4cc2ff', '--accent-h': '#7cd3ff', '--accent-bg': 'rgba(76,194,255,.13)', '--on-accent': '#06101f',
    '--ok': '#3ecf8e', '--ok-bg': 'rgba(62,207,142,.12)',
    '--warn': '#e2b53e', '--warn-bg': 'rgba(226,181,62,.12)',
    '--err': '#f4695e', '--err-bg': 'rgba(244,105,94,.12)',
    '--purple': '#b18cff', '--purple-bg': 'rgba(177,140,255,.13)',
    '--cat-archive': '#e2a55e', '--cat-archive-bg': 'rgba(226,165,94,.12)',
    '--scroll': '#26395a', '--scroll-h': '#324971',
    '--spin-track': 'rgba(0,0,0,.25)', '--spin-dot': '#06101f',
    '--shadow-pop': '0 8px 28px rgba(0,0,0,.5), 0 2px 8px rgba(0,0,0,.35)',
  },
};

export const BUILTIN_THEMES = { dark, light, purple, blue };

/* ------------------------------------------------------------------ */
/* Color parsing + WCAG contrast (pure functions — unit tested)        */
/* ------------------------------------------------------------------ */

const HEX_RE = /^#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i;
// rgba?(…) / hsla?(…) with only numbers, percentages, commas, spaces, dots,
// slash and a leading minus — anything else (url(, var(, calc(…) fails.
const FUNC_COLOR_RE = /^(?:rgb|rgba|hsl|hsla)\(\s*[-0-9.,%apcehlnosu \/]*\)$/i;

/** True when `v` is a plain color value that cannot carry a url() or code. */
export function isSafeColor(v) {
  if (typeof v !== 'string') return false;
  const s = v.trim();
  if (s.length === 0 || s.length > 40) return false;
  if (s.toLowerCase() === 'transparent') return true;
  if (HEX_RE.test(s)) return true;
  return FUNC_COLOR_RE.test(s) && !/[, ]\s*(?:url|var|calc|attr|element)\s*\(/i.test(s);
}

function parseColorParts(s) {
  s = s.trim().toLowerCase();
  if (s.startsWith('#')) {
    let h = s.slice(1);
    if (h.length === 3 || h.length === 4) h = [...h].map((c) => c + c).join('');
    const n = parseInt(h.slice(0, 6), 16);
    const a = h.length === 8 ? parseInt(h.slice(6, 8), 16) / 255 : 1;
    return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255, a];
  }
  const m = s.match(/^rgba?\(([^)]*)\)$/);
  if (m) {
    const parts = m[1].split(/[\s,\/]+/).filter(Boolean);
    const [r, g, b] = parts.slice(0, 3).map((p) => (p.endsWith('%') ? (parseFloat(p) / 100) * 255 : parseFloat(p)));
    const a = parts[3] != null ? (parts[3].endsWith('%') ? parseFloat(parts[3]) / 100 : parseFloat(parts[3])) : 1;
    return [r / 255, g / 255, b / 255, a];
  }
  const h = s.match(/^hsla?\(([^)]*)\)$/);
  if (h) {
    const parts = h[1].split(/[\s,\/]+/).filter(Boolean);
    let deg = parseFloat(parts[0]) || 0;
    const sat = (parseFloat(parts[1]) || 0) / 100;
    const lig = (parseFloat(parts[2]) || 0) / 100;
    const a = parts[3] != null ? parseFloat(parts[3]) : 1;
    deg = ((deg % 360) + 360) % 360;
    const c = (1 - Math.abs(2 * lig - 1)) * sat;
    const x = c * (1 - Math.abs(((deg / 60) % 2) - 1));
    const mm = lig - c / 2;
    let r = 0, g = 0, b = 0;
    if (deg < 60) [r, g, b] = [c, x, 0];
    else if (deg < 120) [r, g, b] = [x, c, 0];
    else if (deg < 180) [r, g, b] = [0, c, x];
    else if (deg < 240) [r, g, b] = [0, x, c];
    else if (deg < 300) [r, g, b] = [x, 0, c];
    else [r, g, b] = [c, 0, x];
    return [r + mm, g + mm, b + mm, a];
  }
  return null;
}

/** sRGB relative luminance (WCAG 2.x). Returns null for unparseable colors. */
export function luminance(color) {
  const p = parseColorParts(color);
  if (!p) return null;
  const lin = p.slice(0, 3).map((ch) => {
    const c = Math.min(1, Math.max(0, ch));
    return c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
  });
  return 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
}

/** WCAG contrast ratio between two colors (1…21). Rendering model: a
 *  translucent `bg` composites onto `base` (the panel behind it), and a
 *  translucent `fg` composites onto the effective bg beneath it. */
export function contrastRatio(fg, bg, base) {
  const mix = (top, bottom) => {
    const t = parseColorParts(top), b = parseColorParts(bottom);
    if (!t || !b) return null;
    if (t[3] >= 1) return top;
    const out = t.slice(0, 3).map((ch, i) => ch * t[3] + b[i] * (1 - t[3]));
    return `rgb(${Math.round(255 * out[0])},${Math.round(255 * out[1])},${Math.round(255 * out[2])})`;
  };
  const effBg = mix(bg, base ?? '#000000') ?? bg;
  const l1 = luminance(mix(fg, effBg) ?? fg);
  const l2 = luminance(effBg);
  if (l1 == null || l2 == null) return null;
  const [a, b] = l1 >= l2 ? [l1, l2] : [l2, l1];
  return (a + 0.05) / (b + 0.05);
}

/**
 * WCAG 2.1 AA for this UI:
 *  - body text (--t1) ≥ 4.5:1 on every surface it can appear on
 *  - secondary text (--t2) ≥ 4.5:1 on panels
 *  - de-emphasized hints (--t3) ≥ 3:1 (timestamps, hints — large/secondary)
 *  - UI components & status colors (--accent, --ok, --warn, --err, --purple)
 *    ≥ 3:1 against panels (SC 1.4.11 non-text contrast)
 * Returns [] when the theme passes, else a list of human-readable failures.
 */
export function contrastReport(theme) {
  const c = theme.colors;
  const fails = [];
  const check = (name, fg, bg, min, base) => {
    const r = base ? contrastRatio(fg, bg, base) : contrastRatio(fg, bg);
    if (r == null) { fails.push(`${name}: unparseable color`); return; }
    if (r < min) fails.push(`${name}: ${r.toFixed(2)}:1 (needs ${min}:1)`);
  };
  for (const bg of ['--bg0', '--bg1', '--bg2']) check(`text on ${bg}`, c['--t1'], c[bg], 4.5);
  check('secondary text on panels', c['--t2'], c['--bg1'], 4.5);
  check('muted text on panels', c['--t3'], c['--bg1'], 3);
  for (const v of ['--accent', '--ok', '--warn', '--err', '--purple', '--cat-archive']) {
    check(`${v} on panels`, c[v], c['--bg1'], 3);
  }
  // accent-tinted selection backgrounds must still carry body text
  check('text on selection tint', c['--t1'], c['--accent-bg'], 4.5, c['--bg1']);
  return fails;
}

/* ------------------------------------------------------------------ */
/* Third-party theme manifests                                         */
/* ------------------------------------------------------------------ */

/**
 * Validate an untrusted parsed-JSON theme manifest. Returns
 * `{ ok: true, theme }` or `{ ok: false, errors: [...] }`.
 *
 * Rules (see docs/THEMES.md): only the known color variables, only safe
 * color values, sane field types. Unknown keys are ignored — a theme can
 * carry extra metadata without breaking — but every variable in THEME_VARS
 * must be present, possibly inherited from `base`.
 */
export function sanitizeTheme(manifest) {
  const errors = [];
  if (manifest == null || typeof manifest !== 'object' || Array.isArray(manifest)) {
    return { ok: false, errors: ['theme must be a JSON object'] };
  }
  const name = typeof manifest.name === 'string' ? manifest.name.trim() : '';
  if (!name || name.length > 40) errors.push('name must be 1–40 characters');
  const id = typeof manifest.id === 'string' && /^[\w-]{1,32}$/.test(manifest.id)
    ? manifest.id.toLowerCase() : null;
  if (!id) errors.push('id must be 1–32 letters, digits, "_" or "-"');
  if (BUILTIN_THEMES[id]) errors.push(`id "${id}" collides with a built-in theme`);
  const type = manifest.type === 'light' ? 'light' : manifest.type === 'dark' ? 'dark' : null;
  if (!type) errors.push('type must be "dark" or "light"');
  const base = manifest.base && BUILTIN_THEMES[manifest.base] ? manifest.base : 'dark';
  const raw = manifest.colors ?? {};
  if (raw == null || typeof raw !== 'object' || Array.isArray(raw)) {
    errors.push('colors must be an object of variable → color');
    return { ok: false, errors };
  }
  const colors = {};
  const allowed = new Set(THEME_VARS);
  for (const [k, v] of Object.entries(raw)) {
    if (!k.startsWith('--')) continue; // ignore non-variable metadata
    if (!allowed.has(k)) { errors.push(`unknown variable "${k}"`); continue; }
    if (!isSafeColor(v)) { errors.push(`unsafe or invalid value for ${k}: ${JSON.stringify(String(v)).slice(0, 60)}`); continue; }
    colors[k] = v.trim();
  }
  if (errors.length) return { ok: false, errors };
  // inherit anything not overridden from the chosen base theme
  const theme = {
    id, name, type, base,
    author: typeof manifest.author === 'string' ? manifest.author.slice(0, 60) : '',
    version: typeof manifest.version === 'string' ? manifest.version.slice(0, 16) : '',
    colors: { ...BUILTIN_THEMES[base].colors, ...colors },
    custom: true,
  };
  const cf = contrastReport(theme);
  if (cf.length) return { ok: false, errors: [...cf, '(contrast failures — adjust the listed colors)'] };
  return { ok: true, theme };
}

/* ------------------------------------------------------------------ */
/* Application + persistence                                           */
/* ------------------------------------------------------------------ */

const LS_CACHE = 'velox.theme.cache';   // {id} or {id, colors} for custom
let customThemes = new Map();           // id -> sanitized theme

export function registerCustomTheme(theme) { customThemes.set(theme.id, theme); }
export function unregisterCustomTheme(id) { customThemes.delete(id); }
export function customThemeList() { return [...customThemes.values()]; }
export function getTheme(id) { return BUILTIN_THEMES[id] ?? customThemes.get(id) ?? null; }

/** Apply a theme to the document. Returns true on success. */
export function applyTheme(id) {
  const t = getTheme(id);
  if (!t) return false;
  if (contrastReport(t).length) return false; // never apply a failing theme
  const root = document.documentElement;
  root.dataset.theme = t.built_in ? t.id : 'custom';
  root.style.colorScheme = t.type;
  if (!t.built_in) {
    for (const [k, v] of Object.entries(t.colors)) root.style.setProperty(k, v);
  } else {
    for (const k of THEME_VARS) root.style.removeProperty(k);
  }
  try {
    localStorage.setItem(LS_CACHE, JSON.stringify(
      t.built_in ? { id: t.id } : { id: t.id, colors: t.colors, type: t.type },
    ));
  } catch { /* private mode — persistence handled by settings file anyway */ }
  return true;
}

/** Boot-time restore: called by app.js after settings load (the synchronous
 *  first-paint hint is assets/theme-boot.js reading the localStorage cache). */
export function initThemes(savedId) {
  if (savedId && applyTheme(savedId)) return true;
  // fall back to whatever theme-boot applied from the cache (e.g. first run
  // after installing a custom theme whose id is not yet in settings)
  try {
    const c = JSON.parse(localStorage.getItem(LS_CACHE) || 'null');
    if (c?.id && !getTheme(c.id) && c.colors) {
      const res = sanitizeTheme({ id: c.id, name: c.id, type: c.type || 'dark', colors: c.colors });
      if (res.ok) { registerCustomTheme(res.theme); if (applyTheme(res.theme.id)) return true; }
    }
  } catch { /* ignore */ }
  return applyTheme('dark');
}

/** Desktop: list installed third-party themes (Tauri command). */
export async function listInstalledThemes() {
  try {
    const t = window.__TAURI__?.core?.invoke
      ? await window.__TAURI__.core.invoke('themes_list') : null;
    const out = [];
    for (const mf of t || []) {
      const res = sanitizeTheme(mf.theme);
      if (res.ok) { registerCustomTheme(res.theme); out.push(res.theme); }
      else out.push({ id: mf.id, name: mf.name || mf.id, invalid: true, errors: res.errors });
    }
    return out;
  } catch {
    return []; // browser preview / command unavailable
  }
}
