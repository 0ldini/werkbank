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
    # Einzelne Dokumente, die Programme einbetten (z. B. «Neuigkeiten» in Lichtwerk).
    for rel in ("docs/whats-new.md",):
        p = ROOT / rel
        if p.is_file():
            try:
                yield p, read(p)
            except (UnicodeDecodeError, OSError):
                pass
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


# ---------------------------------------------------------------------------------------------
# Sprache: Der Werkbank-Launcher legt Deutsch oder Englisch fest (Umgebungsvariable WERKBANK_LANG
# beim Start, sonst die Datei sprache.txt im Einstellungsordner der Werkbank). Ein kleiner Einschub
# je Programm lässt diese Wahl vor der gespeicherten Einstellung und der Systemsprache gelten.
# Programme ohne Übersetzungen (Klangwerk, Schauwerk) bleiben englisch.

# Rust-Ausdruck vom Typ Option<String>: «de», «en» oder None (dann gilt die Wahl im Programm).
WB = (
    '(std::env::var("WERKBANK_LANG").ok().or_else(|| { '
    'let base = if cfg!(windows) { std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Werkbank").join("config") } '
    'else if cfg!(target_os = "macos") { std::path::PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/Werkbank") } '
    'else { std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).filter(|p| p.is_absolute())'
    '.or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?.join("werkbank") }; '
    'std::fs::read_to_string(base.join("sprache.txt")).ok() })'
    '.map(|v| v.trim().to_ascii_lowercase()).filter(|v| v == "de" || v == "en"))'
)

_FROM_PREF = (
    "after",
    "pub fn from_pref(pref: &str) -> Lang {",
    0,
    [
        "// Werkbank: Sprache aus dem Launcher.",
        "static WERKBANK: OnceLock<Option<Lang>> = OnceLock::new();",
        f"if let Some(lang) = *WERKBANK.get_or_init(|| {WB}.and_then(|v| Lang::from_code(&v))) {{ return lang; }}",
    ],
)

# Programm → [(Datei, (Art, Ankerzeile, Zeilen danach überspringen, neue Zeilen))]
#   Art: «after» fügt nach der Ankerzeile (+ übersprungene Zeilen) ein, «before» davor,
#        «replace» ersetzt die Ankerzeile.
LANG_PATCHES = {
    "photocraft": [("crates/ui-egui/src/i18n/mod.rs", _FROM_PREF)],
    "vectorcraft": [("crates/ui-egui/src/i18n/mod.rs", _FROM_PREF)],
    "pdfcraft": [("crates/ui-egui/src/i18n/mod.rs", _FROM_PREF)],
    "wordcraft": [("apps/wordcraft/src/main.rs", ("after", "load_prefs(&mut app);", 0, [
        f"if let Some(code) = {WB}.and_then(|v| wordcraft_ui_egui::i18n::normalize_pref(&v)) {{ app.ui.language = code.to_string(); }}",
    ]))],
    "gridcraft": [("apps/gridcraft/src/main.rs", ("after", "load_prefs(&mut app);", 0, [
        f"if let Some(l) = {WB}.and_then(|v| gridcraft_ui_egui::i18n::Language::parse(&v)) {{ app.ui.language = l; }}",
    ]))],
    "cadcraft": [("apps/cadcraft/src/main.rs", ("after", "app.load_prefs(&prefs);", 1, [
        f"if let Some(p) = {WB}.and_then(|v| cadcraft_ui_egui::i18n::Preference::parse(&v)) {{ app.ui.interface_language = p; }}",
    ]))],
    "lightcraft": [("apps/lightcraft/src/main.rs", ("before", "lightcraft_ui_egui::i18n::set_language(app.ui.language);", 0, [
        f"if let Some(l) = {WB}.and_then(|v| lightcraft_ui_egui::i18n::Locale::parse_tag(&v)) {{ app.ui.language = l; }}",
    ]))],
    "designcraft": [("apps/designcraft/src/main.rs", ("before", "// Engine preferences (Preferences dialog, favourites…) live beside the UI state.", 0, [
        f'match {WB}.as_deref() {{ Some("de") => app.ui.language = "de".into(), Some("en") => app.ui.language = String::new(), _ => {{}} }}',
    ]))],
    "filmcraft": [("crates/ui-egui/src/lib.rs", ("replace", "let language = match i18n::Language::parse(&p.general.interface_language) {", 0, [
        f"let werkbank = {WB}.and_then(|v| i18n::Language::parse(&v));",
        "let language = match werkbank.or_else(|| i18n::Language::parse(&p.general.interface_language)) {",
    ]))],
    "effectcraft": [("crates/ui-egui/src/i18n.rs", ("after", "pub(crate) fn language(app: &EffectcraftApp) -> &'static str {", 0, [
        "static WERKBANK: std::sync::OnceLock<Option<&'static str>> = std::sync::OnceLock::new();",
        f'if let Some(l) = *WERKBANK.get_or_init(|| {WB}.map(|v| if v == "de" {{ "de" }} else {{ "en" }})) {{ return l; }}',
    ]))],
}


def patch_language():
    patches = LANG_PATCHES.get(APP, [])
    if not patches:
        print("  (keine Übersetzungen im Original)")
    for rel, (kind, anchor, skip, lines) in patches:
        path = ROOT / rel
        if not path.is_file():
            problems.append(f"Sprache: {rel} fehlt")
            continue
        text = read(path)
        nl = "\r\n" if "\r\n" in text else "\n"
        rows = text.split(nl)
        hits = [i for i, r in enumerate(rows) if r.strip() == anchor]
        if len(hits) != 1:
            problems.append(f"Sprache: Ankerzeile {len(hits)}× statt 1× in {rel}: {anchor}")
            continue
        i = hits[0]
        indent = rows[i][: len(rows[i]) - len(rows[i].lstrip())]
        if kind == "after":
            inner = indent + ("    " if anchor.endswith("{") else "")
            at = i + 1 + skip
            if skip:
                inner = rows[i + skip][: len(rows[i + skip]) - len(rows[i + skip].lstrip())]
            rows[at:at] = [inner + l for l in lines]
        elif kind == "before":
            rows[i:i] = [indent + l for l in lines]
        else:
            rows[i : i + 1] = [indent + l for l in lines]
        if not CHECK:
            path.write_bytes(nl.join(rows).encode("utf-8"))
        print(f"  Sprachwahl der Werkbank in {rel}")


# ---------------------------------------------------------------------------------------------
# Ergänzungen je Programm aus werk/apps/<app>/werk.json und *.extra.json (alle optional):
#   "merge_tsv":  [{"src": "de.tsv", "dst": "crates/ui-egui/src/i18n/de.tsv"}]
#                 Zeilen aus werk/apps/<app>/<src> anhängen, deren Schlüssel (alle Spalten ausser
#                 der letzten) im Ziel noch fehlt. Kommentarzeilen (#) werden übersprungen.
#   "merge_json": [{"src": "de.json", "dst": "crates/ui-egui/locales/de.json"}]
#                 Schlüssel ergänzen, die im Ziel (flaches JSON-Objekt) fehlen.
#   "add_files":  [{"src": "werk_de.rs", "dst": "crates/ui-egui/src/werk_de.rs"}]   kopieren
#   "patches":    [{"file": …, "anchor": "eine ganze Zeile", "kind": "after|before|replace",
#                   "skip": 0, "lines": ["…"]}]   wie die Sprach-Einschübe oben
#   "wrap":       [{"globs": ["crates/ui-egui/src/**/*.rs"], "calls": ["ui.button(", …],
#                   "func": "crate::werk_de::t", "exclude": ["tests.rs"]}]
#                 Ein Zeichenketten-Literal direkt nach einem der Aufrufe wird zu func("…").
#                 Zeilen mit «const », «static », «#[» oder Kommentaren bleiben unberührt.

def _patch_lines(rel, kind, anchor, skip, lines):
    path = ROOT / rel
    if not path.is_file():
        problems.append(f"Einschub: {rel} fehlt")
        return
    text = read(path)
    nl = "\r\n" if "\r\n" in text else "\n"
    rows = text.split(nl)
    hits = [i for i, r in enumerate(rows) if r.strip() == anchor.strip()]
    if len(hits) != 1:
        problems.append(f"Einschub: Ankerzeile {len(hits)}× statt 1× in {rel}: {anchor}")
        return
    i = hits[0]
    indent = rows[i][: len(rows[i]) - len(rows[i].lstrip())]
    if kind == "after":
        inner = indent + ("    " if anchor.rstrip().endswith("{") else "")
        if skip:
            r = rows[i + skip]
            inner = r[: len(r) - len(r.lstrip())]
        at = i + 1 + skip
        rows[at:at] = [inner + l for l in lines]
    elif kind == "before":
        rows[i:i] = [indent + l for l in lines]
    else:
        rows[i : i + 1] = [indent + l for l in lines]
    if not CHECK:
        path.write_bytes(nl.join(rows).encode("utf-8"))


def _tsv_key(line):
    parts = line.rstrip("\r").split("\t")
    return "\t".join(parts[:-1]) if len(parts) > 1 else None


def extras():
    import fnmatch
    # werk.json und alle Teile werk/apps/<app>/*.extra.json (z. B. sprache.extra.json,
    # thema.extra.json): Listen gleichen Namens werden aneinandergehängt.
    cfg = {}
    for part in [WERK / "werk.json"] + sorted(WERK.glob("*.extra.json")):
        data = json.loads(read(part))
        for key in ("merge_tsv", "merge_json", "add_files", "patches", "wrap"):
            cfg.setdefault(key, []).extend(data.get(key, []))
    n = 0
    for m in cfg.get("merge_tsv", []):
        dst, src = ROOT / m["dst"], WERK / m["src"]
        if not dst.is_file() or not src.is_file():
            problems.append(f"merge_tsv: {m['dst']} oder {m['src']} fehlt")
            continue
        text = read(dst)
        nl = "\r\n" if "\r\n" in text else "\n"
        have = {_tsv_key(l) for l in text.split(nl) if l and not l.startswith("#")}
        add = []
        for l in read(src).replace("\r", "").split("\n"):
            if not l or l.startswith("#"):
                continue
            k = _tsv_key(l)
            if k is not None and k not in have:
                have.add(k)
                add.append(l)
        if add and not CHECK:
            dst.write_bytes((text.rstrip("\r\n") + nl + nl.join(add) + nl).encode("utf-8"))
        print(f"  {len(add)} Übersetzungen ergänzt in {m['dst']}")
        n += 1
    for m in cfg.get("merge_json", []):
        dst, src = ROOT / m["dst"], WERK / m["src"]
        if not dst.is_file() or not src.is_file():
            problems.append(f"merge_json: {m['dst']} oder {m['src']} fehlt")
            continue
        base = json.loads(read(dst))
        extra = json.loads(read(src))
        added = 0
        for k, v in extra.items():
            if k not in base:
                base[k] = v
                added += 1
        if not CHECK:
            dst.write_bytes((json.dumps(base, ensure_ascii=False, indent=2) + "\n").encode("utf-8"))
        print(f"  {added} Übersetzungen ergänzt in {m['dst']}")
    for m in cfg.get("add_files", []):
        src = WERK / m["src"]
        if not src.is_file():
            problems.append(f"add_files: {m['src']} fehlt")
            continue
        if not CHECK:
            (ROOT / m["dst"]).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(src, ROOT / m["dst"])
        print(f"  Datei {m['dst']}")
    for m in cfg.get("patches", []):
        _patch_lines(m["file"], m.get("kind", "after"), m["anchor"], m.get("skip", 0), m["lines"])
        print(f"  Einschub in {m['file']}")
    for m in cfg.get("wrap", []):
        calls = sorted(m["calls"], key=len, reverse=True)
        pat = re.compile("(" + "|".join(re.escape(c) for c in calls) + r')\s*("(?:[^"\\\n]|\\.)*")')
        func = m["func"]
        skip = re.compile(r"^\s*(//|const |static |pub const |pub static |#\[)")
        count = 0
        for g in m["globs"]:
            for path in sorted(ROOT.glob(g)):
                rel = path.relative_to(ROOT).as_posix()
                if any(fnmatch.fnmatch(rel, e) or path.name == e for e in m.get("exclude", [])):
                    continue
                text = read(path)
                out = []
                changed = False
                in_test = False
                for line in text.split("\n"):
                    if re.match(r"\s*#\[cfg\(test\)\]", line):
                        in_test = True
                    if in_test or skip.match(line):
                        out.append(line)
                        continue
                    new, k = pat.subn(lambda mm: f'{mm.group(1)}{func}({mm.group(2)})', line)
                    if k:
                        changed = True
                        count += k
                    out.append(new)
                if changed and not CHECK:
                    path.write_bytes("\n".join(out).encode("utf-8"))
        print(f"  {count} Texte übersetzbar gemacht ({func})")


# Schweizer Rechtschreibung: in den deutschen Übersetzungen «ss» statt «ß».
ESZETT_NAMES = {"de.tsv", "de.json", "de-formats.json", "de.rs", "de_werk.rs", "ui_de.rs", "effects_de.rs"}
# Dateien, die deutsche Tabellen neben anderem Code enthalten.
ESZETT_EXTRA = {"designcraft": ["crates/ui-egui/src/i18n.rs"], "effectcraft": ["crates/ui-egui/src/i18n.rs"]}


def swiss_spelling():
    base = ROOT / "crates" / "ui-egui"
    files = [p for p in sorted(base.rglob("*")) if p.is_file() and p.name in ESZETT_NAMES and "target" not in p.parts] if base.is_dir() else []
    files += [ROOT / f for f in ESZETT_EXTRA.get(APP, []) if (ROOT / f).is_file()]
    n = 0
    for p in files:
        text = read(p)
        k = text.count("ß") + text.count("ẞ")
        if k and not CHECK:
            p.write_bytes(text.replace("ß", "ss").replace("ẞ", "SS").encode("utf-8"))
        n += k
    print(f"  {n} × ß → ss in {len(files)} Dateien")


def main():
    print(f"{ORIGINAL} → {NAME}{'  (nur Prüfung)' if CHECK else ''}")
    # Vor dem Umbenennen: die Ankerzeilen stehen so im Original.
    print("Sprache:")
    patch_language()
    print("Ergänzungen:")
    extras()
    swiss_spelling()
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
