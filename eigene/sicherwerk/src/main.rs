#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod aufgabe;
mod auftrag;
mod lauf;
mod motor;
mod plan;
mod pruefsumme;
mod regeln;
mod sperre;
mod sprache;
mod stil;
mod store;
mod thema;
mod zeitplan;
mod zipper;

use auftrag::{Ausloeser, Ergebnis, Taetigkeit, Zeitplan};
use sprache::{t, tf};

/// Befehlszeile (ohne Fenster): `--auftrag "<name>"`, `--anschluss`, `--hilfe`.
enum Befehl {
    Fenster,
    Auftrag(String),
    Anschluss,
    Hilfe,
    Falsch(String),
}

fn befehl() -> Befehl {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(erstes) = args.first() else { return Befehl::Fenster };
    match erstes.as_str() {
        "--auftrag" | "-a" => match args.get(1) {
            Some(n) if !n.trim().is_empty() => Befehl::Auftrag(n.clone()),
            _ => Befehl::Falsch(erstes.clone()),
        },
        s if s.starts_with("--auftrag=") => Befehl::Auftrag(s["--auftrag=".len()..].to_string()),
        "--anschluss" => Befehl::Anschluss,
        "--hilfe" | "--help" | "-h" | "/?" => Befehl::Hilfe,
        // Beim Öffnen über den Dateimanager o. Ä. einfach das Fenster zeigen.
        s if !s.starts_with('-') => Befehl::Fenster,
        s => Befehl::Falsch(s.to_string()),
    }
}

fn hilfe() {
    println!("{}", t("Sicherwerk – Ordner regelmässig sichern."));
    println!();
    println!("  sicherwerk                      {}", t("Fenster öffnen"));
    println!("  sicherwerk --auftrag \"<Name>\"   {}", t("Auftrag ohne Fenster ausführen (für Zeitpläne)"));
    println!("  sicherwerk --anschluss          {}", t("Aufträge «beim Anschliessen» ausführen, deren Ziel erreichbar ist"));
    println!();
    if let Some(p) = store::datei() {
        println!("{}", tf("Daten: {pfad}", &[("pfad", &p.display())]));
    }
}

fn zusammenfassung(p: &auftrag::Protokoll) {
    let groesse = plan::groesse_text(p.bytes);
    println!(
        "{}",
        tf("{name}: {ergebnis} – {neu} neu, {geaendert} geändert, {entfernt} entfernt, {verknuepft} verknüpft, {groesse} kopiert, {fehler} Fehler",
            &[
                ("name", &p.auftrag_name),
                ("ergebnis", &t(p.ergebnis.text())),
                ("neu", &p.neu),
                ("geaendert", &p.geaendert),
                ("entfernt", &p.entfernt),
                ("verknuepft", &p.verknuepft),
                ("groesse", &groesse),
                ("fehler", &p.fehler_anzahl),
            ],
        )
    );
    for f in p.fehler.iter().take(20) {
        eprintln!("  {}: {} {}", t(f.art.text()), f.pfad, f.detail);
    }
}

fn code(e: Ergebnis) -> i32 {
    match e {
        Ergebnis::Ok => 0,
        Ergebnis::MitFehlern | Ergebnis::Abgebrochen => 1,
        Ergebnis::Fehlgeschlagen => 2,
    }
}

fn laden_oder_ende() -> auftrag::Daten {
    match store::laden() {
        Ok((d, _)) => d,
        Err(e) => {
            eprintln!("{} {}", t(e.text), e.detail);
            std::process::exit(2);
        }
    }
}

fn main() -> eframe::Result<()> {
    match befehl() {
        Befehl::Fenster => {}
        Befehl::Hilfe => {
            hilfe();
            return Ok(());
        }
        Befehl::Falsch(a) => {
            eprintln!("{}", tf("Unbekannte Angabe: {a}", &[("a", &a)]));
            hilfe();
            std::process::exit(2);
        }
        Befehl::Auftrag(name) => {
            let d = laden_oder_ende();
            let Some(a) = d.finden(&name).cloned() else {
                eprintln!("{}", tf("Auftrag «{name}» nicht gefunden.", &[("name", &name)]));
                std::process::exit(2);
            };
            let fs_ = motor::Fortschritt::default();
            let (p, fehler) = lauf::ausfuehren(&a, Taetigkeit::Sichern, Ausloeser::Zeitplan, &fs_);
            zusammenfassung(&p);
            if let Some(e) = fehler {
                eprintln!("{} {}", t(e.text), e.detail);
            }
            std::process::exit(code(p.ergebnis));
        }
        Befehl::Anschluss => {
            let d = laden_oder_ende();
            let heute = lauf::heute();
            let mut schlimmster = 0;
            for a in d.auftraege.iter().filter(|a| a.zeitplan == Zeitplan::BeimAnschliessen) {
                if !motor::ziel_da(a) || d.heute_gesichert(a.id, &heute) {
                    continue;
                }
                let fs_ = motor::Fortschritt::default();
                let (p, _) = lauf::ausfuehren(a, Taetigkeit::Sichern, Ausloeser::Anschluss, &fs_);
                zusammenfassung(&p);
                schlimmster = schlimmster.max(code(p.ergebnis));
            }
            std::process::exit(schlimmster);
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_icon(thema::fenster_icon())
            .with_inner_size([1120.0, 740.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("Sicherwerk"),
        ..Default::default()
    };
    eframe::run_native("Sicherwerk", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
