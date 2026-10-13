#!/usr/bin/env python3
"""Macht aus dem Originalprogramm die Werk-Ausgabe (Name, Logos, Links).

Läuft im Build auf einer frischen Kopie des Originalcodes. Der Originalcode
selbst wird nirgends dauerhaft verändert, darum lässt sich jede neue Version
des Originals ohne Konflikte übernehmen.

Ein Skript für alle Programme; was sich pro Programm unterscheidet, steht in
werk/apps/<app>/werk.json. Findet das Skript den Programmnamen oder die Icons
nicht mehr, bricht es ab und nennt die Stelle, statt still ein halb
umbenanntes Programm zu bauen.

Aufruf:  python3 werk/rebrand.py <app> <quellordner> [--check]
         <quellordner> ist der ausgecheckte Originalcode; --check prüft nur.
"""
import json
import pathlib
import re
import shutil
import struct
import sys

# Windows-Konsolen nutzen oft eine Codepage ohne «→» und Umlaute.
for stream in (sys.stdout, sys.stderr):
    stream.reconfigure(encoding="utf-8", errors="replace")

ARGS = [a for a in sys.argv[1:] if not a.startswith("--")]
if len(ARGS) != 2:
    sys.exit(__doc__)
CHECK = "--check" in sys.argv
WERK = pathlib.Path(__file__).resolve().parent / "apps" / ARGS[0]
ROOT = pathlib.Path(ARGS[1]).resolve()
CFG = json.loads((WERK / "werk.json").read_bytes().decode("utf-8"))

APP = CFG["app"]  # z. B. "pdfcraft": Programmdatei, Ordner- und Paketnamen (bleibt)
ORIGINAL = CFG["original"]  # z. B. "PdfCraft": Anzeigename im Original
NAME = CFG["name"]  # z. B. "Blattwerk": neuer Anzeigename
SUITE = CFG["suite"]  # z. B. "Werkbank": ersetzt den Namen der Suite
GITHUB = CFG["github"]  # z. B. "0ldini/werkbank": hier erscheinen die Releases
UPSTREAM = CFG["upstream"]  # z. B. "storytold/pdfcraft": das Original
OWNER = GITHUB.split("/")[0]

# Ordner mit Quelltext und Paketierung. Lizenztexte und Urheberhinweise bleiben, wie sie sind.
TEXT_DIRS = ["crates", "apps", "xtask", "packaging"]
KEEP = ("LICENSE", "LICENCE", "NOTICE", "ATTRIBUTION", "OFL", "COPYING")
# Zeilen mit Urheberhinweis werden nie angefasst.
COPYRIGHT = re.compile(r"Copyright|©|\(c\) ?(19|20)\d\d", re.IGNORECASE)

# (Beschreibung, Muster, Ersatz, auch in Urheberzeilen?) in dieser Reihenfolge.
RULES = [
    # Verweise: Hilfe und Downloads führen zur Werkbank statt zu ArtCraft.
    ("Discord-Link", r"(https://)?discord\.gg/artcraft", f"https://github.com/{GITHUB}/issues", True),
    ("Seiten der Apps", r"(https://)?(www\.)?getartcraft\.com/apps(/[a-z]+)?\b", f"https://github.com/{GITHUB}/releases", True),
    ("ArtCraft-Website", r"(https://)?(www\.)?getartcraft\.com", f"https://github.com/{GITHUB}", True),
    # Die eingebaute Update-Prüfung des Programms läuft ins Leere: Updates kommen über den Launcher.
    ("GitHub-API", rf"github\.com/repos/{UPSTREAM}\b", f"github.com/repos/{GITHUB}-kein-eigenes-update", True),
    ("GitHub-Seite", rf"github\.com/{UPSTREAM}\b", f"github.com/{GITHUB}", True),
    # Texte: ehrlich bleiben, von wem das Original stammt, aber keine Zugehörigkeit behaupten.
    ("Herkunft", r"\b(?:from|by) the ArtCraft [Tt]eam", "based on work by the ArtCraft team", False),
    ("Suite", r"ArtCraft(?![ -][Tt]eam)", SUITE, False),
    # «Discord» als Wort, auch gebeugt («Discordu») oder nach einem \n im Text; nicht in Bezeichnern.
    ("Discord", r"(?:(?<![A-Za-z0-9_])|(?<=\\n))Discord(?![A-Z0-9_])", "Forum", False),
    ("Programmname", re.escape(ORIGINAL), NAME, False),
]
REQUIRED = {"Programmname"}

problems: list[str] = []


def read(path: pathlib.Path) -> str:
    """Liest UTF-8, ohne Zeilenenden umzuwandeln (wichtig beim Build unter Windows)."""
    return path.read_bytes().decode("utf-8")


def text_files():
    for d in TEXT_DIRS:
        base = ROOT / d
        if not base.is_dir():
            continue
        for p in sorted(base.rglob("*")):
            if not p.is_file() or p.name.upper().startswith(KEEP) or "target" in p.parts:
                continue
            try:
                yield p, read(p)
            except (UnicodeDecodeError, OSError):
                continue  # Binärdatei


def rewrite_text():
    compiled = [(what, re.compile(pattern), repl, everywhere) for what, pattern, repl, everywhere in RULES]
    hits = {what: 0 for what, *_ in RULES}
    changed = 0
    for path, text in text_files():
        out = []
        for line in text.split("\n"):
            protected = bool(COPYRIGHT.search(line))
            for what, pattern, repl, everywhere in compiled:
                if protected and not everywhere:
                    continue
                line, n = pattern.subn(repl, line)
                hits[what] += n
            out.append(line)
        new = "\n".join(out)
        if new != text:
            changed += 1
            if not CHECK:
                path.write_bytes(new.encode("utf-8"))
    for what, n in hits.items():
        print(f"  {n:6d} × {what}")
        if n == 0 and what in REQUIRED:
            problems.append(f"«{ORIGINAL}» kommt im Quelltext nicht mehr vor")
    print(f"  {changed} Dateien geändert")


def png_size(path: pathlib.Path):
    head = path.read_bytes()[:24]
    if head[:8] != b"\x89PNG\r\n\x1a\n":
        return None
    return struct.unpack(">II", head[16:24])


def put(src: pathlib.Path, dst: pathlib.Path):
    if not src.is_file():
        problems.append(f"Icons: {src.name} fehlt in {WERK.name}/icons")
    elif not CHECK:
        shutil.copyfile(src, dst)


def replace_images():
    icons = WERK / "icons"
    count = 0
    # App-Icon: alles unter assets/app-icon, dazu Favicons der Browser-Version.
    targets = [p for p in sorted((ROOT / "assets" / "app-icon").rglob("*")) if p.is_file()]
    targets += [p for p in sorted((ROOT / "apps").rglob("favicon.*")) if "target" not in p.parts]
    app_id = f"ai.storyteller.{APP}"
    for p in targets:
        stem, ext = p.stem, p.suffix.lower()
        if ext == ".png":
            size = png_size(p)
            if size is None or size[0] != size[1]:
                continue
            put(icons / "png" / f"{size[0]}.png", p)
        elif ext == ".svg" and stem in (APP, app_id, "icon", "favicon"):
            put(icons / "icon.svg", p)
        elif ext == ".svg" and stem == f"{APP}-small":
            put(icons / "icon-small.svg", p)
        elif ext == ".ico" and stem in (APP, "favicon"):
            put(icons / "icon.ico", p)
        else:
            continue  # .icns (nur macOS), Dokument-Icons, Lizenztexte
        count += 1
    if count == 0:
        problems.append("Icons: unter assets/app-icon wurde nichts ersetzt")
    # Marke der Suite: das Programm bettet sie aus docs/brand ein.
    brand = ROOT / "docs" / "brand"
    for src in sorted((icons / "brand").iterdir()):
        if (brand / src.name).is_file():
            put(src, brand / src.name)
            count += 1
    print(f"  {count} Bilddateien ersetzt")


def extend_notice():
    notice = ROOT / "NOTICE"
    addition = (
        f"\n\n{NAME}\n"
        f"{NAME} is a modified version of {ORIGINAL} (https://github.com/{UPSTREAM}), renamed and\n"
        f"repackaged by {OWNER} (https://github.com/{GITHUB}). It is not made, sponsored or endorsed by\n"
        f"the ArtCraft Team. The original copyright notice and licenses above continue to apply.\n"
    )
    if not notice.is_file():
        problems.append("NOTICE fehlt im Original")
        return
    text = read(notice)
    if addition.strip() not in text.replace("\r", "") and not CHECK:
        notice.write_bytes((text.rstrip("\r\n") + addition).encode("utf-8"))
    print("  NOTICE ergänzt")


def main():
    print(f"{ORIGINAL} → {NAME}{'  (nur Prüfung)' if CHECK else ''}")
    print("Texte:")
    rewrite_text()
    print("Bilder:")
    replace_images()
    extend_notice()
    if problems:
        print("\nABBRUCH, das Original hat sich geändert:", file=sys.stderr)
        for p in problems:
            print(f"  - {p}", file=sys.stderr)
        sys.exit(1)
    print("Fertig.")


main()
