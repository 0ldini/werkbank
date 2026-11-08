//! Ende-zu-Ende: Ordnerbaum mit Doppeln anlegen, suchen, nach Regel auswählen und mit dem echten
//! Papierkorb (`trash`) verschieben – unter Linux und Windows (siehe speicherwerk-test.yml).

use crate::doppelt::{self, Auftrag, Fortschritt, Optionen, Regel};
use std::fs;
use std::path::{Path, PathBuf};

fn schreiben(p: &Path, inhalt: &[u8]) {
    if let Some(d) = p.parent() {
        fs::create_dir_all(d).expect("Ordner");
    }
    fs::write(p, inhalt).expect("schreiben");
}

#[test]
fn doppelte_finden_und_in_den_papierkorb() {
    let root = std::env::temp_dir().join(format!("speicherwerk-e2e-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let daten = root.join("Daten");
    // Papierkorb unter Linux in den Testordner legen (gleiches Laufwerk).
    #[cfg(all(unix, not(target_os = "macos")))]
    std::env::set_var("XDG_DATA_HOME", root.join("xdg"));

    let foto: Vec<u8> = (0..300_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
    schreiben(&daten.join("foto.jpg"), &foto);
    schreiben(&daten.join("Ferien 2025/Kopie von foto.jpg"), &foto);
    schreiben(&daten.join("Ferien 2025/Ünterordner äöü/IMG_0001.JPG"), &foto);
    // gleicher Name, anderer Inhalt (gleiche Grösse)
    let mut anders = foto.clone();
    anders[150_000] ^= 0x55;
    schreiben(&daten.join("Archiv/foto.jpg"), &anders);
    // kleine Doppel
    schreiben(&daten.join("notiz.txt"), b"Einkaufen: Brot, Milch");
    schreiben(&daten.join("Archiv/alt/notiz (1).txt"), b"Einkaufen: Brot, Milch");
    // leere Dateien sind keine Doppel
    schreiben(&daten.join("leer.txt"), b"");
    schreiben(&daten.join("Archiv/leer.txt"), b"");
    // harter Link: kein Doppel (NTFS und Linux)
    schreiben(&daten.join("einmalig.bin"), b"nur einmal vorhanden");
    let link = daten.join("Archiv/einmalig-link.bin");
    let hat_link = fs::hard_link(daten.join("einmalig.bin"), &link).is_ok();

    let fp = Fortschritt::new();
    let (gruppen, st) = doppelt::suchen(&daten, &Optionen::default(), &fp).expect("nicht abgebrochen");
    eprintln!("Gruppen: {}, Statistik: {st:?}, harter Link: {hat_link}", gruppen.len());
    assert_eq!(gruppen.len(), 2, "{gruppen:#?}");
    assert_eq!(st.zu_klein, 2);
    let gross = &gruppen[0];
    assert_eq!(gross.eintraege.len(), 3);
    assert_eq!(gross.freigebbar(), 600_000);
    assert!(gross.eintraege.iter().all(|e| !e.pfad.ends_with("Archiv/foto.jpg")));
    assert_eq!(gruppen[1].eintraege.len(), 2);

    let auftraege: Vec<Auftrag> = gruppen.iter().map(|g| Auftrag { gruppe: g.clone(), loeschen: doppelt::auswahl_nach_regel(g, Regel::KuerzesterPfad) }).collect();
    assert_eq!(doppelt::umfang(&auftraege), (3, 600_000 + 22));
    let r = doppelt::loeschen(&auftraege, &fp, &mut |p: &Path| trash::delete(p).map_err(|e| e.to_string()));
    for x in &r {
        eprintln!("Papierkorb: {} → {:?}", x.pfad.display(), x.erfolg);
    }
    assert_eq!(r.len(), 3);
    assert!(r.iter().all(|x| x.erfolg.is_ok()), "{r:?}");
    assert_eq!(r.iter().map(|x| x.bytes).sum::<u64>(), 600_022);

    let bleibt: Vec<PathBuf> = ["foto.jpg", "notiz.txt", "Archiv/foto.jpg", "leer.txt", "Archiv/leer.txt", "einmalig.bin"].iter().map(|p| daten.join(p)).collect();
    for p in &bleibt {
        assert!(p.exists(), "muss bleiben: {}", p.display());
    }
    if hat_link {
        assert!(link.exists(), "harter Link bleibt");
    }
    for p in ["Ferien 2025/Kopie von foto.jpg", "Ferien 2025/Ünterordner äöü/IMG_0001.JPG", "Archiv/alt/notiz (1).txt"] {
        assert!(!daten.join(p).exists(), "noch da: {p}");
    }
    assert_eq!(fs::read(daten.join("foto.jpg")).expect("lesen"), foto);

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let korb = root.join("xdg/Trash/files");
        let n = fs::read_dir(&korb).map(|r| r.count()).unwrap_or(0);
        assert_eq!(n, 3, "Dateien im Papierkorb {}", korb.display());
    }

    // nochmals suchen: keine Doppel mehr
    let (gruppen, _) = doppelt::suchen(&daten, &Optionen::default(), &fp).expect("ok");
    assert!(gruppen.is_empty(), "{gruppen:#?}");
    let _ = fs::remove_dir_all(&root);
}
