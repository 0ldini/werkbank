//! Der Motor hinter der Oberfläche: eine Browser-Verbindung (Ordner anzeigen, anlegen, löschen …)
//! und mehrere Übertragungs-Verbindungen, die eine gemeinsame Warteschlange abarbeiten.
//! Alles läuft in eigenen Threads; die Oberfläche spricht nur über Kanäle und Zähler mit ihm.
//! Kein egui hier – der Motor wird mit einem echten Testserver geprüft.

#![allow(dead_code)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::model::{is_part_name, numbered_name, part_name, rjoin, rname, rparent, Entry, Proto, RErr, Res, Site};
use crate::remote::{self, Remote, Tick};
use crate::sprache::{t, tf};
use crate::store::{Overwrite, Trust};

/// Weckt die Oberfläche, damit sie neu zeichnet.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

// ---------------------------------------------------------------- Befehle und Meldungen

pub enum Cmd {
    List(String),
    Mkdir(String),
    Rename { from: String, to: String },
    /// (Pfad, ist Ordner) – Ordner samt Inhalt.
    Delete(Vec<(String, bool)>),
    Chmod { items: Vec<(String, bool)>, mode: u32, recursive: bool },
    /// (Serverpfad, ist Ordner, Grösse) → in den lokalen Ordner.
    PlanDownload { items: Vec<(String, bool, u64)>, local_dir: PathBuf },
    /// Lokale Dateien/Ordner → in den Serverordner.
    PlanUpload { items: Vec<PathBuf>, remote_dir: String },
    Stop,
}

pub enum Ev {
    Connected { cwd: String },
    ConnectFailed(RErr),
    Listing { path: String, entries: Vec<Entry> },
    ListFailed { path: String, err: RErr },
    /// Eine Änderung am Server ist erledigt – Liste neu laden.
    Changed(String),
    Failed { what: String, err: RErr },
    /// Neue Aufträge sind in der Warteschlange.
    Queued(usize),
    /// Verbindung verloren und nicht wiederherstellbar.
    Lost(String),
    Busy(String),
}

// ---------------------------------------------------------------- Warteschlange

pub const WAIT: u8 = 0;
pub const RUN: u8 = 1;
pub const DONE: u8 = 2;
pub const FAIL: u8 = 3;
pub const CANCELLED: u8 = 4;
pub const SKIPPED: u8 = 5;

pub struct Job {
    pub id: u64,
    pub up: bool,
    pub remote: String,
    pub local: PathBuf,
    /// Hochladen: Zielordner, ab dem fehlende Unterordner angelegt werden.
    pub root_remote: String,
    pub total: AtomicU64,
    pub done: AtomicU64,
    pub status: AtomicU8,
    pub error: Mutex<String>,
    pub cancel: AtomicBool,
    /// «Abbrechen und verwerfen»: die Teil-Datei wird gelöscht.
    pub discard: AtomicBool,
    /// Pfad der Teil-Datei, in die gerade übertragen wird (lokal oder auf dem Server).
    pub part: Mutex<String>,
    /// Ab diesem Byte wurde zuletzt fortgesetzt (0 = von vorn).
    pub resumed_from: AtomicU64,
    /// Nur für Tests: nach so vielen Bytes anhalten, als hätte der Benutzer abgebrochen (0 = aus).
    pub stop_at: AtomicU64,
}

impl Job {
    pub fn name(&self) -> String {
        if self.up {
            self.local.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
        } else {
            crate::model::rname(&self.remote)
        }
    }

    pub fn status(&self) -> u8 {
        self.status.load(Ordering::Relaxed)
    }

    pub fn fraction(&self) -> f32 {
        let t = self.total.load(Ordering::Relaxed);
        if t == 0 {
            return if self.status() == DONE { 1.0 } else { 0.0 };
        }
        (self.done.load(Ordering::Relaxed) as f64 / t as f64).clamp(0.0, 1.0) as f32
    }

    pub fn error_text(&self) -> String {
        self.error.lock().map(|e| e.clone()).unwrap_or_default()
    }
}

pub struct Queue {
    pub jobs: Mutex<Vec<Arc<Job>>>,
    cv: Condvar,
    pub paused: AtomicBool,
    pub stop: AtomicBool,
    next_id: AtomicU64,
    overwrite: AtomicU8,
    wake: Wake,
}

fn ow_to_u8(o: Overwrite) -> u8 {
    match o {
        Overwrite::Overwrite => 0,
        Overwrite::Rename => 1,
        Overwrite::Skip => 2,
    }
}

impl Queue {
    pub fn new(wake: Wake, overwrite: Overwrite) -> Arc<Queue> {
        Arc::new(Queue {
            jobs: Mutex::new(Vec::new()),
            cv: Condvar::new(),
            paused: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            next_id: AtomicU64::new(1),
            overwrite: AtomicU8::new(ow_to_u8(overwrite)),
            wake,
        })
    }

    pub fn overwrite(&self) -> Overwrite {
        match self.overwrite.load(Ordering::Relaxed) {
            1 => Overwrite::Rename,
            2 => Overwrite::Skip,
            _ => Overwrite::Overwrite,
        }
    }

    pub fn set_overwrite(&self, o: Overwrite) {
        self.overwrite.store(ow_to_u8(o), Ordering::Relaxed);
    }

    pub fn add(&self, up: bool, remote: String, local: PathBuf, root_remote: String, total: u64) {
        let job = Arc::new(Job {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            up,
            remote,
            local,
            root_remote,
            total: AtomicU64::new(total),
            done: AtomicU64::new(0),
            status: AtomicU8::new(WAIT),
            error: Mutex::new(String::new()),
            cancel: AtomicBool::new(false),
            discard: AtomicBool::new(false),
            part: Mutex::new(String::new()),
            resumed_from: AtomicU64::new(0),
            stop_at: AtomicU64::new(0),
        });
        if let Ok(mut j) = self.jobs.lock() {
            j.push(job);
        }
        self.cv.notify_all();
    }

    pub fn snapshot(&self) -> Vec<Arc<Job>> {
        self.jobs.lock().map(|j| j.clone()).unwrap_or_default()
    }

    pub fn set_paused(&self, p: bool) {
        self.paused.store(p, Ordering::Relaxed);
        self.cv.notify_all();
    }

    /// Einen Auftrag abbrechen (läuft er, wird er zwischen zwei Datenblöcken gestoppt).
    pub fn cancel(&self, id: u64) {
        for j in self.snapshot() {
            if j.id == id {
                j.cancel.store(true, Ordering::Relaxed);
                let _ = j.status.compare_exchange(WAIT, CANCELLED, Ordering::Relaxed, Ordering::Relaxed);
            }
        }
        (self.wake)();
    }

    /// «Abbrechen und verwerfen»: abbrechen und die Teil-Datei löschen (lokal oder auf dem
    /// Server). Ein Übertragungs-Thread erledigt das Löschen.
    pub fn discard(&self, id: u64) {
        for j in self.snapshot() {
            if j.id == id && matches!(j.status(), WAIT | RUN | FAIL | CANCELLED) {
                j.discard.store(true, Ordering::Relaxed);
                j.cancel.store(true, Ordering::Relaxed);
                // Nicht laufende Aufträge einem Thread zum Aufräumen geben
                let _ = j.status.compare_exchange(FAIL, WAIT, Ordering::Relaxed, Ordering::Relaxed);
                let _ = j.status.compare_exchange(CANCELLED, WAIT, Ordering::Relaxed, Ordering::Relaxed);
            }
        }
        self.cv.notify_all();
        (self.wake)();
    }

    pub fn cancel_all(&self) {
        for j in self.snapshot() {
            j.cancel.store(true, Ordering::Relaxed);
            let _ = j.status.compare_exchange(WAIT, CANCELLED, Ordering::Relaxed, Ordering::Relaxed);
        }
        (self.wake)();
    }

    /// Fehlgeschlagene oder abgebrochene Aufträge erneut einreihen.
    pub fn retry(&self, id: Option<u64>) {
        for j in self.snapshot() {
            if (id.is_none() || id == Some(j.id)) && matches!(j.status(), FAIL | CANCELLED) && !j.discard.load(Ordering::Relaxed) {
                j.cancel.store(false, Ordering::Relaxed);
                if let Ok(mut e) = j.error.lock() {
                    e.clear();
                }
                j.status.store(WAIT, Ordering::Relaxed);
            }
        }
        self.cv.notify_all();
        (self.wake)();
    }

    /// Erledigte, übersprungene und abgebrochene Aufträge aus der Liste nehmen.
    pub fn clear_finished(&self) {
        if let Ok(mut j) = self.jobs.lock() {
            j.retain(|x| matches!(x.status(), WAIT | RUN | FAIL));
        }
        (self.wake)();
    }

    pub fn remove(&self, id: u64) {
        if let Ok(mut j) = self.jobs.lock() {
            j.retain(|x| x.id != id || x.status() == RUN);
        }
        (self.wake)();
    }

    pub fn counts(&self) -> (usize, usize, usize, usize) {
        let mut c = (0, 0, 0, 0); // wartend, laufend, fertig, Fehler
        for j in self.snapshot() {
            match j.status() {
                WAIT => c.0 += 1,
                RUN => c.1 += 1,
                DONE | SKIPPED => c.2 += 1,
                FAIL => c.3 += 1,
                _ => {}
            }
        }
        c
    }

    fn next_job(&self) -> Option<Arc<Job>> {
        let mut guard = self.jobs.lock().ok()?;
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return None;
            }
            if !self.paused.load(Ordering::Relaxed) {
                for j in guard.iter() {
                    if j.status.compare_exchange(WAIT, RUN, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
                        return Some(j.clone());
                    }
                }
            }
            guard = self.cv.wait_timeout(guard, Duration::from_millis(500)).ok()?.0;
        }
    }
}

// ---------------------------------------------------------------- Hilfen

/// Name vom Server → sicherer Dateiname für die lokale Platte (kein Ausbruch aus dem Zielordner).
pub fn safe_local_name(name: &str) -> Option<String> {
    let n = name.trim_end_matches(['/', '\\']);
    if n.is_empty() || n == "." || n == ".." {
        return None;
    }
    let cleaned: String = n
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' => '_',
            ':' | '*' | '?' | '"' | '<' | '>' | '|' if cfg!(windows) => '_',
            c if (c as u32) < 32 && cfg!(windows) => '_',
            c => c,
        })
        .collect();
    Some(cleaned)
}

fn trust_kind(p: Proto) -> &'static str {
    if p == Proto::Sftp {
        "ssh"
    } else {
        "tls"
    }
}

fn connect_site(site: &Site, trust: &Arc<Mutex<Trust>>, log: &Sender<String>) -> Res<Box<dyn Remote>> {
    let fps = trust.lock().unwrap_or_else(|e| e.into_inner()).for_host(trust_kind(site.proto), &site.hostport());
    remote::connect(site, &fps, Some(log.clone()))
}

// ---------------------------------------------------------------- Sitzung

pub struct Session {
    pub site: Site,
    pub tx: Sender<Cmd>,
    pub rx: Receiver<Ev>,
    pub queue: Arc<Queue>,
    pub log_rx: Receiver<String>,
    threads: Vec<JoinHandle<()>>,
}

impl Session {
    pub fn start(site: Site, trust: Arc<Mutex<Trust>>, parallel: usize, overwrite: Overwrite, wake: Wake) -> Session {
        let (cmd_tx, cmd_rx) = channel::<Cmd>();
        let (ev_tx, ev_rx) = channel::<Ev>();
        let (log_tx, log_rx) = channel::<String>();
        let queue = Queue::new(wake.clone(), overwrite);
        let mut threads = Vec::new();

        {
            let (site, trust, wake, log_tx, queue) = (site.clone(), trust.clone(), wake.clone(), log_tx.clone(), queue.clone());
            let (fail_tx, fail_wake) = (ev_tx.clone(), wake.clone());
            let spawned = thread::Builder::new().name("ftpwerk-browser".into()).spawn(move || browser_loop(site, trust, cmd_rx, ev_tx, log_tx, wake, queue));
            match spawned {
                Ok(h) => threads.push(h),
                Err(e) => {
                    let _ = fail_tx.send(Ev::ConnectFailed(RErr::Msg(e.to_string())));
                    fail_wake();
                }
            }
        }
        for i in 0..parallel.clamp(1, 6) {
            let (site, trust, log_tx, queue, wake) = (site.clone(), trust.clone(), log_tx.clone(), queue.clone(), wake.clone());
            // Fehlt ein Übertragungs-Thread, arbeiten die übrigen die Warteschlange ab.
            if let Ok(h) = thread::Builder::new().name(format!("ftpwerk-xfer-{i}")).spawn(move || worker_loop(site, trust, queue, log_tx, wake)) {
                threads.push(h);
            }
        }
        Session { site, tx: cmd_tx, rx: ev_rx, queue, log_rx, threads }
    }

    pub fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
    }

    /// Beendet alle Threads (laufende Übertragungen werden abgebrochen). Wartet nicht darauf.
    pub fn stop(&self) {
        self.queue.stop.store(true, Ordering::Relaxed);
        self.queue.cancel_all();
        let _ = self.tx.send(Cmd::Stop);
        self.queue.cv.notify_all();
    }

    /// Wartet, bis alle Threads beendet sind (für Tests).
    pub fn join(mut self) {
        for h in std::mem::take(&mut self.threads) {
            let _ = h.join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------- Browser-Verbindung

struct Browser {
    site: Site,
    trust: Arc<Mutex<Trust>>,
    conn: Option<Box<dyn Remote>>,
    log: Sender<String>,
}

impl Browser {
    fn reconnect(&mut self) -> Res<()> {
        if let Some(mut c) = self.conn.take() {
            c.close();
        }
        self.conn = Some(connect_site(&self.site, &self.trust, &self.log)?);
        Ok(())
    }

    /// Führt eine Aktion aus; bricht die Verbindung ab, wird einmal neu verbunden und es wird wiederholt.
    fn run<T>(&mut self, op: &mut dyn FnMut(&mut dyn Remote) -> Res<T>) -> Res<T> {
        let mut tries = 0;
        loop {
            if self.conn.is_none() {
                self.reconnect()?;
            }
            let r = match self.conn.as_mut() {
                Some(c) => op(c.as_mut()),
                None => return Err(RErr::Msg(t("Nicht verbunden.").to_string())),
            };
            match r {
                Err(e) if e.is_disconnect() && tries < 1 => {
                    tries += 1;
                    let _ = self.log.send(t("Verbindung unterbrochen – verbinde neu …").to_string());
                    self.conn = None;
                }
                other => return other,
            }
        }
    }
}

fn browser_loop(site: Site, trust: Arc<Mutex<Trust>>, rx: Receiver<Cmd>, tx: Sender<Ev>, log: Sender<String>, wake: Wake, queue: Arc<Queue>) {
    let send = |e: Ev| {
        let _ = tx.send(e);
        wake();
    };
    let mut b = Browser { site: site.clone(), trust, conn: None, log };
    let cwd = match b.reconnect().and_then(|_| b.run(&mut |c| c.pwd())) {
        Ok(p) => p,
        Err(e) => {
            send(Ev::ConnectFailed(e));
            return;
        }
    };
    send(Ev::Connected { cwd });

    loop {
        let cmd = match rx.recv_timeout(Duration::from_secs(25)) {
            Ok(c) => c,
            Err(RecvTimeoutError::Timeout) => {
                // Verbindung wachhalten; geht es nicht, beim nächsten Befehl neu verbinden
                if let Some(c) = b.conn.as_mut() {
                    if c.keepalive().is_err() {
                        b.conn = None;
                    }
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        match cmd {
            Cmd::Stop => break,
            Cmd::List(path) => match b.run(&mut |c| c.list(&path)) {
                Ok(entries) => send(Ev::Listing { path, entries }),
                Err(err) => {
                    if err.is_disconnect() {
                        send(Ev::Lost(err.to_string()));
                    }
                    send(Ev::ListFailed { path, err })
                }
            },
            Cmd::Mkdir(path) => match b.run(&mut |c| c.mkdir(&path)) {
                Ok(()) => send(Ev::Changed(t("Ordner angelegt.").to_string())),
                Err(err) => send(Ev::Failed { what: t("Ordner anlegen").to_string(), err }),
            },
            Cmd::Rename { from, to } => match b.run(&mut |c| c.rename(&from, &to)) {
                Ok(()) => send(Ev::Changed(t("Umbenannt.").to_string())),
                Err(err) => send(Ev::Failed { what: t("Umbenennen").to_string(), err }),
            },
            Cmd::Delete(items) => {
                send(Ev::Busy(t("Lösche …").to_string()));
                let mut failed = 0;
                for (path, is_dir) in items {
                    let r = b.run(&mut |c| delete_tree(c, &path, is_dir, 0));
                    if let Err(err) = r {
                        failed += 1;
                        send(Ev::Failed { what: tf("Löschen von «{p}»", &[("p", &path)]), err });
                    }
                }
                send(Ev::Changed(if failed == 0 { t("Gelöscht.").to_string() } else { t("Nicht alles konnte gelöscht werden.").to_string() }));
            }
            Cmd::Chmod { items, mode, recursive } => {
                for (path, is_dir) in items {
                    let r = b.run(&mut |c| chmod_tree(c, &path, is_dir, mode, recursive, 0));
                    if let Err(err) = r {
                        send(Ev::Failed { what: tf("Rechte von «{p}» ändern", &[("p", &path)]), err });
                    }
                }
                send(Ev::Changed(t("Rechte geändert.").to_string()));
            }
            Cmd::PlanDownload { items, local_dir } => {
                send(Ev::Busy(t("Lese Ordner …").to_string()));
                let mut n = 0usize;
                for (path, is_dir, size) in items {
                    let Some(name) = safe_local_name(&crate::model::rname(&path)) else { continue };
                    let target = local_dir.join(&name);
                    if is_dir {
                        let r = b.run(&mut |c| plan_download_dir(c, &path, &target, &queue, 0));
                        match r {
                            Ok(k) => n += k,
                            Err(err) => send(Ev::Failed { what: tf("Ordner «{p}» lesen", &[("p", &path)]), err }),
                        }
                    } else {
                        queue.add(false, path.clone(), target, String::new(), size);
                        n += 1;
                    }
                }
                send(Ev::Queued(n));
            }
            Cmd::PlanUpload { items, remote_dir } => {
                send(Ev::Busy(t("Lese Ordner …").to_string()));
                let mut n = 0usize;
                for local in items {
                    let Some(name) = local.file_name().map(|n| n.to_string_lossy().to_string()) else { continue };
                    let target = rjoin(&remote_dir, &name);
                    if local.is_dir() {
                        let r = b.run(&mut |c| plan_upload_dir(c, &local, &target, &remote_dir, &queue, 0));
                        match r {
                            Ok(k) => n += k,
                            Err(err) => send(Ev::Failed { what: tf("Ordner «{p}» hochladen", &[("p", &name)]), err }),
                        }
                    } else {
                        let size = std::fs::metadata(&local).map(|m| m.len()).unwrap_or(0);
                        queue.add(true, target, local.clone(), remote_dir.clone(), size);
                        n += 1;
                    }
                }
                send(Ev::Queued(n));
                send(Ev::Changed(String::new()));
            }
        }
    }
    if let Some(mut c) = b.conn.take() {
        c.close();
    }
}

const MAX_DEPTH: usize = 40;

fn delete_tree(c: &mut dyn Remote, path: &str, is_dir: bool, depth: usize) -> Res<()> {
    if !is_dir {
        return c.remove_file(path);
    }
    if depth > MAX_DEPTH {
        return Err(RErr::Msg(t("Ordner zu tief verschachtelt.").to_string()));
    }
    for e in c.list(path)? {
        let p = rjoin(path, &e.name);
        // Verknüpfungen werden nur entfernt, nie «hineingelöscht»
        let r = if e.is_dir && !e.is_link { delete_tree(c, &p, true, depth + 1) } else { c.remove_file(&p) };
        r?;
    }
    c.remove_dir(path)
}

fn chmod_tree(c: &mut dyn Remote, path: &str, is_dir: bool, mode: u32, recursive: bool, depth: usize) -> Res<()> {
    c.chmod(path, mode)?;
    if is_dir && recursive && depth < MAX_DEPTH {
        for e in c.list(path)? {
            if e.is_link {
                continue;
            }
            chmod_tree(c, &rjoin(path, &e.name), e.is_dir, mode, true, depth + 1)?;
        }
    }
    Ok(())
}

fn plan_download_dir(c: &mut dyn Remote, remote: &str, local: &Path, q: &Queue, depth: usize) -> Res<usize> {
    if depth > MAX_DEPTH {
        return Ok(0);
    }
    std::fs::create_dir_all(local)?;
    let mut n = 0;
    for e in c.list(remote)? {
        // eigene Teil-Dateien (unfertige Übertragungen) nicht mitnehmen
        if is_part_name(&e.name) {
            continue;
        }
        let Some(name) = safe_local_name(&e.name) else { continue };
        let rp = rjoin(remote, &e.name);
        let lp = local.join(&name);
        if e.is_dir {
            if e.is_link {
                continue; // Verknüpfungen auf Ordner nicht verfolgen (Schleifen)
            }
            n += plan_download_dir(c, &rp, &lp, q, depth + 1)?;
        } else {
            q.add(false, rp, lp, String::new(), e.size);
            n += 1;
        }
    }
    Ok(n)
}

fn plan_upload_dir(c: &mut dyn Remote, local: &Path, remote: &str, root: &str, q: &Queue, depth: usize) -> Res<usize> {
    if depth > MAX_DEPTH {
        return Ok(0);
    }
    let _ = c.mkdir(remote); // gibt es ihn schon, ist das in Ordnung
    let mut n = 0;
    let mut entries: Vec<_> = std::fs::read_dir(local)?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        if is_part_name(&name) {
            continue;
        }
        let lp = e.path();
        let rp = rjoin(remote, &name);
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            n += plan_upload_dir(c, &lp, &rp, root, q, depth + 1)?;
        } else if ft.is_file() || (ft.is_symlink() && lp.is_file()) {
            let size = std::fs::metadata(&lp).map(|m| m.len()).unwrap_or(0);
            q.add(true, rp, lp, root.to_string(), size);
            n += 1;
        }
    }
    Ok(n)
}

// ---------------------------------------------------------------- Übertragungs-Verbindungen

fn worker_loop(site: Site, trust: Arc<Mutex<Trust>>, queue: Arc<Queue>, log: Sender<String>, wake: Wake) {
    let mut conn: Option<Box<dyn Remote>> = None;
    let mut made: HashSet<String> = HashSet::new();
    while let Some(job) = queue.next_job() {
        wake();
        let result = run_job(&job, &site, &trust, &queue, &log, &mut conn, &mut made);
        match result {
            Ok(skipped) => job.status.store(if skipped { SKIPPED } else { DONE }, Ordering::Relaxed),
            Err(RErr::Cancelled) => {
                // Nach einem Abbruch ist der Datenstrom in unklarem Zustand: Verbindung neu aufbauen
                if let Some(mut c) = conn.take() {
                    c.close();
                }
                if job.discard.load(Ordering::Relaxed) && !queue.stop.load(Ordering::Relaxed) {
                    discard_part(&job, &mut conn, &|| connect_site(&site, &trust, &log), &log);
                }
                job.status.store(CANCELLED, Ordering::Relaxed);
            }
            Err(e) => {
                *job.error.lock().unwrap_or_else(|p| p.into_inner()) = e.to_string();
                let _ = log.send(tf("Fehler bei «{n}»: {e}", &[("n", &job.name()), ("e", &e)]));
                job.status.store(FAIL, Ordering::Relaxed);
                // Nach einem Fehler mitten in einer Übertragung lieber frisch verbinden.
                if let Some(mut c) = conn.take() {
                    if !e.is_disconnect() {
                        c.close();
                    }
                }
            }
        }
        wake();
    }
    if let Some(mut c) = conn.take() {
        c.close();
    }
}

/// Rückgabe: `true` = übersprungen.
fn run_job(
    job: &Arc<Job>,
    site: &Site,
    trust: &Arc<Mutex<Trust>>,
    queue: &Arc<Queue>,
    log: &Sender<String>,
    conn: &mut Option<Box<dyn Remote>>,
    made: &mut HashSet<String>,
) -> Res<bool> {
    let mut attempt = 0;
    loop {
        if job.cancel.load(Ordering::Relaxed) || queue.stop.load(Ordering::Relaxed) {
            return Err(RErr::Cancelled);
        }
        if conn.is_none() {
            *conn = Some(connect_site(site, trust, log)?);
            made.clear();
        }
        let ow = queue.overwrite();
        let c = conn.as_mut().map(|c| c.as_mut()).ok_or_else(|| RErr::Msg(t("Nicht verbunden.").to_string()))?;
        job.done.store(0, Ordering::Relaxed);
        let outcome = transfer(c, job, ow, made, &|| queue.stop.load(Ordering::Relaxed));

        match outcome {
            Err(e) if e.is_disconnect() && attempt < 3 => {
                attempt += 1;
                let _ = log.send(tf("Verbindung unterbrochen bei «{n}» – neuer Versuch {a}/3 …", &[("n", &job.name()), ("a", &attempt)]));
                if let Some(mut c) = conn.take() {
                    c.close();
                }
                // beim nächsten Versuch möglichst dort weitermachen, wo es abgebrochen ist
                thread::sleep(Duration::from_millis(800 * attempt as u64));
            }
            other => return other,
        }
    }
}

// ---------------------------------------------------------------- Teil-Dateien und Ziel

/// Was mit einer Übertragung geschieht. Grundsatz: geschrieben wird immer in die eigene
/// Teil-Datei «<ziel>.ftpwerk-teil»; an eine vorhandene (fremde) Zieldatei wird nie angehängt.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Plan {
    /// Ziel gibt es schon, Einstellung «Überspringen».
    Skip,
    /// Ziel gibt es schon, Einstellung «Umbenennen»: neuen freien Namen suchen.
    NewName,
    /// In die Teil-Datei übertragen; `resume`: die eigene Teil-Datei fortsetzen.
    /// `replace`: am Ende eine vorhandene Zieldatei ersetzen (vom Benutzer so gewählt).
    Go { resume: bool, replace: bool },
}

/// Entscheidung aus: gibt es das Ziel, gibt es eine eigene Teil-Datei (Grösse), Einstellung.
pub fn plan(target_exists: bool, own_part: Option<u64>, ow: Overwrite) -> Plan {
    let resume = own_part.is_some_and(|n| n > 0);
    if !target_exists {
        return Plan::Go { resume, replace: false };
    }
    match ow {
        Overwrite::Skip => Plan::Skip,
        Overwrite::Rename => Plan::NewName,
        Overwrite::Overwrite => Plan::Go { resume, replace: true },
    }
}

/// Vor dem Umbenennen: stimmt die Grösse der Teil-Datei mit der erwarteten überein?
/// Unbekannte Grössen (Server ohne SIZE) gelten als passend.
pub fn check_complete(part_size: Option<u64>, expected: Option<u64>) -> Res<()> {
    match (part_size, expected) {
        (Some(have), Some(want)) if have != want => Err(RErr::Msg(tf("Datei unvollständig: {got} von {tot} Bytes", &[("got", &have), ("tot", &want)]))),
        _ => Ok(()),
    }
}

/// Lokale Datei «name (n).ext» suchen, die es noch nicht gibt.
fn free_local(target: &Path) -> Res<PathBuf> {
    let parent = target.parent().map(Path::to_path_buf).unwrap_or_default();
    let name = target.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    for n in 2..1000 {
        let cand = parent.join(numbered_name(&name, n));
        if !cand.exists() {
            return Ok(cand);
        }
    }
    Err(RErr::Msg(t("Kein freier Dateiname gefunden.").to_string()))
}

/// Datei «name (n).ext» auf dem Server suchen, die es noch nicht gibt.
fn free_remote(c: &mut dyn Remote, target: &str) -> Res<String> {
    let (parent, name) = (rparent(target), rname(target));
    for n in 2..1000 {
        let cand = rjoin(&parent, &numbered_name(&name, n));
        if !c.exists(&cand)? {
            return Ok(cand);
        }
    }
    Err(RErr::Msg(t("Kein freier Dateiname gefunden.").to_string()))
}

fn local_part(target: &Path) -> PathBuf {
    let mut s = target.as_os_str().to_os_string();
    s.push(crate::model::PART_SUFFIX);
    PathBuf::from(s)
}

/// Eine Übertragung: in die Teil-Datei, Grösse prüfen, dann auf den Zielnamen umbenennen.
/// Rückgabe `true` = übersprungen.
fn transfer(c: &mut dyn Remote, job: &Job, ow: Overwrite, made: &mut HashSet<String>, stop: &dyn Fn() -> bool) -> Res<bool> {
    let mut prog = |tick: Tick| -> bool {
        match tick {
            Tick::Start { offset, total } => {
                if total > 0 {
                    job.total.store(total, Ordering::Relaxed);
                }
                job.done.store(offset, Ordering::Relaxed);
                job.resumed_from.store(offset, Ordering::Relaxed);
            }
            Tick::Bytes(n) => {
                let done = job.done.fetch_add(n, Ordering::Relaxed).saturating_add(n);
                let at = job.stop_at.load(Ordering::Relaxed);
                if at > 0 && done >= at {
                    job.stop_at.store(0, Ordering::Relaxed);
                    job.cancel.store(true, Ordering::Relaxed);
                }
            }
        }
        !(job.cancel.load(Ordering::Relaxed) || stop())
    };
    if job.up {
        ensure_remote_dirs(c, &job.root_remote, &rparent(&job.remote), made)?;
        let mut target = job.remote.clone();
        let mut p = plan(c.exists(&target)?, c.size(&part_name(&target))?, ow);
        if p == Plan::NewName {
            target = free_remote(c, &target)?;
            p = plan(false, c.size(&part_name(&target))?, ow);
        }
        let Plan::Go { resume, replace } = p else { return Ok(true) };
        let part = part_name(&target);
        *job.part.lock().unwrap_or_else(|e| e.into_inner()) = part.clone();
        let total = std::fs::metadata(&job.local)?.len();
        // Die alte Zieldatei (z. B. .htaccess auf dem Webspace) bleibt bis zum Umbenennen unberührt.
        c.upload(&job.local, &part, resume, &mut prog)?;
        check_complete(c.size(&part)?, Some(total))?;
        if let Err(e) = c.rename(&part, &target) {
            // SFTP (Version 3) und manche FTP-Server benennen nicht auf ein vorhandenes Ziel um:
            // dann – nur wenn «Ersetzen» gewählt ist – das alte Ziel löschen und erneut umbenennen.
            if replace && c.exists(&target)? {
                c.remove_file(&target)?;
                c.rename(&part, &target)?;
            } else {
                return Err(e);
            }
        }
        Ok(false)
    } else {
        if let Some(parent) = job.local.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let own = |t: &Path| std::fs::metadata(local_part(t)).ok().filter(|m| m.is_file()).map(|m| m.len());
        let mut target = job.local.clone();
        let mut p = plan(target.exists(), own(&target), ow);
        if p == Plan::NewName {
            target = free_local(&target)?;
            p = plan(false, own(&target), ow);
        }
        let Plan::Go { resume, .. } = p else { return Ok(true) };
        let part = local_part(&target);
        *job.part.lock().unwrap_or_else(|e| e.into_inner()) = part.display().to_string();
        let expected = c.size(&job.remote).ok().flatten();
        c.download(&job.remote, &part, resume, &mut prog)?;
        check_complete(std::fs::metadata(&part).ok().map(|m| m.len()), expected)?;
        // Umbenennen ersetzt eine vorhandene Datei in einem Schritt (Windows und Linux).
        std::fs::rename(&part, &target)?;
        Ok(false)
    }
}

/// «Abbrechen und verwerfen»: Teil-Datei löschen (Server: dazu bei Bedarf neu verbinden).
fn discard_part(job: &Job, conn: &mut Option<Box<dyn Remote>>, connect: &dyn Fn() -> Res<Box<dyn Remote>>, log: &Sender<String>) {
    let part = job.part.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let part = if part.is_empty() {
        if job.up {
            part_name(&job.remote)
        } else {
            local_part(&job.local).display().to_string()
        }
    } else {
        part
    };
    if !job.up {
        let _ = std::fs::remove_file(&part);
        return;
    }
    if conn.is_none() {
        match connect() {
            Ok(c) => *conn = Some(c),
            Err(e) => {
                let _ = log.send(tf("Teil-Datei «{p}» nicht gelöscht: {e}", &[("p", &part), ("e", &e)]));
                return;
            }
        }
    }
    if let Some(c) = conn.as_mut() {
        if c.exists(&part).unwrap_or(false) {
            if let Err(e) = c.remove_file(&part) {
                let _ = log.send(tf("Teil-Datei «{p}» nicht gelöscht: {e}", &[("p", &part), ("e", &e)]));
            }
        }
    }
}

/// Legt die Unterordner zwischen `root` (schon vorhanden) und `dir` an, soweit nötig.
fn ensure_remote_dirs(c: &mut dyn Remote, root: &str, dir: &str, made: &mut HashSet<String>) -> Res<()> {
    let root_n = root.trim_end_matches('/');
    let dir_n = dir.trim_end_matches('/');
    // «/web/app» liegt unter «/web», «/webx» aber nicht
    let Some(rest) = dir_n.strip_prefix(root_n).filter(|r| r.starts_with('/') || root_n.is_empty()) else {
        return Ok(());
    };
    if dir_n == root_n || dir_n.is_empty() {
        return Ok(());
    }
    let rest = rest.trim_start_matches('/');
    let mut cur = root_n.to_string();
    if cur.is_empty() && dir_n.starts_with('/') {
        cur = String::new();
    }
    for part in rest.split('/').filter(|p| !p.is_empty()) {
        cur = format!("{cur}/{part}");
        if made.contains(&cur) {
            continue;
        }
        let _ = c.mkdir(&cur); // «gibt es schon» ist kein Fehler
        made.insert(cur.clone());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::Prog;
    use std::collections::BTreeMap;

    /// Server im Speicher: Pfad → Inhalt (None = Ordner).
    #[derive(Default)]
    struct Mock {
        fs: BTreeMap<String, Option<Vec<u8>>>,
        /// Server, der beim Hochladen still die Hälfte verliert
        lossy: bool,
    }

    /// Daten in Blöcken übertragen, mit Fortschritt und Abbruch wie echte Verbindungen.
    fn pump(data: &[u8], offset: usize, prog: Prog, out: &mut Vec<u8>) -> Res<u64> {
        let total = data.len() as u64;
        if !prog(Tick::Start { offset: offset as u64, total }) {
            return Err(RErr::Cancelled);
        }
        let mut sent = 0u64;
        for ch in data.get(offset..).unwrap_or(&[]).chunks(1000) {
            out.extend_from_slice(ch);
            sent += ch.len() as u64;
            if !prog(Tick::Bytes(ch.len() as u64)) {
                return Err(RErr::Cancelled);
            }
        }
        Ok(sent)
    }

    impl Remote for Mock {
        fn pwd(&mut self) -> Res<String> {
            Ok("/".into())
        }
        fn list(&mut self, path: &str) -> Res<Vec<Entry>> {
            if path != "/" && !matches!(self.fs.get(path), Some(None)) {
                return Err(RErr::Msg(format!("550 {path}")));
            }
            let pre = if path == "/" { "/".to_string() } else { format!("{path}/") };
            Ok(self
                .fs
                .iter()
                .filter_map(|(k, v)| {
                    let rest = k.strip_prefix(&pre)?;
                    (!rest.is_empty() && !rest.contains('/')).then(|| Entry { name: rest.to_string(), is_dir: v.is_none(), size: v.as_ref().map(|d| d.len() as u64).unwrap_or(0), ..Default::default() })
                })
                .collect())
        }
        fn mkdir(&mut self, path: &str) -> Res<()> {
            if self.fs.contains_key(path) {
                return Err(RErr::Msg("550 exists".into()));
            }
            self.fs.insert(path.to_string(), None);
            Ok(())
        }
        fn rename(&mut self, from: &str, to: &str) -> Res<()> {
            // wie SFTP Version 3: nicht auf ein vorhandenes Ziel
            if self.fs.contains_key(to) {
                return Err(RErr::Msg("550 exists".into()));
            }
            let v = self.fs.remove(from).ok_or(RErr::Msg("550".into()))?;
            self.fs.insert(to.to_string(), v);
            Ok(())
        }
        fn remove_file(&mut self, path: &str) -> Res<()> {
            match self.fs.get(path) {
                Some(Some(_)) => {
                    self.fs.remove(path);
                    Ok(())
                }
                _ => Err(RErr::Msg("550".into())),
            }
        }
        fn remove_dir(&mut self, path: &str) -> Res<()> {
            let pre = format!("{path}/");
            if self.fs.keys().any(|k| k.starts_with(&pre)) {
                return Err(RErr::Msg("550 not empty".into()));
            }
            self.fs.remove(path).map(|_| ()).ok_or(RErr::Msg("550".into()))
        }
        fn chmod(&mut self, _path: &str, _mode: u32) -> Res<()> {
            Ok(())
        }
        fn size(&mut self, path: &str) -> Res<Option<u64>> {
            Ok(self.fs.get(path).and_then(|v| v.as_ref()).map(|d| d.len() as u64))
        }
        fn download(&mut self, remote: &str, local: &Path, resume: bool, prog: Prog) -> Res<u64> {
            let d = self.fs.get(remote).cloned().flatten().ok_or(RErr::Msg("550".into()))?;
            let mut have = if resume { std::fs::read(local).unwrap_or_default() } else { Vec::new() };
            if have.len() > d.len() {
                have.clear();
            }
            let off = have.len();
            let r = pump(&d, off, prog, &mut have);
            std::fs::write(local, &have)?;
            r
        }
        fn upload(&mut self, local: &Path, remote: &str, resume: bool, prog: Prog) -> Res<u64> {
            let parent = rparent(remote);
            if parent != "/" && !matches!(self.fs.get(&parent), Some(None)) {
                return Err(RErr::Msg(format!("553 kein Ordner {parent}")));
            }
            let d = std::fs::read(local)?;
            let mut have = if resume { self.fs.get(remote).cloned().flatten().unwrap_or_default() } else { Vec::new() };
            if have.len() > d.len() {
                have.clear();
            }
            let off = have.len();
            let r = pump(&d, off, prog, &mut have);
            if self.lossy {
                let half = have.len() / 2;
                have.truncate(half);
            }
            self.fs.insert(remote.to_string(), Some(have));
            r
        }
        fn keepalive(&mut self) -> Res<()> {
            Ok(())
        }
        fn close(&mut self) {}
    }

    fn queue() -> Arc<Queue> {
        Queue::new(Arc::new(|| {}), Overwrite::Overwrite)
    }

    #[test]
    fn lokale_namen_sicher() {
        assert_eq!(safe_local_name(".."), None);
        assert_eq!(safe_local_name("."), None);
        assert_eq!(safe_local_name(""), None);
        assert_eq!(safe_local_name("../../etc/passwd").as_deref(), Some(".._.._etc_passwd"));
        assert_eq!(safe_local_name(".htaccess").as_deref(), Some(".htaccess"));
        assert_eq!(safe_local_name("Grüezi – 日本語.txt").as_deref(), Some("Grüezi – 日本語.txt"));
        assert_eq!(safe_local_name("a\\b").as_deref(), Some("a_b"));
    }

    #[test]
    fn unterordner_anlegen() {
        let mut m = Mock::default();
        m.fs.insert("/web".into(), None);
        let mut made = HashSet::new();
        ensure_remote_dirs(&mut m, "/web", "/web/app/ü/tief", &mut made).unwrap();
        assert!(matches!(m.fs.get("/web/app/ü/tief"), Some(None)));
        // «/webx» liegt nicht unter «/web»
        ensure_remote_dirs(&mut m, "/web", "/webx/a", &mut made).unwrap();
        assert!(!m.fs.contains_key("/webx/a"));
        // Wurzel «/»
        ensure_remote_dirs(&mut m, "/", "/neu/x", &mut made).unwrap();
        assert!(matches!(m.fs.get("/neu/x"), Some(None)));
    }

    #[test]
    fn ordner_hochladen_mit_htaccess_und_utf8() {
        let base = std::env::temp_dir().join(format!("ftpwerk-xfer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let src = base.join("Seite ü");
        std::fs::create_dir_all(src.join("Unter 日本語").join(".git")).unwrap();
        std::fs::create_dir_all(src.join("leer")).unwrap();
        std::fs::write(src.join(".htaccess"), "Options -Indexes\n").unwrap();
        std::fs::write(src.join("index.html"), "<h1>Grüezi</h1>").unwrap();
        std::fs::write(src.join("Unter 日本語").join(".git").join("HEAD"), "x").unwrap();

        let mut m = Mock::default();
        let q = queue();
        let n = plan_upload_dir(&mut m, &src, "/www/Seite ü", "/www", &q, 0).unwrap();
        assert_eq!(n, 3);
        assert!(matches!(m.fs.get("/www/Seite ü/leer"), Some(None)), "leerer Ordner wird angelegt");
        // Aufträge abarbeiten wie ein Übertragungs-Thread
        let mut made = HashSet::new();
        m.fs.insert("/www".into(), None);
        for j in q.snapshot() {
            assert!(j.up);
            ensure_remote_dirs(&mut m, &j.root_remote, &rparent(&j.remote), &mut made).unwrap();
            m.upload(&j.local, &j.remote, false, &mut |_| true).unwrap();
        }
        assert_eq!(m.fs.get("/www/Seite ü/.htaccess"), Some(&Some(b"Options -Indexes\n".to_vec())));
        assert!(m.fs.contains_key("/www/Seite ü/Unter 日本語/.git/HEAD"));

        // wieder herunterladen
        let q2 = queue();
        let dl = base.join("zurück");
        let k = plan_download_dir(&mut m, "/www/Seite ü", &dl, &q2, 0).unwrap();
        assert_eq!(k, 3);
        assert!(dl.join("leer").is_dir());
        for j in q2.snapshot() {
            if let Some(p) = j.local.parent() {
                std::fs::create_dir_all(p).unwrap();
            }
            m.download(&j.remote, &j.local, false, &mut |_| true).unwrap();
        }
        assert_eq!(std::fs::read(dl.join(".htaccess")).unwrap(), b"Options -Indexes\n");
        assert!(dl.join("Unter 日本語").join(".git").join("HEAD").is_file());

        // rekursiv löschen
        delete_tree(&mut m, "/www/Seite ü", true, 0).unwrap();
        assert!(!m.fs.keys().any(|k| k.starts_with("/www/Seite ü")));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn entscheidung_nie_an_fremde_datei_anhaengen() {
        use Overwrite as O;
        // Ziel fehlt: neu, oder eigene Teil-Datei fortsetzen
        assert_eq!(plan(false, None, O::Overwrite), Plan::Go { resume: false, replace: false });
        assert_eq!(plan(false, Some(500), O::Skip), Plan::Go { resume: true, replace: false });
        assert_eq!(plan(false, Some(0), O::Overwrite), Plan::Go { resume: false, replace: false });
        // Ziel vorhanden (fremd): nie anhängen, sondern Einstellung
        assert_eq!(plan(true, None, O::Skip), Plan::Skip);
        assert_eq!(plan(true, None, O::Rename), Plan::NewName);
        assert_eq!(plan(true, None, O::Overwrite), Plan::Go { resume: false, replace: true });
        assert_eq!(plan(true, Some(10), O::Overwrite), Plan::Go { resume: true, replace: true });
        // Grössenprüfung vor dem Umbenennen
        assert!(check_complete(Some(5), Some(5)).is_ok());
        assert!(check_complete(Some(4), Some(5)).is_err());
        assert!(check_complete(None, Some(5)).is_ok());
        assert!(check_complete(Some(5), None).is_ok());
    }

    fn job(q: &Arc<Queue>, up: bool, remote: &str, local: &Path) -> Arc<Job> {
        q.add(up, remote.into(), local.to_path_buf(), "/".into(), 0);
        q.snapshot().pop().unwrap()
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ftpwerk-teil-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn data(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 7 % 251) as u8).collect()
    }

    #[test]
    fn herunterladen_fremde_kleinere_datei_bleibt() {
        let d = tmp("fremd");
        let mut m = Mock::default();
        m.fs.insert("/a.bin".into(), Some(data(5000)));
        let target = d.join("a.bin");
        let q = queue();
        let no = || false;
        // «Überspringen»: fremde Datei unverändert, nichts angehängt
        std::fs::write(&target, b"fremd").unwrap();
        assert!(transfer(&mut m, &job(&q, false, "/a.bin", &target), Overwrite::Skip, &mut HashSet::new(), &no).unwrap());
        assert_eq!(std::fs::read(&target).unwrap(), b"fremd");
        // «Umbenennen»: fremde Datei bleibt, neue unter «a (2).bin»
        assert!(!transfer(&mut m, &job(&q, false, "/a.bin", &target), Overwrite::Rename, &mut HashSet::new(), &no).unwrap());
        assert_eq!(std::fs::read(&target).unwrap(), b"fremd");
        assert_eq!(std::fs::read(d.join("a (2).bin")).unwrap(), data(5000));
        // «Ersetzen»: ganz neu, nie angehängt
        let j = job(&q, false, "/a.bin", &target);
        transfer(&mut m, &j, Overwrite::Overwrite, &mut HashSet::new(), &no).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), data(5000));
        assert_eq!(j.resumed_from.load(Ordering::Relaxed), 0);
        assert!(!local_part(&target).exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn herunterladen_abbrechen_fortsetzen_verwerfen() {
        let d = tmp("teil");
        let mut m = Mock::default();
        m.fs.insert("/b.bin".into(), Some(data(8000)));
        let target = d.join("b.bin");
        std::fs::write(&target, b"alte Fassung").unwrap();
        let q = queue();
        let no = || false;
        let j = job(&q, false, "/b.bin", &target);
        j.stop_at.store(3000, Ordering::Relaxed);
        assert_eq!(transfer(&mut m, &j, Overwrite::Overwrite, &mut HashSet::new(), &no), Err(RErr::Cancelled));
        // alte Datei unberührt, eigene Teil-Datei liegt daneben
        assert_eq!(std::fs::read(&target).unwrap(), b"alte Fassung");
        assert_eq!(std::fs::metadata(local_part(&target)).unwrap().len(), 3000);
        // Fortsetzen aus der eigenen Teil-Datei
        j.cancel.store(false, Ordering::Relaxed);
        transfer(&mut m, &j, Overwrite::Overwrite, &mut HashSet::new(), &no).unwrap();
        assert_eq!(j.resumed_from.load(Ordering::Relaxed), 3000);
        assert_eq!(std::fs::read(&target).unwrap(), data(8000));
        assert!(!local_part(&target).exists());
        // Abbrechen und verwerfen: Teil-Datei weg
        let j2 = job(&q, false, "/b.bin", &d.join("c.bin"));
        j2.stop_at.store(2000, Ordering::Relaxed);
        let _ = transfer(&mut m, &j2, Overwrite::Overwrite, &mut HashSet::new(), &no);
        assert!(local_part(&d.join("c.bin")).exists());
        let (tx, _rx) = channel();
        discard_part(&j2, &mut None, &|| Err(RErr::Msg("x".into())), &tx);
        assert!(!local_part(&d.join("c.bin")).exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn hochladen_alte_datei_bleibt_bis_zum_ende() {
        let d = tmp("hoch");
        let local = d.join(".htaccess");
        std::fs::write(&local, data(9000)).unwrap();
        let mut m = Mock::default();
        m.fs.insert("/www".into(), None);
        m.fs.insert("/www/.htaccess".into(), Some(b"Require all denied".to_vec()));
        let q = queue();
        let no = || false;
        let j = job(&q, true, "/www/.htaccess", &local);
        j.stop_at.store(4000, Ordering::Relaxed);
        assert_eq!(transfer(&mut m, &j, Overwrite::Overwrite, &mut HashSet::new(), &no), Err(RErr::Cancelled));
        assert_eq!(m.fs.get("/www/.htaccess"), Some(&Some(b"Require all denied".to_vec())), "alte .htaccess muss bleiben");
        assert_eq!(m.fs.get("/www/.htaccess.ftpwerk-teil").and_then(|v| v.as_ref()).map(|v| v.len()), Some(4000));
        // fortsetzen; Server benennt nicht auf vorhandenes Ziel um → bei «Ersetzen» löschen und umbenennen
        j.cancel.store(false, Ordering::Relaxed);
        transfer(&mut m, &j, Overwrite::Overwrite, &mut HashSet::new(), &no).unwrap();
        assert_eq!(j.resumed_from.load(Ordering::Relaxed), 4000);
        assert_eq!(m.fs.get("/www/.htaccess"), Some(&Some(data(9000))));
        assert!(!m.fs.contains_key("/www/.htaccess.ftpwerk-teil"));
        // fremde kleinere Datei auf dem Server: «Umbenennen» lässt sie unverändert
        m.fs.insert("/www/x.bin".into(), Some(b"fremd".to_vec()));
        let j = job(&q, true, "/www/x.bin", &local);
        transfer(&mut m, &j, Overwrite::Rename, &mut HashSet::new(), &no).unwrap();
        assert_eq!(m.fs.get("/www/x.bin"), Some(&Some(b"fremd".to_vec())));
        assert_eq!(m.fs.get("/www/x (2).bin"), Some(&Some(data(9000))));
        // «Überspringen» ebenso
        assert!(transfer(&mut m, &job(&q, true, "/www/x.bin", &local), Overwrite::Skip, &mut HashSet::new(), &no).unwrap());
        assert_eq!(m.fs.get("/www/x.bin"), Some(&Some(b"fremd".to_vec())));
        // Grösse stimmt nicht → nicht umbenennen, altes Ziel bleibt
        m.lossy = true;
        let r = transfer(&mut m, &job(&q, true, "/www/x.bin", &local), Overwrite::Overwrite, &mut HashSet::new(), &no);
        assert!(matches!(r, Err(RErr::Msg(_))), "{r:?}");
        assert_eq!(m.fs.get("/www/x.bin"), Some(&Some(b"fremd".to_vec())));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn teil_dateien_werden_nicht_mit_uebertragen() {
        let mut m = Mock::default();
        m.fs.insert("/d".into(), None);
        m.fs.insert("/d/a.txt".into(), Some(b"a".to_vec()));
        m.fs.insert("/d/b.txt.ftpwerk-teil".into(), Some(b"b".to_vec()));
        let q = queue();
        let dl = tmp("plan");
        assert_eq!(plan_download_dir(&mut m, "/d", &dl, &q, 0).unwrap(), 1);
        let _ = std::fs::remove_dir_all(&dl);
    }

    #[test]
    fn warteschlange() {
        let q = queue();
        q.add(true, "/a".into(), PathBuf::from("a"), "/".into(), 10);
        q.add(true, "/b".into(), PathBuf::from("b"), "/".into(), 10);
        assert_eq!(q.counts(), (2, 0, 0, 0));
        let j = q.next_job().unwrap();
        assert_eq!(j.status(), RUN);
        q.cancel(2);
        assert_eq!(q.counts(), (0, 1, 0, 0));
        j.status.store(FAIL, Ordering::Relaxed);
        q.retry(None);
        assert_eq!(q.counts(), (2, 0, 0, 0));
        // verwerfen: abgebrochener Auftrag geht zum Aufräumen zurück an einen Thread, nicht wiederholbar
        q.cancel(1);
        j.status.store(CANCELLED, Ordering::Relaxed);
        q.discard(1);
        assert!(j.discard.load(Ordering::Relaxed) && j.cancel.load(Ordering::Relaxed));
        assert_eq!(j.status(), WAIT);
        j.status.store(CANCELLED, Ordering::Relaxed);
        q.retry(Some(1));
        assert_eq!(j.status(), CANCELLED);
        q.set_paused(true);
        q.stop.store(true, Ordering::Relaxed);
        assert!(q.next_job().is_none());
    }
}
