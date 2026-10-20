#!/usr/bin/env bash
# Packt eine fertige Programmdatei als macOS-Programm (Name.app), ad hoc signiert.
# Aufruf: mac_bundle.sh <Anzeigename> <programmdatei> <icon.icns> <bundle-id> <version> <zielordner>
# Ergebnis: <zielordner>/<Anzeigename>.app (Contents/MacOS/<Name der Programmdatei>).
set -euo pipefail
NAME="$1"; EXE="$2"; ICNS="$3"; ID="$4"; VERSION="$5"; OUT="$6"
BIN=$(basename "$EXE")
APP="$OUT/$NAME.app"
SHORT="${VERSION%%-*}"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$EXE" "$APP/Contents/MacOS/$BIN"
chmod 755 "$APP/Contents/MacOS/$BIN"
cp "$ICNS" "$APP/Contents/Resources/AppIcon.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>de</string>
  <key>CFBundleDisplayName</key><string>$NAME</string>
  <key>CFBundleName</key><string>$NAME</string>
  <key>CFBundleExecutable</key><string>$BIN</string>
  <key>CFBundleIdentifier</key><string>$ID</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$SHORT</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
PLIST
plutil -lint "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"
# Ohne Apple-Entwicklerkonto: ad hoc signieren (Apple Silicon verlangt eine Signatur).
codesign --force --deep --sign - --timestamp=none "$APP"
codesign --verify --strict --deep --verbose=2 "$APP"
echo "$APP"
