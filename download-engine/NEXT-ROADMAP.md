# NEXT-ROADMAP

Updated after the desktop milestone.

## 1. Execute the Windows pipeline (the single blocking step)
- Push → `windows.yml` runs engine gates + desktop build on `windows-latest`.
- Fix anything the real runner finds (first real MSVC compile of the app crate).
- Execute INSTALLER-TEST-RESULTS.md's 14-step plan on Win10 22H2 x64 and Win11 x64.

## 2. Signing & release feed (unblocks public distribution)
- Obtain a signing identity (Azure Artifact Signing or OV cert — see
  RELEASE-AND-UPDATE-GUIDE.md) and wire `certificateThumbprint` into CI.
- Create the updater keypair + JSON release feed on GitHub Releases; enable
  `createUpdaterArtifacts` + `plugins.updater` (architecture already documented).

## 3. Engine items carried over (unchanged priority)
- Finalize-window recovery marker (SIGKILL between rename and DB commit → duplicate file
  with no checksum configured; data-safe, needs manifest change + adoption logic).
- Lock-file residue: verify-on-acquire (inode/file-index) + unlink-on-drop so
  `.part-N.lock` files disappear after completion.
- Long-path destination test in CI; cookie PSL bundling.

## 4. Desktop polish (after first real install test)
- System tray (optional) with quick pause/resume; window-state persistence.
- In-app settings surface (theme is dark-only by design; downloads dir override; per-host
  connection caps).
