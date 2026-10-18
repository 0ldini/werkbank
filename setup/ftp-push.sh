#!/usr/bin/env bash
# Lädt Dateien per FTP(S) auf den Server (Ordner «software» auf jarns.ch).
#
#   ftp-push.sh mirror <lokaler-ordner> [ziel-unterordner]   nur fehlende Dateien hochladen
#   ftp-push.sh put <datei> [ziel-unterordner] [zielname]    Datei hochladen (überschreibt)
#
# Zugangsdaten kommen aus Umgebungsvariablen (auf GitHub: Settings → Secrets and variables → Actions):
#   FTP_HOST        Server, z. B. jarns.ch  (Pflicht)
#   FTP_USER        Benutzer                 (Pflicht)
#   FTP_PASSWORD    Passwort                 (Pflicht)
#   FTP_DIR         Ordner «software» auf dem Server, vom FTP-Wurzelordner aus (Standard: software)
#   FTP_SSL_FORCE   yes (Standard) = verschlüsselt erzwingen; no = auch unverschlüsselt
#   FTP_VERIFY_CERT yes (Standard); no, falls das Zertifikat des Servers nicht zum Namen passt
set -euo pipefail

: "${FTP_HOST:?FTP_HOST fehlt}" "${FTP_USER:?FTP_USER fehlt}" "${FTP_PASSWORD:?FTP_PASSWORD fehlt}"
mode="${1:?Aufruf: ftp-push.sh mirror|put <quelle> [ziel]}"
src="${2:?Quelle fehlt}"
sub="${3:-}"
name="${4:-}"
base="${FTP_DIR:-software}"
dest="${base%/}${sub:+/$sub}"
target="$FTP_HOST"
case "$target" in *://*) ;; *) target="ftp://$target" ;; esac

cmds="$(mktemp)"
trap 'rm -f "$cmds"' EXIT
{
  echo "set cmd:fail-exit yes"
  echo "set net:max-retries 3"
  echo "set net:timeout 30"
  echo "set ftp:ssl-allow yes"
  echo "set ftp:ssl-force ${FTP_SSL_FORCE:-yes}"
  echo "set ssl:verify-certificate ${FTP_VERIFY_CERT:-yes}"
  echo "mkdir -p -f \"$dest\""
  case "$mode" in
    mirror) echo "mirror -R --only-missing --no-perms --parallel=2 --verbose \"$src\" \"$dest\"" ;;
    put)
      if [ -n "$name" ]; then
        echo "put -O \"$dest\" \"$src\" -o \"$name\""
      else
        echo "put -O \"$dest\" \"$src\""
      fi
      ;;
    *) echo "unbekannter Modus: $mode" >&2; exit 2 ;;
  esac
} > "$cmds"

lftp -u "$FTP_USER,$FTP_PASSWORD" "$target" -f "$cmds"
