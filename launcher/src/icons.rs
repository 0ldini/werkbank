//! Eingebaute Programm-Icons (128 px, aus werk/apps/*/icons). Neu erzeugen: werk/tools/make_icons.py.

pub const ICONS: &[(&str, &[u8])] = &[
    ("aufnahmewerk", include_bytes!("../icons/aufnahmewerk.png")),
    ("cadcraft", include_bytes!("../icons/cadcraft.png")),
    ("codewerk", include_bytes!("../icons/codewerk.png")),
    ("deckcraft", include_bytes!("../icons/deckcraft.png")),
    ("designcraft", include_bytes!("../icons/designcraft.png")),
    ("diawerk", include_bytes!("../icons/diawerk.png")),
    ("effectcraft", include_bytes!("../icons/effectcraft.png")),
    ("filmcraft", include_bytes!("../icons/filmcraft.png")),
    ("gridcraft", include_bytes!("../icons/gridcraft.png")),
    ("lightcraft", include_bytes!("../icons/lightcraft.png")),
    ("mailwerk", include_bytes!("../icons/mailwerk.png")),
    ("mediathekwerk", include_bytes!("../icons/mediathekwerk.png")),
    ("organiwerk", include_bytes!("../icons/organiwerk.png")),
    ("pdfcraft", include_bytes!("../icons/pdfcraft.png")),
    ("photocraft", include_bytes!("../icons/photocraft.png")),
    ("schutzwerk", include_bytes!("../icons/schutzwerk.png")),
    ("soundcraft", include_bytes!("../icons/soundcraft.png")),
    ("speicherwerk", include_bytes!("../icons/speicherwerk.png")),
    ("umbenennwerk", include_bytes!("../icons/umbenennwerk.png")),
    ("vectorcraft", include_bytes!("../icons/vectorcraft.png")),
    ("wandelwerk", include_bytes!("../icons/wandelwerk.png")),
    ("wordcraft", include_bytes!("../icons/wordcraft.png")),
];

pub const WERKBANK: &[u8] = include_bytes!("../icons/werkbank.png");
