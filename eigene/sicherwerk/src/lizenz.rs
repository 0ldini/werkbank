//! Freischaltung der Werkbank: Ohne gültige Lizenz zeigt das Programm nur einen Hinweis.
//!
//! Die Lizenz holt der Werkbank-Launcher beim Freischalten vom Server (ein gemeinsamer Code für
//! alle) und legt sie als `lizenz.txt` in den Einstellungsordner der Werkbank (neben
//! `sprache.txt`): Zeile 1 `werkbank|1|<Gerät>|<Ablauf>`, Zeile 2 die Ed25519-Unterschrift (hex).
//! Geprüft wird hier ohne Netz: Unterschrift, Ablaufdatum und Gerät (`geraet.txt`).
//!
//! Beim Start in `main` vor `eframe::run_native`:
//! `if !lizenz::ok() { return lizenz::sperre("Name"); }`
//!
//! Diese Datei ist in allen eigenen Programmen gleich (Vorlage: eigene/lizenz.rs).

#![allow(dead_code)]

use std::path::PathBuf;

/// Öffentlicher Schlüssel des Lizenzservers (der geheime liegt nur auf dem Server).
pub const PUBLIC_KEY_HEX: &str = "4cb8422c7e8266fe45c99fb20e6336ca5d282a5cc2251fca9f95f9e4a6a90493";

fn hex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// Prüft den Text einer Lizenz; liefert (Gerät, Ablauf in Unix-Sekunden).
pub fn pruefen(text: &str, jetzt: u64) -> Option<(String, u64)> {
    let mut lines = text.lines();
    let payload = lines.next()?.trim();
    let sig: [u8; 64] = hex(lines.next()?)?.try_into().ok()?;
    let key: [u8; 32] = hex(PUBLIC_KEY_HEX)?.try_into().ok()?;
    let vk = ed25519_dalek::VerifyingKey::from_bytes(&key).ok()?;
    vk.verify_strict(payload.as_bytes(), &ed25519_dalek::Signature::from_bytes(&sig)).ok()?;
    let parts: Vec<&str> = payload.split('|').collect();
    if parts.len() != 4 || parts[0] != "werkbank" || parts[1] != "1" {
        return None;
    }
    let ablauf: u64 = parts[3].parse().ok()?;
    (ablauf > jetzt).then(|| (parts[2].to_string(), ablauf))
}

fn ordner() -> Option<PathBuf> {
    crate::sprache::werkbank_file()?.parent().map(|p| p.to_path_buf())
}

fn jetzt() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// true, wenn diese Werkbank freigeschaltet ist (im Browser immer).
pub fn ok() -> bool {
    if cfg!(target_arch = "wasm32") {
        return true;
    }
    let Some(dir) = ordner() else { return false };
    let Ok(text) = std::fs::read_to_string(dir.join("lizenz.txt")) else { return false };
    let geraet = std::fs::read_to_string(dir.join("geraet.txt")).unwrap_or_default();
    matches!(pruefen(&text, jetzt()), Some((g, _)) if g == geraet.trim())
}

/// Pfad des Launchers, den er beim Start ablegt.
fn launcher() -> Option<PathBuf> {
    let p = PathBuf::from(std::fs::read_to_string(ordner()?.join("launcher.txt")).ok()?.trim());
    p.is_file().then_some(p)
}

/// Kleines Fenster mit dem Hinweis, dass die Werkbank erst freigeschaltet werden muss.
#[cfg(not(target_arch = "wasm32"))]
pub fn sperre(name: &str) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([460.0, 250.0]).with_resizable(false).with_title(name),
        ..Default::default()
    };
    let name = name.to_string();
    eframe::run_native(
        &name.clone(),
        options,
        Box::new(move |cc| {
            crate::thema::apply(&cc.egui_ctx);
            Ok(Box::new(Sperre { name }))
        }),
    )
}

#[cfg(not(target_arch = "wasm32"))]
struct Sperre {
    name: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl eframe::App for Sperre {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let en = crate::sprache::is_en();
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(18.0);
            ui.vertical_centered(|ui| {
                ui.heading(if en { "Werkbank is not activated" } else { "Die Werkbank ist nicht freigeschaltet" });
                ui.add_space(8.0);
                let text = if en {
                    format!("{} is part of the Werkbank. Open the Werkbank launcher and enter your code; it needs the internet once.", self.name)
                } else {
                    format!("{} gehört zur Werkbank. Öffne den Werkbank-Launcher und gib dort deinen Code ein; dafür braucht es einmal Internet.", self.name)
                };
                ui.label(text);
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if let Some(l) = launcher() {
                        if ui.button(if en { "Open Werkbank" } else { "Werkbank öffnen" }).clicked() {
                            let _ = std::process::Command::new(l).spawn();
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                    if ui.button(if en { "Close" } else { "Schliessen" }).clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
        });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn unterschrift() {
        let ok = "werkbank|1|00112233445566778899aabbccddeeff|4102444800\ndc1a1d96a8b5ce76f1c8c4dafbd9a9713e70701efc5f8c559f9a18adcf4f7e4e951ce97d2754eaa72f027238dd4bbed05601d283d71ec9dc53a609dd609d0d0b";
        assert_eq!(super::pruefen(ok, 1_800_000_000).map(|x| x.0), Some("00112233445566778899aabbccddeeff".to_string()));
        // abgelaufen
        assert!(super::pruefen(ok, 4_102_444_801).is_none());
        // verändert
        assert!(super::pruefen(&ok.replace("4102444800", "4102444900"), 0).is_none());
        assert!(super::pruefen("unsinn", 0).is_none());
    }
}
