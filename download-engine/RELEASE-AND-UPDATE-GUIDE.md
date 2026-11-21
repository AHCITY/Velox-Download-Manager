# RELEASE-AND-UPDATE-GUIDE

## Artifacts (what CI produces)

| Artifact | What it is |
| --- | --- |
| `Velox-Setup-x64.exe` | NSIS setup executable (the installer) — per-user install, Start Menu shortcut, uninstaller, WebView2 bootstrapper |
| `Velox.exe` (inside the setup) | The installed APPLICATION — Tauri 2 window + in-process Rust engine |
| `Velox-app-x64.exe` | The bare application executable (uploaded for CI verification/testing) |

These are different artifacts; CI verifies the application exe exists before packaging
and uploads both with normalized names.

## Binary naming (explicit, verified in CI)

The installed application executable is `Velox.exe`: the app crate declares
`[[bin]] name = "Velox"` (Tauri's recommended way) and `tauri.conf.json` mirrors it via
top-level `mainBinaryName` ("Velox"). `productName` ("Velox") names the installer,
install directory and Start Menu entry — it does NOT determine the exe name. CI asserts
the produced binary is literally `Velox.exe` before packaging.

## Reproducibility

`app/src-tauri` is a standalone Cargo workspace with its own committed `Cargo.lock`
(527 packages resolved). CI runs `cargo fetch --locked` before building — the build
fails instead of silently updating dependencies.

## Building locally (a Windows 10/11 machine with Rust MSVC)

```powershell
cargo install tauri-cli --version "^2" --locked
cd app\src-tauri
cargo tauri build --target x86_64-pc-windows-msvc
# outputs:
#   target\x86_64-pc-windows-msvc\release\Velox.exe
#   target\x86_64-pc-windows-msvc\release\bundle\nsis\Velox_0.2.0_x64-setup.exe
```

Cross-NSIS from Linux is possible per Tauri docs (cargo-xwin) but Velox's `ring`
dependency additionally needs clang-cl — CI on a real Windows runner is the supported path.

## WebView2 strategy (per Microsoft Evergreen recommendation)

- `webviewInstallMode: downloadBootstrapper` — small installer; if the Evergreen runtime
  is missing it is fetched from Microsoft's official CDN at install time.
- Windows 11 and Windows 10 (since the 2022 managed rollout) typically already ship the
  runtime; the bootstrapper covers the edge cases. Fixed Version (~180 MB) is NOT used.
- Offline machines: install the Evergreen Standalone Installer from Microsoft's official
  download page first. No unofficial runtime sources are used anywhere.

## Updates (architecture prepared, activation deferred)

- Activation requires: a minisign keypair (`cargo tauri signer generate`), the private
  key as a GitHub Actions secret (`TAURI_SIGNING_PRIVATE_KEY`), and a release endpoint.
- Config to enable: `bundle.createUpdaterArtifacts: true` (emits `.sig` files) and
  `plugins.updater` with `pubkey` + HTTPS endpoint; capability grants `updater:default`.
- Windows `installMode: passive` is the documented default. Signature verification
  cannot be disabled — no unverified update can install.
- NOT enabled now: faking an update endpoint or keys would produce an update system that
  has never delivered a real update. Deferred until a real release feed exists.

## Code signing (honest status)

The CI-built installer is UNSIGNED. Microsoft's guidance (code-signing options doc):
unsigned installers trigger SmartScreen ("Windows protected your PC"); self-signed certs
block install for the public; the recommended non-Store path is Azure Artifact Signing
(~$9.99/mo, US/CA/EU/UK orgs, US/CA individuals) or an OV certificate (~$150–300/yr,
SmartScreen reputation builds over releases). Store distribution (MSIX) gets free
re-signing. No signing identity is invented here; when credentials exist, add
`certificateThumbprint` to `bundle.windows` and sign in CI with the cert as a secret.
