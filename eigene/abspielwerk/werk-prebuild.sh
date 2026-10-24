#!/usr/bin/env bash
# Läuft im Build-Ablauf vor «cargo build» (KIND = windows | linux | macos | web).
# Holt libVLC (offizieller VLC-Build von videolan.org) nach vendor/bundle/. Der Ablauf legt den
# Inhalt von vendor/bundle neben die Programmdatei (macOS: nach Contents/MacOS/).
# Linux: nichts mitliefern, Abspielwerk lädt das libvlc.so.5 des Systems (Paket «vlc»).
set -euo pipefail
trap 'echo "::error title=werk-prebuild ($KIND)::Zeile $LINENO: $BASH_COMMAND"' ERR
cd "$(dirname "$0")"
VLC_VER=3.0.23

dl() { curl -fsSL --retry 5 --retry-all-errors --retry-delay 10 -o "$1" "$2"; }
# get.videolan.org leitet auf einen Spiegelserver um; download.videolan.org ist der Ursprung.
fetch() { # $1 Ziel, $2 Pfad unter vlc/
  local base
  for base in https://get.videolan.org/vlc https://download.videolan.org/pub/videolan/vlc; do
    if dl "$1" "$base/$2"; then echo "geladen: $base/$2"; return 0; fi
  done
  return 1
}
sha() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }
pruefen() { # $1 Datei, $2 Pfad unter vlc/ — Prüfsumme vom Ursprungsserver
  if dl "$1.sha256" "https://download.videolan.org/pub/videolan/vlc/$2.sha256" || fetch "$1.sha256" "$2.sha256"; then
    want=$(grep -oE '[0-9a-f]{64}' "$1.sha256" | head -1 || true)
    have=$(sha "$1")
    [ -n "$want" ] && [ "$want" = "$have" ] || { echo "::error::$1: Prüfsumme stimmt nicht ($have / $want)"; exit 1; }
    echo "$1: Prüfsumme stimmt ($have)"
  else
    echo "::warning::Keine Prüfsumme für $2 gefunden"
  fi
  rm -f "$1.sha256"
}
lizenz() { # $@ gefundene Lizenzdateien von VLC
  {
    echo "libVLC $VLC_VER (VideoLAN, https://www.videolan.org)"
    echo
    echo "Abspielwerk bringt libVLC und seine Plugins unverändert mit. libVLC steht unter der"
    echo "GNU Lesser General Public License, Version 2.1 oder später (LGPL-2.1-or-later);"
    echo "einzelne Plugins unter der GNU General Public License, Version 2 oder später."
    echo "Quelltext: https://code.videolan.org/videolan/vlc"
    echo "LGPL 2.1: https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html"
    echo "GPL 2:    https://www.gnu.org/licenses/old-licenses/gpl-2.0.html"
    for f in "$@"; do
      [ -f "$f" ] || continue
      echo; echo "==================== $(basename "$f") ===================="; echo
      cat "$f"
    done
  } > LICENSE-vlc.txt
}

# Plugin-Ordner, die ein eingebetteter Player nicht braucht (Oberflächen, Lua-Skripte,
# Visualisierungen, Streaming-Ausgabe, Fernsteuerung, Netzwerk-Dienste).
UNNOETIG="gui lua visualization video_splitter control services_discovery stream_out access_output mux keystore"

rm -rf vendor
mkdir -p vendor/bundle
case "$KIND" in
  windows)
    P="$VLC_VER/win64/vlc-$VLC_VER-win64.zip"
    fetch vendor/vlc.zip "$P"
    pruefen vendor/vlc.zip "$P"
    mkdir -p vendor/tmp
    unzip -q -o vendor/vlc.zip -d vendor/tmp
    rm vendor/vlc.zip
    SRC=$(dirname "$(find vendor/tmp -maxdepth 2 -name libvlc.dll | head -1)")
    [ -f "$SRC/libvlc.dll" ] || { echo "::error::libvlc.dll fehlt im Archiv"; exit 1; }
    cp "$SRC/libvlc.dll" "$SRC/libvlccore.dll" vendor/bundle/
    cp -R "$SRC/plugins" vendor/bundle/
    for d in $UNNOETIG; do rm -rf "vendor/bundle/plugins/$d"; done
    # Plugin-Verzeichnis neu schreiben (sonst liest libVLC bei jedem Start alle Plugins ein).
    rm -f vendor/bundle/plugins/plugins.dat
    if [ -f "$SRC/vlc-cache-gen.exe" ]; then
      timeout 300 "$SRC/vlc-cache-gen.exe" "$(cygpath -w "$PWD/vendor/bundle/plugins" 2>/dev/null || echo "$PWD/vendor/bundle/plugins")" \
        || echo "::warning::vlc-cache-gen fehlgeschlagen; libVLC liest die Plugins beim Start ein"
    fi
    if [ -f vendor/bundle/plugins/plugins.dat ]; then echo "::notice::plugins.dat neu geschrieben"; else echo "::warning::kein plugins.dat"; fi
    lizenz "$SRC/COPYING.txt" "$SRC/COPYING.LIB.txt" "$SRC/COPYING.LIB"
    rm -rf vendor/tmp ;;
  macos)
    DMG=""
    for name in "vlc-$VLC_VER-arm64.dmg" "vlc-$VLC_VER-universal.dmg"; do
      if fetch vendor/vlc.dmg "$VLC_VER/macosx/$name"; then DMG="$VLC_VER/macosx/$name"; break; fi
    done
    [ -n "$DMG" ] || { echo "::error::VLC für macOS nicht ladbar"; exit 1; }
    pruefen vendor/vlc.dmg "$DMG"
    MNT=$(mktemp -d)
    hdiutil attach -nobrowse -readonly -noautoopen -mountpoint "$MNT" vendor/vlc.dmg >/dev/null
    APPD=$(find "$MNT" -maxdepth 2 -name 'VLC.app' | head -1)
    [ -n "$APPD" ] || { echo "::error::VLC.app fehlt im dmg"; ls -la "$MNT"; exit 1; }
    M="$APPD/Contents/MacOS"
    ls -la "$M" "$M/lib" || true
    mkdir -p vendor/bundle/lib
    # Gleiche Anordnung wie in VLC.app: lib/ und plugins/ neben der Programmdatei.
    ditto "$M/lib" vendor/bundle/lib
    ditto "$M/plugins" vendor/bundle/plugins
    lizenz $(find "$APPD/Contents/Resources" -maxdepth 2 -iname 'COPYING*' 2>/dev/null)
    hdiutil detach "$MNT" >/dev/null || hdiutil detach -force "$MNT" >/dev/null || true
    rm vendor/vlc.dmg
    # Unnötige Plugins (Oberfläche, Lua, Visualisierungen, Streaming-Ausgabe).
    for pat in 'libmacosx_*' 'liblua_*' 'libgoom_*' 'libprojectm_*' 'libvisual_*' 'libglspectrum_*' 'libstream_out_*' 'libaccess_output_*' 'libmux_*' 'libchromecast_*' 'libupnp_*' 'libmicrodns_*' 'libbonjour_*'; do
      find vendor/bundle/plugins -name "$pat" -delete
    done
    # libvlc und die Plugins verweisen auf @rpath/libvlccore.dylib, ohne eigenen Suchpfad
    # (VLC.app setzt ihn in der Programmdatei; Abspielwerk ebenso, siehe build.rs). Zusätzlich
    # eigene Suchpfade eintragen, damit es auch ohne den der Programmdatei klappt.
    for f in vendor/bundle/lib/*.dylib; do
      [ -L "$f" ] || install_name_tool -add_rpath @loader_path "$f" 2>/dev/null || true
    done
    find vendor/bundle/plugins -type f -name '*.dylib' | while read -r f; do
      rel=${f#vendor/bundle/plugins/}; up="../"; d=$(dirname "$rel")
      while [ "$d" != "." ]; do up="../$up"; d=$(dirname "$d"); done
      install_name_tool -add_rpath "@loader_path/${up}lib" "$f" 2>/dev/null || true
    done
    # Hinweise für die Fehlersuche (Abhängigkeiten von libvlc und einem Plugin).
    echo "::notice title=libvlc.dylib::$(otool -L vendor/bundle/lib/libvlc.dylib | tr '\n' ' ') RPATH: $(otool -l vendor/bundle/lib/libvlc.dylib | grep -A2 LC_RPATH | grep path | tr '\n' ' ')"
    P1=$(find vendor/bundle/plugins -name 'libavcodec_plugin.dylib' | head -1)
    [ -n "$P1" ] && echo "::notice title=plugin::$(otool -L "$P1" | head -4 | tr '\n' ' ') RPATH: $(otool -l "$P1" | grep -A2 LC_RPATH | grep path | tr '\n' ' ')"
    ;;
  *) rm -rf vendor; exit 0 ;;
esac
du -sh vendor/bundle
echo "::notice title=libVLC $KIND::$(du -sh vendor/bundle | cut -f1) in vendor/bundle ($(find vendor/bundle -type f | wc -l | tr -d ' ') Dateien)"
