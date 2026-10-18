#!/usr/bin/env bash
# Läuft im Build-Ablauf vor «cargo build» (KIND = windows | linux | web).
# Windows: holt 7z.exe und 7z.dll aus der neuesten offiziellen 7-Zip-Veröffentlichung von
# Igor Pavlov (github.com/ip7z/7zip, wie auf 7-zip.org) nach vendor/ und setzt WERK_7Z_DIR,
# damit build.rs sie in ladewerk.exe einbettet. Linux nutzt das 7z/unrar/tar des Systems.
set -euo pipefail
cd "$(dirname "$0")"
[ "${KIND:-}" = "windows" ] || exit 0
arch=x64
[ "${PROCESSOR_ARCHITECTURE:-}" = "ARM64" ] && arch=arm64
# Neueste Version über die Weiterleitung von …/releases/latest (braucht keine Anmeldung;
# die GitHub-Schnittstelle sperrt ohne Anmeldung schnell). Mit Token geht auch die Schnittstelle.
url=""
tok="${GH_TOKEN:-${GITHUB_TOKEN:-}}"
if [ -n "$tok" ]; then
  url=$(curl -fsSL -H "Authorization: Bearer $tok" https://api.github.com/repos/ip7z/7zip/releases/latest 2>/dev/null \
    | grep -o "\"browser_download_url\": *\"[^\"]*/7z[0-9]*-$arch\.exe\"" | head -1 | sed 's/.*"\(https[^"]*\)"/\1/') || true
fi
if [ -z "$url" ]; then
  tag=$(curl -fsSI https://github.com/ip7z/7zip/releases/latest | tr -d '\r' | sed -n 's#^[Ll]ocation: .*/releases/tag/\(.*\)$#\1#p' | tail -1)
  [ -n "$tag" ] || { echo "Neueste 7-Zip-Version nicht gefunden"; exit 1; }
  url="https://github.com/ip7z/7zip/releases/download/$tag/7z${tag//./}-$arch.exe"
fi
echo "7-Zip: $url"
rm -rf vendor && mkdir -p vendor
curl -fsSL -o vendor/7z-setup.exe "$url"
SZ=$(command -v 7z || command -v 7z.exe || echo "/c/Program Files/7-Zip/7z.exe")
"$SZ" x -y -ovendor vendor/7z-setup.exe 7z.exe 7z.dll License.txt >/dev/null
rm vendor/7z-setup.exe
for f in 7z.exe 7z.dll License.txt; do [ -f "vendor/$f" ] || { echo "$f fehlt"; exit 1; }; done
cp vendor/License.txt LICENSE-7zip.txt
DIR="$PWD/vendor"
command -v cygpath >/dev/null && DIR=$(cygpath -w "$DIR")
echo "WERK_7Z_DIR=$DIR" >> "$GITHUB_ENV"
ls -l vendor
