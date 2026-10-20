#!/usr/bin/env bash
# Läuft im Build-Ablauf vor «cargo build» (KIND = windows | linux | macos | web).
# Holt ein fertiges ffmpeg (GPL-Build von github.com/BtbN/FFmpeg-Builds) nach vendor/ und
# setzt WERK_FFMPEG, damit build.rs es in diawerk(.exe) einbettet.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p vendor
case "$KIND" in
  windows)
    curl -fsSL -o vendor/ff.zip https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip
    unzip -q -o -j vendor/ff.zip '*/bin/ffmpeg.exe' '*/LICENSE.txt' -d vendor
    rm vendor/ff.zip
    BIN=vendor/ffmpeg.exe ;;
  linux)
    curl -fsSL -o vendor/ff.tar.xz https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz
    tar -xJf vendor/ff.tar.xz -C vendor --wildcards --strip-components=2 '*/bin/ffmpeg'
    tar -xJf vendor/ff.tar.xz -C vendor --wildcards --strip-components=1 '*/LICENSE.txt'
    rm vendor/ff.tar.xz
    BIN=vendor/ffmpeg ;;
  macos)
    # Apple Silicon: statischer Build von ffmpeg.martin-riedl.de (mit x264, vpx, opus, lame, freetype).
    curl -fsSL -o vendor/ff.zip https://ffmpeg.martin-riedl.de/redirect/latest/macos/arm64/release/ffmpeg.zip
    unzip -q -o -j vendor/ff.zip ffmpeg -d vendor
    rm vendor/ff.zip
    chmod 755 vendor/ffmpeg
    printf '%s\n' "ffmpeg für macOS (Apple Silicon): statischer Build von https://ffmpeg.martin-riedl.de" \
      "Enthält GPL-Bibliotheken (u. a. x264); es gilt die GNU General Public License, Version 3:" \
      "https://www.gnu.org/licenses/gpl-3.0.txt — Quelltext: https://ffmpeg.org" > vendor/LICENSE.txt
    BIN=vendor/ffmpeg ;;
  *) exit 0 ;;
esac
# Diawerk braucht: H.264, VP9, AAC, Opus (Ausgabeformate) und drawtext (Vor-/Abspann).
for enc in libx264 libvpx-vp9 aac libopus; do
  "$BIN" -hide_banner -encoders | grep -q " $enc " || { echo "ffmpeg ohne Encoder $enc"; exit 1; }
done
"$BIN" -hide_banner -filters | grep -q " drawtext " || { echo "ffmpeg ohne Filter drawtext"; exit 1; }
cp vendor/LICENSE.txt LICENSE-ffmpeg.txt
FULL="$PWD/$BIN"
command -v cygpath >/dev/null && FULL=$(cygpath -w "$FULL")
echo "WERK_FFMPEG=$FULL" >> "$GITHUB_ENV"
ls -l vendor
