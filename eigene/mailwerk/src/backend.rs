//! Verteilt Befehle an das richtige Konto-Verfahren und sammelt die Ereignisse ein.

use std::sync::{mpsc, Arc, Mutex};

use crate::demo::DemoStore;
use crate::timeutil;
use crate::types::{Account, Cmd, Ev, Kind};

/// Ereignis-Absender, der die Oberfläche nach jedem Ereignis aufweckt.
#[derive(Clone)]
pub struct Out {
    tx: mpsc::Sender<Ev>,
    ctx: egui::Context,
}

impl Out {
    pub fn emit(&self, ev: Ev) {
        let _ = self.tx.send(ev);
        self.ctx.request_repaint();
    }
}

pub struct Backend {
    out: Out,
    rx: mpsc::Receiver<Ev>,
    demo: Arc<Mutex<DemoStore>>,
}

impl Backend {
    pub fn new(ctx: egui::Context) -> Backend {
        let (tx, rx) = mpsc::channel();
        Backend { out: Out { tx, ctx }, rx, demo: Arc::new(Mutex::new(DemoStore::new(timeutil::now_ts()))) }
    }

    pub fn send(&self, acc: &Account, cmd: Cmd) {
        match acc.kind {
            Kind::Demo => {
                let evs = match self.demo.lock() {
                    Ok(mut d) => d.handle(acc.id, cmd),
                    Err(_) => vec![],
                };
                for e in evs {
                    self.out.emit(e);
                }
            }
            Kind::Jmap => crate::jmap::run(acc, cmd, self.out.clone()),
            Kind::Imap => self.send_imap(acc, cmd),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn send_imap(&self, acc: &Account, cmd: Cmd) {
        let acc = acc.clone();
        let out = self.out.clone();
        std::thread::spawn(move || {
            for e in crate::native::run(&acc, cmd) {
                out.emit(e);
            }
        });
    }

    #[cfg(target_arch = "wasm32")]
    fn send_imap(&self, acc: &Account, cmd: Cmd) {
        self.out.emit(Ev::Failed {
            acc: acc.id,
            what: cmd.what(),
            err: crate::sprache::t("Dieses Konto funktioniert nur in der Desktop-Version (Browser dürfen keine IMAP/SMTP-Verbindung öffnen).").into(),
        });
    }

    pub fn poll(&self) -> Vec<Ev> {
        self.rx.try_iter().collect()
    }
}
