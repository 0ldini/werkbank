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

use crate::model::{rjoin, rparent, Entry, Proto, RErr, Res, Site};
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
        Overwrite::Resume => 0,
        Overwrite::Overwrite => 1,
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
            1 => Overwrite::Overwrite,
            2 => Overwrite::Skip,
            _ => Overwrite::Resume,
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
            if (id.is_none() || id == Some(j.id)) && matches!(j.status(), FAIL | CANCELLED) {
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
    let fps = trust.lock().map(|t| t.for_host(trust_kind(site.proto), &site.hostport())).unwrap_or_default();
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
            threads.push(thread::spawn(move || browser_loop(site, trust, cmd_rx, ev_tx, log_tx, wake, queue)));
        }
        for _ in 0..parallel.clamp(1, 6) {
            let (site, trust, log_tx, queue, wake) = (site.clone(), trust.clone(), log_tx.clone(), queue.clone(), wake.clone());
            threads.push(thread::spawn(move || worker_loop(site, trust, queue, log_tx, wake)));
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
                job.status.store(CANCELLED, Ordering::Relaxed);
                // Nach einem Abbruch ist der Datenstrom in unklarem Zustand: Verbindung neu aufbauen
                if let Some(mut c) = conn.take() {
                    c.close();
                }
            }
            Err(e) => {
                if let Ok(mut m) = job.error.lock() {
                    *m = e.to_string();
                }
                let _ = log.send(tf("Fehler bei «{n}»: {e}", &[("n", &job.name()), ("e", &e)]));
                job.status.store(FAIL, Ordering::Relaxed);
                if e.is_disconnect() {
                    conn = None;
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

        let mut prog = |tick: Tick| -> bool {
            match tick {
                Tick::Start { offset, total } => {
                    if total > 0 {
                        job.total.store(total, Ordering::Relaxed);
                    }
                    job.done.store(offset, Ordering::Relaxed);
                }
                Tick::Bytes(n) => {
                    job.done.fetch_add(n, Ordering::Relaxed);
                }
            }
            !(job.cancel.load(Ordering::Relaxed) || queue.stop.load(Ordering::Relaxed))
        };

        let outcome: Res<bool> = (|| {
            if job.up {
                if ow == Overwrite::Skip && c.size(&job.remote).ok().flatten().is_some() {
                    return Ok(true);
                }
                ensure_remote_dirs(c, &job.root_remote, &rparent(&job.remote), made)?;
                c.upload(&job.local, &job.remote, ow == Overwrite::Resume, &mut prog).map(|_| false)
            } else {
                if ow == Overwrite::Skip && job.local.exists() {
                    return Ok(true);
                }
                if let Some(parent) = job.local.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                c.download(&job.remote, &job.local, ow == Overwrite::Resume, &mut prog).map(|_| false)
            }
        })();

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

/// Legt die Unterordner zwischen `root` (schon vorhanden) und `dir` an, soweit nötig.
fn ensure_remote_dirs(c: &mut dyn Remote, root: &str, dir: &str, made: &mut HashSet<String>) -> Res<()> {
    let root_n = root.trim_end_matches('/');
    let dir_n = dir.trim_end_matches('/');
    if dir_n == root_n || dir_n.is_empty() || !dir_n.starts_with(root_n) {
        return Ok(());
    }
    let rest = dir_n[root_n.len()..].trim_start_matches('/');
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
