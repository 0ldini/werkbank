//! Zwischenablage beobachten (optional): ein Hintergrund-Thread schaut jede Sekunde nach,
//! ob neuer Text mit Links kopiert wurde, und legt ihn für den Linksammler bereit.
//! Was beim Einschalten schon in der Zwischenablage lag, wird nicht übernommen.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::links;
use crate::net::lock;

pub struct Watcher {
    pub enabled: Arc<AtomicBool>,
    /// Neu kopierte Texte, die Links enthalten.
    pub inbox: Arc<Mutex<Vec<String>>>,
}

impl Watcher {
    pub fn start(ctx: egui::Context, enabled: bool) -> Watcher {
        let w = Watcher { enabled: Arc::new(AtomicBool::new(enabled)), inbox: Arc::new(Mutex::new(Vec::new())) };
        let (flag, inbox) = (w.enabled.clone(), w.inbox.clone());
        std::thread::spawn(move || {
            let mut board: Option<arboard::Clipboard> = None;
            let mut last: Option<String> = None;
            loop {
                std::thread::sleep(Duration::from_millis(1000));
                if !flag.load(Ordering::Relaxed) {
                    board = None;
                    last = None;
                    continue;
                }
                if board.is_none() {
                    board = arboard::Clipboard::new().ok();
                }
                let Some(b) = board.as_mut() else { continue };
                // Kein Text (z. B. ein Bild) oder ein Fehler: einfach beim nächsten Mal wieder.
                let Ok(text) = b.get_text() else { continue };
                if last.as_deref() == Some(text.as_str()) {
                    continue;
                }
                let first = last.is_none();
                last = Some(text.clone());
                if first || text.len() > 4 << 20 {
                    continue;
                }
                if !links::extract_links(&text).is_empty() {
                    lock(&inbox).push(text);
                    ctx.request_repaint();
                }
            }
        });
        w
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    pub fn take(&self) -> Vec<String> {
        std::mem::take(&mut *lock(&self.inbox))
    }
}
