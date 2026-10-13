"""Erzeugt alle Icon-Dateien einer Werk-App aus Farbe + Zeichen.

Aufruf: python3 make_icons.py <app-id> <zielordner>
Ausgabe im Zielordner: icon.svg, icon-small.svg, icon-1024.png, icon.ico,
hicolor/<n>x<n>.png, sowie die Werkbank-Marke (mark.svg, logo.svg, logo-white.svg).
"""
import io, re, sys, pathlib
from PIL import Image
from playwright.sync_api import sync_playwright
from fontTools.ttLib import TTFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen

S = 'fill="none" stroke="#fff" stroke-linecap="round" stroke-linejoin="round"'
APPS = {  # id: (Name, Farbe, Zeichen im 24er-Raster)
    "photocraft": ("Bildwerk", "#2F6FDE", '<rect x="3" y="4" width="18" height="16" rx="2"/><circle cx="9" cy="9.5" r="1.6"/><path d="M4 18l5-5 4 4 3-3 4 4"/>'),
    "vectorcraft": ("Linienwerk", "#D9601F", '<path d="M5 18C5 7 19 17 19 6"/><circle cx="5" cy="18" r="1.9" fill="#fff"/><circle cx="19" cy="6" r="1.9" fill="#fff"/>'),
    "designcraft": ("Designwerk", "#B8328A", '<rect x="4" y="3" width="16" height="18" rx="2"/><path d="M8 7h8M8 11h3M8 14h3M8 17h3"/><rect x="13.5" y="11" width="2.5" height="6"/>'),
    "pdfcraft": ("Blattwerk", "#C93636", '<path d="M7 3h7l5 5v12a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1z"/><path d="M14 3v5h5"/><path d="M9 13h6M9 17h4"/>'),
    "lightcraft": ("Lichtwerk", "#17879E", '<circle cx="12" cy="12" r="4"/><path d="M12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6L7 7M17 17l1.4 1.4M5.6 18.4L7 17M17 7l1.4-1.4"/>'),
    "filmcraft": ("Filmwerk", "#7448D0", '<rect x="3" y="5" width="18" height="14" rx="2"/><path d="M7 5v14M17 5v14"/><path d="M10.5 9.5v5l4-2.5z" fill="#fff"/>'),
    "effectcraft": ("Trickwerk", "#A97A0A", '<path d="M11 3l1.9 5.1L18 10l-5.1 1.9L11 17l-1.9-5.1L4 10l5.1-1.9z"/><path d="M18 15v5M15.5 17.5h5"/>'),
    "wordcraft": ("Schreibwerk", "#2A7A4B", '<path d="M5 6h14M5 10h14M5 14h14M5 18h8"/>'),
    "soundcraft": ("Klangwerk", "#D0457A", '<path d="M4 10v4M8 6v12M12 3v18M16 7v10M20 10v4"/>'),
    "cadcraft": ("Planwerk", "#4A5568", '<path d="M4 20V5l15 15z"/><path d="M8 17v-4l4 4z"/>'),
    "gridcraft": ("Rechenwerk", "#5E8C1A", '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 10h18M3 15h18M9 4v16"/>'),
    "deckcraft": ("Schauwerk", "#3B4BA8", '<rect x="3" y="4" width="18" height="12" rx="1.5"/><path d="M12 16v4M8 20h8"/><path d="M7 12l3-3 2 2 4-4"/>'),
}
AMBER, INK = "#F2B544", "#17181C"
TILES = '<rect x="3" y="3" width="8" height="8" rx="2"/><rect x="13" y="3" width="8" height="8" rx="2"/><rect x="3" y="13" width="8" height="8" rx="2"/><rect x="13" y="13" width="8" height="8" rx="2" opacity=".45"/>'


def app_svg(color, glyph, small=False):
    scale, width = (17.5, 2.3) if small else (14.0, 1.8)
    off = (512 - 24 * scale) / 2
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><rect width="512" height="512" rx="112" fill="{color}"/>'
            f'<g transform="translate({off} {off}) scale({scale})" {S} stroke-width="{width}">{glyph}</g></svg>\n')


def mark_svg():
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" role="img" aria-label="Werkbank"><title>Werkbank</title>'
            f'<rect width="512" height="512" rx="112" fill="{AMBER}"/><g transform="translate(64 64) scale(16)" fill="{INK}">{TILES}</g></svg>\n')


def logo_svg(font_path, text_color):
    """Marke + Schriftzug «Werkbank», Buchstaben als Pfade (kein Font zur Laufzeit nötig)."""
    font = TTFont(font_path)
    glyphs, cmap, upm = font.getGlyphSet(), font.getBestCmap(), font["head"].unitsPerEm
    size, x, paths = 78.0, 124.0, []
    k = size / upm
    for ch in "Werkbank":
        name = cmap[ord(ch)]
        pen = SVGPathPen(glyphs)
        glyphs[name].draw(TransformPen(pen, (k, 0, 0, -k, x, 78)))
        paths.append(pen.getCommands())
        x += glyphs[name].width * k
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {x + 4:.0f} 100" role="img" aria-label="Werkbank"><title>Werkbank</title>'
            f'<rect width="100" height="100" rx="22" fill="{AMBER}"/><g transform="translate(12.5 12.5) scale(3.125)" fill="{INK}">{TILES}</g>'
            f'<path fill="{text_color}" d="{" ".join(paths)}"/></svg>\n')


def render(page, svg, w, h=None):
    h = h or w
    page.set_viewport_size({"width": w, "height": h})
    page.set_content(f'<body style="margin:0;background:transparent">{svg.replace("<svg ", f"<svg width={w} height={h} ", 1)}</body>')
    return Image.open(io.BytesIO(page.screenshot(omit_background=True))).convert("RGBA")


def main():
    """Ausgabe: icon.svg, icon-small.svg, icon.ico, png/<n>.png, brand/artcraft-*.{svg,png}
    (die Marken-Dateien tragen die Namen ihrer Ziele in docs/brand)."""
    app, out, font = sys.argv[1], pathlib.Path(sys.argv[2]), sys.argv[3]
    _, color, glyph = APPS[app]
    (out / "png").mkdir(parents=True, exist_ok=True)
    (out / "brand").mkdir(parents=True, exist_ok=True)
    big, small = app_svg(color, glyph), app_svg(color, glyph, small=True)
    (out / "icon.svg").write_text(big)
    (out / "icon-small.svg").write_text(small)
    mark, logo, logo_w = mark_svg(), logo_svg(font, "#0b0b0c"), logo_svg(font, "#ffffff")
    brand = {"artcraft-mark": mark, "artcraft-mark-black": mark, "artcraft-logo": logo, "artcraft-logo-white": logo_w}
    for name, svg in brand.items():
        (out / "brand" / f"{name}.svg").write_text(svg)
    with sync_playwright() as p:
        browser = p.chromium.launch()
        page = browser.new_page()
        pick = lambda n: small if n <= 32 else big
        for n in (16, 20, 24, 32, 40, 48, 64, 96, 128, 180, 192, 256, 512, 1024):
            render(page, pick(n), n).save(out / "png" / f"{n}.png")
        ico = [render(page, pick(n), n) for n in (16, 20, 24, 32, 40, 48, 64, 128, 256)]
        ico[-1].save(out / "icon.ico", format="ICO", append_images=ico[:-1], sizes=[(i.width, i.height) for i in ico])
        for name, svg in brand.items():
            if "mark" in name:
                render(page, svg, 1024).save(out / "brand" / f"{name}.png")
            else:
                w = float(re.search(r'viewBox="0 0 (\d+)', svg).group(1))
                render(page, svg, round(w * 3.33), 333).save(out / "brand" / f"{name}.png")
        browser.close()


main()
