"""Legt werk/apps/<app>/ an: werk.json und alle Icons.

Aufruf: python3 werk/tools/make_app.py <schrift.ttf> [app ...]     (ohne App: alle)
Die Schrift (z. B. Inter-SemiBold.ttf aus dem Originalcode, assets/fonts) wird nur
gebraucht, um den Schriftzug «Werkbank» in Pfade umzuwandeln; sie wird nicht mitgeliefert.
Braucht Python mit Pillow, fontTools und Playwright (Chromium).
"""
import json, pathlib, subprocess, sys

HERE = pathlib.Path(__file__).resolve().parent
APPS_DIR = HERE.parent / "apps"
REPO = "0ldini/werkbank"
# app: (Anzeigename im Original, neuer Name, Beschreibung im Launcher)
APPS = {
    "photocraft": ("PhotoCraft", "Bildwerk", "Bildbearbeitung mit Ebenen und Masken"),
    "vectorcraft": ("VectorCraft", "Linienwerk", "Vektorgrafik und Illustration"),
    "designcraft": ("DesignCraft", "Designwerk", "Layout für Flyer, Hefte und Bücher"),
    "pdfcraft": ("PdfCraft", "Blattwerk", "PDFs lesen, ordnen und schützen"),
    "lightcraft": ("LightCraft", "Lichtwerk", "Fotobibliothek und RAW-Entwicklung"),
    "filmcraft": ("FilmCraft", "Filmwerk", "Videoschnitt, Farbe und Ton"),
    "effectcraft": ("EffectCraft", "Trickwerk", "Animation und visuelle Effekte"),
    "wordcraft": ("WordCraft", "Schreibwerk", "Briefe, Berichte und Dokumente"),
    "deckcraft": ("DeckCraft", "Schauwerk", "Präsentationen und Folien"),
    "soundcraft": ("SoundCraft", "Klangwerk", "Musik und Ton aufnehmen und mischen"),
    "cadcraft": ("CADCraft", "Planwerk", "Technisches Zeichnen (CAD)"),
    "gridcraft": ("GridCraft", "Rechenwerk", "Tabellen und Berechnungen"),
}

font = sys.argv[1]
for app in sys.argv[2:] or APPS:
    original, name, _ = APPS[app]
    out = APPS_DIR / app
    out.mkdir(parents=True, exist_ok=True)
    cfg = {"app": app, "name": name, "original": original, "suite": "Werkbank", "github": REPO, "upstream": f"storytold/{app}"}
    (out / "werk.json").write_text(json.dumps(cfg, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    subprocess.run([sys.executable, str(HERE / "make_icons.py"), app, str(out / "icons"), font], check=True)
    print(f"{app}: {name}")
