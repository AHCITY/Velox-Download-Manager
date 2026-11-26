#!/usr/bin/env bash
# Release-page conformance — brings the published release for the version in
# tauri.conf.json to the download policy. Runs in CI (needs a token with
# contents:write + actions:read) and is IDEMPOTENT: exits 0 without changes
# when the page already conforms.
#
# Download policy (numbers make GitHub's alphabetical asset listing show the
# downloads in order, then metadata; labels carry the same numbers so the
# rendered order is correct whichever basis GitHub sorts by):
#
#   1-Velox-Setup-x64.exe                     label: 1. Velox Setup
#   2-Velox-Setup-x64-Offline.exe             label: 2. Velox Setup (Offline)
#   3-Velox-linux-x64-<ver>-offline.tar.gz    label: 3. Velox <ver> for Linux (Offline)
#   4-velox-setup-linux-online.sh             label: 4. Velox Online Installer for Linux
#   latest.json, SHA256SUMS.txt, Velox-Setup-x64-Offline.exe.sig,
#   Velox-linux-x64-<ver>.tar.gz              (metadata, unlabelled)
#
# The updater payload URL inside latest.json is retargeted to the renamed
# offline installer WITHOUT re-signing: the signature covers the payload
# bytes and renaming does not change a single byte of them.
set -euo pipefail

REPO="${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"
VER="$(jq -r .version download-engine/app/src-tauri/tauri.conf.json)"
TAG="v$VER"
LABEL_1="1. Velox Setup"
LABEL_2="2. Velox Setup (Offline)"
LABEL_3="3. Velox $VER for Linux (Offline)"
LABEL_4="4. Velox Online Installer for Linux"

echo "==> Release-page conformance for $TAG ($REPO)"

mapfile -t ASSETS < <(gh api "repos/$REPO/releases/tags/$TAG" --jq '.assets[].name' 2>/dev/null || true)
if [ "${#ASSETS[@]}" -eq 0 ]; then
  echo "    release $TAG has no assets (or does not exist yet) — nothing to do"
  exit 0
fi

has() { local n; for n in "${ASSETS[@]}"; do [ "$n" = "$1" ] && return 0; done; return 1; }

# --- 0. already conforming? (cheap exit — runs on every main push) ----------
if has "2-Velox-Setup-x64-Offline.exe" && has "1-Velox-Setup-x64.exe" \
   && ! has "Velox-Setup-x64-unified.exe" && ! has "Velox-linux-x64-$VER-offline.tar.gz" \
   && ! has "velox-setup-linux-online.sh"; then
  echo "    page already conforms — nothing to do"
  exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

refresh_assets() {
  mapfile -t ASSETS < <(gh api "repos/$REPO/releases/tags/$TAG" --jq '.assets[].name')
}

rename_asset() { # old_name new_name label(optional)
  local old="$1" new="$2" label="${3:-}" id
  if has "$new"; then echo "    $new already present — rename skipped"; return 0; fi
  if ! has "$old"; then
    echo "ERROR: asset '$old' not found (cannot rename to '$new')" >&2
    exit 1
  fi
  id="$(gh api "repos/$REPO/releases/tags/$TAG" --jq ".assets[] | select(.name==\"$old\") | .id")"
  if [ -n "$label" ]; then
    gh api -X PATCH "repos/$REPO/releases/assets/$id" -f name="$new" -f label="$label" --jq .name >/dev/null
  else
    gh api -X PATCH "repos/$REPO/releases/assets/$id" -f name="$new" --jq .name >/dev/null
  fi
  echo "    renamed: $old -> $new"
}

# --- 1. rename + label -------------------------------------------------------
echo "==> Renaming assets to the download policy"
rename_asset "Velox-Setup-x64-unified.exe"         "2-Velox-Setup-x64-Offline.exe"         "$LABEL_2"
rename_asset "Velox-Setup-x64-unified.exe.sig"     "Velox-Setup-x64-Offline.exe.sig"       ""
rename_asset "Velox-linux-x64-$VER-offline.tar.gz" "3-Velox-linux-x64-$VER-offline.tar.gz" "$LABEL_3"
rename_asset "velox-setup-linux-online.sh"         "4-velox-setup-linux-online.sh"         "$LABEL_4"
refresh_assets

# --- 2. the online installer (CI builds it; the release job of old versions never published it)
if ! has "1-Velox-Setup-x64.exe"; then
  echo "==> Attaching the online installer from the windows release run"
  REF="$(gh api "repos/$REPO/git/ref/tags/$TAG")"
  TAG_SHA="$(printf '%s' "$REF" | jq -r .object.sha)"
  if [ "$(printf '%s' "$REF" | jq -r .object.type)" != "commit" ]; then
    TAG_SHA="$(gh api "repos/$REPO/git/tags/$TAG_SHA" --jq .object.sha)"
  fi
  RUN_ID="$(gh run list --workflow=windows.yml --branch "release/windows/v$VER" --status success \
              --json databaseId,headSha \
              --jq "[.[] | select(.headSha==\"$TAG_SHA\")][0].databaseId // \"\"")"
  if [ -z "$RUN_ID" ]; then
    echo "ERROR: no successful windows.yml run for $TAG_SHA — cannot fetch the online installer" >&2
    exit 1
  fi
  echo "    windows run $RUN_ID (commit $TAG_SHA)"
  gh run download "$RUN_ID" -n Velox-Setup-x64-Online -D "$WORK/artifact"
  ONLINE="$WORK/artifact/Velox-Setup-x64-Online.exe"
  # E-19 discipline: never trust a file whose NAME is right — verify content.
  SIZE="$(stat -c%s "$ONLINE")"
  file "$ONLINE" | grep -q "PE32" || { echo "ERROR: online installer is not a Windows PE" >&2; exit 1; }
  if [ "$SIZE" -lt 2000000 ] || [ "$SIZE" -gt 100000000 ]; then
    echo "ERROR: online installer size suspicious: $SIZE bytes" >&2
    exit 1
  fi
  cp "$ONLINE" "$WORK/1-Velox-Setup-x64.exe"
  gh release upload "$TAG" "$WORK/1-Velox-Setup-x64.exe" --clobber
  ID="$(gh api "repos/$REPO/releases/tags/$TAG" --jq '.assets[] | select(.name=="1-Velox-Setup-x64.exe") | .id')"
  gh api -X PATCH "repos/$REPO/releases/assets/$ID" -f label="$LABEL_1" --jq .name >/dev/null
  echo "    uploaded 1-Velox-Setup-x64.exe ($SIZE bytes)"
  refresh_assets
else
  echo "==> 1-Velox-Setup-x64.exe already present — upload skipped"
fi

# --- 3. latest.json: retarget the updater URL (signature stays valid) --------
echo "==> Retargeting latest.json at the renamed offline payload"
gh release download "$TAG" -p latest.json -D "$WORK" --clobber
NEW_URL="https://github.com/$REPO/releases/download/$TAG/2-Velox-Setup-x64-Offline.exe"
jq --arg u "$NEW_URL" '.platforms["windows-x86_64"].url = $u' "$WORK/latest.json" > "$WORK/latest.json.new"
LJ_VER="$(jq -r .version "$WORK/latest.json.new")"
LJ_SIG="$(jq -r '.platforms["windows-x86_64"].signature' "$WORK/latest.json.new")"
LJ_URL="$(jq -r '.platforms["windows-x86_64"].url' "$WORK/latest.json.new")"
if [ "$LJ_VER" != "$VER" ] || [ -z "$LJ_SIG" ] || [ "$LJ_URL" != "$NEW_URL" ]; then
  echo "ERROR: latest.json sanity check failed (ver=$LJ_VER url=$LJ_URL)" >&2
  exit 1
fi
mv "$WORK/latest.json.new" "$WORK/latest.json"
gh release upload "$TAG" "$WORK/latest.json" --clobber
echo "    latest.json -> $LJ_URL (signature unchanged: payload bytes untouched)"

# --- 4. SHA256SUMS.txt: same hashes under the new names + the online installer
echo "==> Regenerating SHA256SUMS.txt"
mkdir -p "$WORK/old" "$WORK/verify"
gh release download "$TAG" -p SHA256SUMS.txt -D "$WORK/old" --clobber
OLD_SUMS="$WORK/old/SHA256SUMS.txt"
[ -s "$OLD_SUMS" ] || { echo "ERROR: release has no SHA256SUMS.txt to remap" >&2; exit 1; }
SUMS="$WORK/SHA256SUMS.txt"
: > "$SUMS"
while read -r hash name; do
  [ -n "$hash" ] || continue
  # files whose CONTENT changed or that were added get fresh hashes below
  [ "$name" = "latest.json" ] && continue
  [ "$name" = "1-Velox-Setup-x64.exe" ] && continue
  case "$name" in
    "Velox-Setup-x64-unified.exe")         name="2-Velox-Setup-x64-Offline.exe" ;;
    "Velox-Setup-x64-unified.exe.sig")     name="Velox-Setup-x64-Offline.exe.sig" ;;
    "Velox-linux-x64-$VER-offline.tar.gz") name="3-Velox-linux-x64-$VER-offline.tar.gz" ;;
    "velox-setup-linux-online.sh")         name="4-velox-setup-linux-online.sh" ;;
  esac
  printf '%s  %s\n' "$hash" "$name" >> "$SUMS"
done < "$OLD_SUMS"

# download EVERY asset and verify the remapped sums against the real bytes
gh release download "$TAG" -D "$WORK/verify" --clobber
( cd "$WORK" && sha256sum latest.json ) >> "$SUMS"
( cd "$WORK/verify" && sha256sum 1-Velox-Setup-x64.exe ) >> "$SUMS"
cp "$SUMS" "$WORK/verify/SHA256SUMS.txt"
( cd "$WORK/verify" && sha256sum -c SHA256SUMS.txt )
gh release upload "$TAG" "$SUMS" --clobber
echo "    SHA256SUMS.txt regenerated and VERIFIED against all downloaded assets"

# --- 5. formal release notes (shared template) --------------------------------
echo "==> Rewriting the release notes"
bash scripts/release-notes.sh "$VER" > "$WORK/notes.md"
grep -q "## Downloads" "$WORK/notes.md"
gh release edit "$TAG" --title "Velox $VER" --notes-file "$WORK/notes.md"
echo "    notes updated from scripts/release-notes.sh"

# --- 6. final state ------------------------------------------------------------
echo "==> Final asset order on the release page (alphabetical = display order):"
gh api "repos/$REPO/releases/tags/$TAG" --jq '.assets | sort_by(.name)[] | "    \(.name)  (\(.size) bytes)  label: \(.label // "-")"'
echo "==> Release-page conformance complete."
