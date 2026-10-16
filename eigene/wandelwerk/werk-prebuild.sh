#!/usr/bin/env bash
# Läuft im Build-Ablauf vor «cargo build» (KIND = windows | linux | web).
# Holt ein fertiges ffmpeg (GPL-Build von github.com/BtbN/FFmpeg-Builds) nach vendor/ und
# setzt WERK_FFMPEG, damit build.rs es in wandelwerk(.exe) einbettet.
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
  *) exit 0 ;;
esac
"$BIN" -hide_banner -encoders | grep -E 'libx264|libvpx-vp9|libmp3lame|libopus|libvorbis' || { echo "ffmpeg ohne nötige Encoder"; exit 1; }
cp vendor/LICENSE.txt LICENSE-ffmpeg.txt
FULL="$PWD/$BIN"
command -v cygpath >/dev/null && FULL=$(cygpath -w "$FULL")
echo "WERK_FFMPEG=$FULL" >> "$GITHUB_ENV"
ls -l vendor
