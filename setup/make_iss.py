"""Erzeugt das Inno-Setup-Skript für das Offline-Setup aus launcher/apps.toml.

Aufruf: python3 setup/make_iss.py <version> <stage-ordner> <ausgabe.iss>
Der Stage-Ordner enthält werkbank.exe und bundle/<app>-…-windows-x64-portable.zip (+ web-zips).
"""
import pathlib, sys, tomllib

version, stage, out = sys.argv[1], pathlib.Path(sys.argv[2]), pathlib.Path(sys.argv[3])
apps = tomllib.loads(pathlib.Path("launcher/apps.toml").read_text(encoding="utf-8"))["app"]
bundle = stage / "bundle"
present = {a["id"]: sorted(p.name for p in bundle.glob(f'{a["id"]}-*')) for a in apps}
apps = [a for a in apps if present[a["id"]]]

lines = [f"""; Automatisch erzeugt von setup/make_iss.py
#define AppVersion "{version}"

[Setup]
AppName=Werkbank
AppVersion={{#AppVersion}}
AppPublisher=0ldini
AppPublisherURL=https://github.com/0ldini/werkbank
DefaultDirName={{localappdata}}\\Programs\\Werkbank
PrivilegesRequired=lowest
DisableProgramGroupPage=yes
OutputDir=out
OutputBaseFilename=werkbank-setup-{{#AppVersion}}
Compression=lzma2/fast
SolidCompression=no
WizardStyle=modern
SetupIconFile={stage / "werkbank.ico"}
UninstallDisplayIcon={{app}}\\werkbank.exe
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "de"; MessagesFile: "compiler:Languages\\German.isl"

[Types]
Name: "full"; Description: "Alle Programme"
Name: "custom"; Description: "Auswahl"; Flags: iscustom

[Components]
Name: "launcher"; Description: "Werkbank-Launcher (nötig)"; Types: full custom; Flags: fixed"""]
for a in apps:
    lines.append(f'Name: "app_{a["id"]}"; Description: "{a["name"]} – {a.get("about", "")}"; Types: full')

lines.append("""
[Tasks]
Name: "online"; Description: "Vor dem Einrichten nach neueren Versionen suchen (braucht Internet)"
Name: "desktopicon"; Description: "Verknüpfung auf dem Desktop anlegen"; Flags: unchecked

[Files]
Source: "{stage}\\werkbank.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{stage}\\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{stage}\\LICENSE"; DestDir: "{app}"; Flags: ignoreversion""".replace("{stage}", str(stage)))
for a in apps:
    for f in present[a["id"]]:
        lines.append(f'Source: "{bundle / f}"; DestDir: "{{app}}\\bundle"; Components: app_{a["id"]}; Flags: ignoreversion')

lines.append('''
[Icons]
Name: "{autoprograms}\\Werkbank"; Filename: "{app}\\werkbank.exe"
Name: "{autodesktop}\\Werkbank"; Filename: "{app}\\werkbank.exe"; Tasks: desktopicon

; Nach dem Kopieren startet der Launcher und richtet die Programme aus dem Bündel ein.
[Run]
Filename: "{app}\\werkbank.exe"; Parameters: "--bundle ""{app}\\bundle"""; Flags: nowait postinstall; Description: "Werkbank starten und die Programme einrichten"; Tasks: online
Filename: "{app}\\werkbank.exe"; Parameters: "--bundle ""{app}\\bundle"" --offline"; Flags: nowait postinstall; Description: "Werkbank starten und die Programme einrichten"; Tasks: not online

[UninstallDelete]
Type: filesandordirs; Name: "{app}\\bundle"
Type: filesandordirs; Name: "{localappdata}\\Werkbank"
''')
out.write_text("\n".join(lines), encoding="utf-8-sig")
print(f"{out}: {len(apps)} Programme, {sum(len(v) for v in present.values())} Archive")
