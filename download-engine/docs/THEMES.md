# Velox Themes

Velox ships five built-in themes — **Dark** (default), **Light**, **Purple**,
**Blue** and **Soft Canopy** — and supports **user-installed third-party themes**.

## How it works

- A theme is a set of CSS custom properties (variables) applied to the app's
  design system. Every color in the UI comes from these variables; components
  never hard-code colors, so a theme change restyles everything at once.
- Built-ins are defined twice on purpose: as JS objects
  (`web/assets/themes.js`) and as `[data-theme="…"]` blocks in
  `web/assets/app.css`. The CSS blocks make the first paint instant (no flash
  of the wrong theme); a unit test (`web/tests/themes.test.mjs`) asserts the
  two stay exactly in sync.
- Your choice persists in the app settings (`theme`) and is re-applied at
  startup before the window paints.

## Third-party themes

### Format

A theme is a **JSON file**. JSON was chosen over raw CSS deliberately: a theme
*cannot contain selectors, `url()` references, `@import`, or any executable
content by construction* — themes are pure data and cannot run code, fetch
remote resources, or alter layout. Structure:

```json
{
  "id": "midnight-rose",
  "name": "Midnight Rose",
  "author": "you",
  "version": "1.0.0",
  "type": "dark",
  "base": "dark",
  "colors": {
    "--bg0": "#12070f",
    "--bg1": "#1a0c16",
    "--accent": "#ff7ac6",
    "--accent-h": "#ff9ad6",
    "--accent-bg": "rgba(255,122,198,.14)"
  }
}
```

| Field | Required | Rules |
|---|---|---|
| `id` | yes | 1–32 chars, letters/digits/`_`/`-`; must not collide with a built-in |
| `name` | yes | 1–40 chars, shown in Settings |
| `type` | yes | `"dark"` or `"light"` (drives `color-scheme` for native controls) |
| `base` | no | built-in to inherit unspecified variables from (`dark`/`light`/`purple`/`blue`; default `dark`) |
| `colors` | yes | variable → color map (see below); partial overrides allowed |
| `author`, `version` | no | informational, displayed in the picker |

### Accepted values

Only plain **colors**: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()`,
`rgba()`, `hsl()`, `hsla()` (alpha forms blend over the panel they sit on) and
`transparent`. The validator rejects everything else — `url()`, `var()`,
`calc()`, gradients, unknown variable names, non-string values — so a hostile
manifest cannot beacon, exfiltrate, or break the UI. Unknown metadata fields
are ignored.

### Variables

Every theme defines (or inherits from `base`) this complete set:

| Group | Variables |
|---|---|
| Surfaces | `--bg0` (app background) `--bg1` (panels) `--bg2` (raised/hover) `--bg3` (inputs) `--bg4` (active) |
| Lines | `--line` `--line2` |
| Text | `--t1` (primary) `--t2` (secondary) `--t3` (muted/hints) |
| Accent | `--accent` `--accent-h` (hover) `--accent-bg` (selection tint) `--on-accent` (text on accent fills) |
| Status | `--ok` `--ok-bg` `--warn` `--warn-bg` `--err` `--err-bg` `--purple` `--purple-bg` |
| Categories | `--cat-archive` `--cat-archive-bg` |
| Controls | `--scroll` `--scroll-h` `--spin-track` `--spin-dot` |
| Depth | `--shadow-pop` |

Derived tints (borders, focus rings, badges) are computed at render time with
CSS `color-mix()` from the solid colors — custom themes get them for free.

### Contrast (accessibility)

On install, a theme must pass the WCAG 2.1 AA check or it is **rejected** with
the exact failing ratios:

- `--t1` ≥ **4.5:1** on every surface (`--bg0/1/2`) and on the selection tint
- `--t2` ≥ **4.5:1** on panels; `--t3` ≥ **3:1** (muted hints)
- every status/accent color ≥ **3:1** on panels (non-text contrast, SC 1.4.11)

The same check runs in CI for all four built-ins — the shipped themes are
guaranteed readable.

### Install / remove

- **Settings → Appearance → Install theme…** — pick a `.json` file. It is
  validated (format + contrast) and stored in the app data `themes/` folder:
  `%APPDATA%\io.velox.downloadmanager\themes\` (Windows),
  `~/.local/share/io.velox.downloadmanager/themes/` (Linux).
- Apply by clicking its swatch; **Refresh list** re-reads the folder (you can
  also drop files there manually).
- Remove with the ✕ on a custom swatch, or by deleting the file.

### The themes folder inside the installation directory

Velox maintains a `themes/` folder INSIDE the installation directory
(`%LOCALAPPDATA%\Programs\Velox\themes\` with the per-user Windows setup;
beside `Velox` on Linux). It is seeded at startup with the canonical manifest
of every built-in theme:

    dark.json · light.json · purple.json · blue.json · soft-canopy.json

- The folder is a real theme library: drop your own `.json` manifests next to
  the built-ins and they appear in Settings → Appearance after **Refresh list**
  (the per-user data dir keeps working too; with the same id, the install-dir
  copy wins).
- Missing manifests are re-seeded on every start, but existing files are
  NEVER overwritten — edits to a built-in copy survive app updates.
- Want to build on a built-in? Copy `soft-canopy.json`, change its `id`, tweak
  the colors, drop it in the same folder — it installs as a third-party theme
  after the WCAG check.

## Community themes (online catalog)

Settings → Appearance → **Community themes** opens the community catalog: a
modal listing downloadable third-party themes from the configured online
sources, with a live preview and one-click download.

- Sources are configured in `web/assets/themes.js` → `COMMUNITY_SOURCES`
  (array of registry URLs). The catalog ships EMPTY — no registry is announced
  yet; adding one URL is all it takes to light the catalog up.
- A registry serves JSON of the shape
  `{ "themes": [ { "id", "name", "type", "base", "author", "version",
  "description", "colors": { … } } ] }` — a bare themes array works too.
- Every entry runs through the same `sanitizeTheme()` gate as local installs:
  only known color variables, only safe color values, WCAG contrast check.
  Invalid entries are dropped and never offered.
- **Preview** applies the theme live (same rendering path as `applyTheme`)
  but never persists — closing the dialog or pressing "Stop preview" reverts.
- **Download** persists the theme: browser mode in localStorage
  (`velox.community.themes.v1`), desktop ALSO in the themes folder via the
  existing `theme_install` command. Downloaded themes appear in the picker
  like any installed theme and survive restarts (boot re-registers them
  before the active theme is restored).
- Downloaded themes that are no longer in the catalog (offline registry,
  removed source, or no sources configured) stay listed and usable under
  "Your downloaded themes".
- The whole dialog is built from the shared `.overlay`/`.dialog` primitives
  and theme variables only — it inherits every theme (built-in or custom)
  with no extra styling work.
