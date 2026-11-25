// First-paint theme restore — loaded synchronously by index.html BEFORE the
// app css paints rows, so there is never a flash of the wrong theme.
// Reads the localStorage cache written by themes.js (which keeps it current);
// on desktop the authoritative value arrives later via settings (initThemes).
(() => {
  try {
    const c = JSON.parse(localStorage.getItem('velox.theme.cache') || 'null');
    const root = document.documentElement;
    if (c && typeof c.id === 'string' && /^[\w-]{1,32}$/.test(c.id)
        && ['dark', 'light', 'purple', 'blue'].includes(c.id)) {
      root.dataset.theme = c.id;
      root.style.colorScheme = c.id === 'light' ? 'light' : 'dark';
    }
    // custom themes are applied by app.js once their colors are loaded;
    // until then the base theme paints (no flash, no unstyled content)
  } catch { /* first run / private mode → dark (CSS default) */ }
})();
