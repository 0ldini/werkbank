//! Eingebaute Programm-Icons (128 px, aus werk/apps/*/icons). Neu erzeugen: werk/tools/make_icons.py.

pub const ICONS: &[(&str, &[u8])] = &[
    ("cadcraft", include_bytes!("../icons/cadcraft.png")),
    ("codewerk", include_bytes!("../icons/codewerk.png")),
    ("deckcraft", include_bytes!("../icons/deckcraft.png")),
    ("designcraft", include_bytes!("../icons/designcraft.png")),
    ("effectcraft", include_bytes!("../icons/effectcraft.png")),
    ("filmcraft", include_bytes!("../icons/filmcraft.png")),
    ("gridcraft", include_bytes!("../icons/gridcraft.png")),
    ("lightcraft", include_bytes!("../icons/lightcraft.png")),
    ("pdfcraft", include_bytes!("../icons/pdfcraft.png")),
    ("photocraft", include_bytes!("../icons/photocraft.png")),
    ("soundcraft", include_bytes!("../icons/soundcraft.png")),
    ("vectorcraft", include_bytes!("../icons/vectorcraft.png")),
    ("wordcraft", include_bytes!("../icons/wordcraft.png")),
];

pub const WERKBANK: &[u8] = include_bytes!("../icons/werkbank.png");
