#!/usr/bin/env python3
"""Baut ein Wörterbuch fest in ein Programm ein (Schreibwerk: Deutsch (Schweiz)).

Das Original prüft Deutsch nur mit einem Sprachpaket, das man selbst herunterladen muss; die
Browser-Version kann gar keine Pakete installieren. Dieses Skript läuft im Build nach
rebrand.py, wenn es werk/apps/<app>/rechtschreibung.json gibt:

  1. holt die Kiste craft-proof (storytold/craft-libs) im Stand, den das Original festlegt,
     nach <quellordner>/../werk-craft-libs und trägt sie im Cargo.toml des Originals als
     Pfad statt als Git-Abhängigkeit ein;
  2. lädt das Wörterbuch (Adressen und SHA-256 fest in rechtschreibung.json), legt es gepackt
     (raw deflate) mit seinen Lizenztexten in die Kiste und fügt das Modul werk_de_ch.rs ein;
  3. trägt Einschübe ins Original ein (neue Dokumente in Deutsch (Schweiz) usw., alle optional);
  4. ergänzt NOTICE um Herkunft, Lizenz und die vollen Lizenztexte des Wörterbuchs (NOTICE
     liegt jedem Paket bei: Windows, Linux, Browser).

Jede veränderte Datei des Originals wird vorher gesichert; «--zurueck» stellt sie wieder her
(der Build baut dann ohne eingebautes Wörterbuch, aber mit allen Werk-Übersetzungen).

Aufruf:  python3 werk/tools/rechtschreibung.py <app> <quellordner> [--zurueck]
Endet mit 0, wenn eingebaut (oder nichts zu tun), sonst 1 (der Build fährt dann ohne fort).
"""
import hashlib
import json
import pathlib
import re
import shutil
import subprocess
import sys
import time
import urllib.request
import zlib

for stream in (sys.stdout, sys.stderr):
    stream.reconfigure(encoding="utf-8", errors="replace")

ARGS = [a for a in sys.argv[1:] if not a.startswith("--")]
if len(ARGS) != 2:
    sys.exit(__doc__)
ZURUECK = "--zurueck" in sys.argv
WERK = pathlib.Path(__file__).resolve().parent.parent / "apps" / ARGS[0]
ROOT = pathlib.Path(ARGS[1]).resolve()
CFG_FILE = WERK / "rechtschreibung.json"
LIBS = ROOT.parent / "werk-craft-libs"
BACKUP = ROOT.parent / "werk-rechtschreibung-sicherung"
MARK = "Werkbank: eingebautes Wörterbuch"


class Fehler(Exception):
    pass


def read(p: pathlib.Path) -> str:
    return p.read_bytes().decode("utf-8")


def backup(rel: str):
    """Sichert <ROOT>/rel einmal (vor der ersten Änderung)."""
    src, dst = ROOT / rel, BACKUP / "dateien" / rel
    if dst.exists() or not src.is_file():
        return
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(src, dst)
    with (BACKUP / "liste.txt").open("a", encoding="utf-8") as f:
        f.write(rel + "\n")


def restore():
    liste = BACKUP / "liste.txt"
    if not liste.is_file():
        print("  nichts zurückzustellen")
        return
    for rel in read(liste).splitlines():
        if rel.strip():
            shutil.copy2(BACKUP / "dateien" / rel, ROOT / rel)
            print(f"  zurück: {rel}")
    shutil.rmtree(BACKUP, ignore_errors=True)


def first_file(base: pathlib.Path, rel):
    rels = rel if isinstance(rel, list) else [rel]
    return next((r for r in rels if (base / r).is_file()), rels[0])


def patch_lines(base: pathlib.Path, m: dict, save=None) -> bool:
    """Ein Einschub wie in rebrand.py: ganze Ankerzeile, «after», «before» oder «replace»."""
    rel = first_file(base, m["file"])
    path = base / rel
    optional = m.get("optional", False)
    if not path.is_file():
        if optional:
            print(f"  (Einschub entfällt, {rel} fehlt)")
            return False
        raise Fehler(f"{rel} fehlt")
    text = read(path)
    nl = "\r\n" if "\r\n" in text else "\n"
    rows = text.split(nl)
    anchor = m["anchor"].strip()
    hits = [i for i, r in enumerate(rows) if r.strip() == anchor]
    if len(hits) != 1:
        if optional and not hits:
            print(f"  (Einschub entfällt, Ankerzeile fehlt in {rel})")
            return False
        raise Fehler(f"Ankerzeile {len(hits)}× statt 1× in {rel}: {anchor}")
    i = hits[0]
    indent = rows[i][: len(rows[i]) - len(rows[i].lstrip())]
    kind = m.get("kind", "after")
    lines = [indent + l for l in m["lines"]]
    if kind == "after":
        rows[i + 1 : i + 1] = lines
    elif kind == "before":
        rows[i:i] = lines
    else:
        rows[i : i + 1] = lines
    if save:
        save(rel)
    path.write_bytes(nl.join(rows).encode("utf-8"))
    print(f"  Einschub in {rel}")
    return True


def download(url: str, sha256: str) -> bytes:
    last = None
    for attempt in range(4):
        try:
            with urllib.request.urlopen(url, timeout=60) as r:
                data = r.read()
            if hashlib.sha256(data).hexdigest() != sha256:
                raise Fehler(f"{url}: Prüfsumme stimmt nicht")
            return data
        except Fehler:
            raise
        except Exception as e:  # Netzwerk: kurz warten, noch einmal
            last = e
            time.sleep(3 * (attempt + 1))
    raise Fehler(f"{url}: {last}")


def as_text(data: bytes) -> str:
    """Lizenztexte als UTF-8 (die Readmes des Wörterbuchs sind ISO 8859-1)."""
    try:
        return data.decode("utf-8")
    except UnicodeDecodeError:
        return data.decode("latin-1")


def git(*args, cwd=None):
    r = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True)
    if r.returncode != 0:
        raise Fehler(f"git {' '.join(args)}: {r.stderr.strip()[-400:]}")
    return r.stdout


def point_cargo_at_copy(cfg) -> str:
    """Die Zeile der Git-Abhängigkeit craft-proof im Cargo.toml des Originals wird zur
    Pfad-Abhängigkeit auf die vorbereitete Kopie. Gibt den Stand (rev) zurück."""
    cargo = ROOT / "Cargo.toml"
    text = read(cargo)
    nl = "\r\n" if "\r\n" in text else "\n"
    rows = text.split(nl)
    crate = re.escape(cfg["crate"])
    repo = re.escape(cfg["repo"])
    want = re.compile(rf'^\s*(?:{crate}\s*=|[\w-]+\s*=\s*\{{[^}}]*package\s*=\s*"{crate}")[^#]*git\s*=\s*"{repo}(?:\.git)?"')
    hits = [i for i, r in enumerate(rows) if want.search(r)]
    if len(hits) != 1:
        raise Fehler(f"Cargo.toml: {cfg['crate']} aus {cfg['repo']} {len(hits)}× statt 1×")
    row = rows[hits[0]]
    rev = re.search(r'rev\s*=\s*"([0-9a-f]{7,40})"', row)
    if not rev:
        raise Fehler(f"Cargo.toml: kein rev für {cfg['crate']}: {row.strip()}")
    rel = (pathlib.Path("..") / LIBS.name / cfg["crate_dir"]).as_posix()
    new = re.sub(rf'git\s*=\s*"{repo}(?:\.git)?"', f'path = "{rel}"', row)
    new = re.sub(r',\s*(rev|branch|tag)\s*=\s*"[^"]*"', "", new)
    backup("Cargo.toml")
    backup("Cargo.lock")
    rows[hits[0]] = new
    cargo.write_bytes(nl.join(rows).encode("utf-8"))
    print(f"  Cargo.toml: {cfg['crate']} aus {rel} (Stand {rev.group(1)[:12]})")
    return rev.group(1)


def fetch_crate(cfg, rev: str):
    if LIBS.exists():
        shutil.rmtree(LIBS)
    LIBS.mkdir(parents=True)
    git("init", "-q", cwd=LIBS)
    git("fetch", "-q", "--depth", "1", cfg["repo"], rev, cwd=LIBS)
    git("-c", "advice.detachedHead=false", "checkout", "-q", "FETCH_HEAD", cwd=LIBS)
    crate = LIBS / cfg["crate_dir"]
    if not (crate / "Cargo.toml").is_file():
        raise Fehler(f"{cfg['crate_dir']} fehlt in {cfg['repo']}@{rev[:12]}")
    print(f"  {cfg['repo']}@{rev[:12]} geholt")
    return crate


def notice(cfg, texts: dict):
    """NOTICE (liegt jedem Paket bei) um das Wörterbuch ergänzen, mit den vollen Lizenztexten."""
    path = ROOT / "NOTICE"
    if not path.is_file():
        raise Fehler("NOTICE fehlt")
    text = read(path)
    if MARK in text:
        return
    parts = [
        "",
        "",
        "=" * 100,
        f"{MARK} Deutsch (Schweiz) / Built-in German (Switzerland) spelling dictionary",
        "=" * 100,
        "",
        "This build of Schreibwerk contains the Hunspell dictionary de_CH_frami (German, Switzerland),",
        "built into the program so German (Switzerland) text is spell-checked without a Language pack.",
        "It is not part of WordCraft or craft-libs; Werkbank adds it at build time.",
        "",
        "  Dictionary: de_CH_frami, version 2017-01-09, by Franz Michael Baumann, based on the",
        "              igerman98 dictionary (2016-12-07) by Björn Jacke (https://www.j3e.de/ispell/igerman98/)",
        f"  Source:     {cfg['source']}",
        "              (files de_CH_frami.aff and de_CH_frami.dic, unmodified; stored compressed)",
        f"  Licence:    {cfg['licence']} (GNU General Public License, version 2 or version 3)",
        "",
        "The dictionary is free software: you can redistribute it and/or modify it under the terms",
        "of the GNU General Public License, version 2 or version 3, as published by the Free Software",
        "Foundation. It is distributed WITHOUT ANY WARRANTY. Its source (the .aff and .dic files above)",
        "is available at the source address, and the build that adds it is public at",
        "https://github.com/0ldini/werkbank (werk/tools/rechtschreibung.py). The upstream notices",
        "and both licence texts follow.",
    ]
    for name, body in texts.items():
        parts += ["", "-" * 100, name, "-" * 100, "", body.replace("\r\n", "\n").rstrip("\n")]
    backup("NOTICE")
    nl = "\r\n" if "\r\n" in text else "\n"
    path.write_bytes((text.rstrip("\r\n") + nl + nl.join(parts) + nl).encode("utf-8"))
    print("  NOTICE ergänzt (Herkunft und Lizenz des Wörterbuchs)")


def apply():
    if not CFG_FILE.is_file():
        print("  (kein eingebautes Wörterbuch für dieses Programm)")
        return
    cfg = json.loads(read(CFG_FILE))
    shutil.rmtree(BACKUP, ignore_errors=True)
    BACKUP.mkdir(parents=True)
    try:
        rev = point_cargo_at_copy(cfg)
        crate = fetch_crate(cfg, rev)
        data = crate / cfg["data_dir"]
        data.mkdir(parents=True, exist_ok=True)
        texts = {}
        for f in cfg["files"]:
            raw = download(f["url"], f["sha256"])
            if f.get("deflate"):
                c = zlib.compressobj(9, zlib.DEFLATED, -15)
                out = c.compress(raw) + c.flush()
                (data / f["to"]).write_bytes(out)
                print(f"  {f['to']}: {len(raw)} → {len(out)} Bytes")
            else:
                body = as_text(raw)
                (data / f["to"]).write_bytes(body.encode("utf-8"))
                if f.get("licence_text"):
                    texts[f["to"]] = body
        shutil.copyfile(WERK / cfg["module"], crate / "src" / pathlib.Path(cfg["module"]).name)
        for m in cfg.get("crate_patches", []):
            patch_lines(crate, {**m, "optional": False})
        for m in cfg.get("patches", []):
            patch_lines(ROOT, m, save=backup)
        notice(cfg, texts)
    except Exception as e:
        print(f"\nABBRUCH (ohne eingebautes Wörterbuch weiter): {e}", file=sys.stderr)
        restore()
        shutil.rmtree(LIBS, ignore_errors=True)
        sys.exit(1)
    print("Eingebautes Wörterbuch: fertig.")


def main():
    print(f"Rechtschreibung {ARGS[0]}{'  (zurück)' if ZURUECK else ''}:")
    if ZURUECK:
        restore()
        shutil.rmtree(LIBS, ignore_errors=True)
    else:
        apply()


main()
