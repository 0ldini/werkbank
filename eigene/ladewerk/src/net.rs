//! Der eigentliche Download eines Links (läuft in einem eigenen Thread):
//! HTTP über ureq 3 mit rustls, Proxy aus der Umgebung oder den Einstellungen,
//! Fortsetzen über `Range` (Teildatei «name.part»), gemeinsame Geschwindigkeitsgrenze,
//! Dateiname aus Content-Disposition oder Adresse, optional SHA-256-Prüfung.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::links;
use crate::plan::{self, HttpClass};
use crate::sprache::{t, tf};

/// Sperre holen, auch wenn ein anderer Thread dabei abgestürzt ist.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

/// Phase eines laufenden Downloads.
pub const PHASE_CONNECT: u8 = 0;
pub const PHASE_LOAD: u8 = 1;
pub const PHASE_CHECK: u8 = 2;

/// Was der Download-Thread über sich mitteilt (nur Atomics und kurze Sperren).
#[derive(Default)]
pub struct Run {
    pub cancel: AtomicBool,
    pub done: AtomicU64,
    /// Gesamtgrösse, `u64::MAX` = unbekannt.
    pub total: AtomicU64,
    pub phase: AtomicU8,
    pub meta: Mutex<Meta>,
    pub result: Mutex<Option<Outcome>>,
    pub finished: AtomicBool,
}

impl Run {
    pub fn new(done: u64, total: Option<u64>) -> Run {
        let r = Run::default();
        r.done.store(done, Ordering::Relaxed);
        r.total.store(total.unwrap_or(u64::MAX), Ordering::Relaxed);
        r
    }
}

#[derive(Clone, Default, Debug)]
pub struct Meta {
    pub file_name: String,
    pub etag: String,
    pub last_modified: String,
    /// Ergebnis der Prüfsummen-Prüfung.
    pub sha_ok: Option<bool>,
}

#[derive(Clone, Debug)]
pub enum Outcome {
    Done,
    Cancelled,
    /// Vorübergehender Fehler: später nochmals versuchen.
    Retry(String),
    /// Endgültiger Fehler.
    Fatal(String),
}

pub struct Job {
    pub url: String,
    pub dir: PathBuf,
    /// Bekannter Dateiname (beim Fortsetzen), sonst leer.
    pub file_name: String,
    pub etag: String,
    pub last_modified: String,
    pub sha256: String,
    pub proxy: String,
}

/// Gemeinsame Geschwindigkeitsgrenze aller Downloads.
pub struct Limiter {
    rate: AtomicU64,
    bucket: Mutex<plan::Bucket>,
    start: Instant,
}

impl Default for Limiter {
    fn default() -> Self {
        Limiter { rate: AtomicU64::new(0), bucket: Mutex::new(plan::Bucket::default()), start: Instant::now() }
    }
}

impl Limiter {
    /// Bytes pro Sekunde, 0 = unbegrenzt.
    pub fn set_rate(&self, bps: u64) {
        self.rate.store(bps, Ordering::Relaxed);
    }

    fn take(&self, n: usize, cancel: &AtomicBool) {
        let rate = self.rate.load(Ordering::Relaxed) as f64;
        let wait = {
            let mut b = lock(&self.bucket);
            b.take(self.start.elapsed().as_secs_f64(), rate, n as u64)
        };
        let until = Instant::now() + Duration::from_secs_f64(wait.clamp(0.0, 60.0));
        while !cancel.load(Ordering::Relaxed) {
            let now = Instant::now();
            if now >= until {
                break;
            }
            std::thread::sleep((until - now).min(Duration::from_millis(100)));
        }
    }
}

/// Dateinamen, die gerade ein Download für sich beansprucht (gegen Doppelvergabe).
static RESERVED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

fn name_taken(dir: &Path, name: &str) -> bool {
    let p = dir.join(name);
    p.exists() || dir.join(format!("{name}.part")).exists() || lock(&RESERVED).contains(&p)
}

fn reserve(dir: &Path, wanted: &str) -> String {
    let name = links::unique_name(wanted, |n| name_taken(dir, n));
    lock(&RESERVED).push(dir.join(&name));
    name
}

fn release(dir: &Path, name: &str) {
    let p = dir.join(name);
    lock(&RESERVED).retain(|x| x != &p);
}

pub fn part_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.part"))
}

fn agent(proxy: &str) -> Result<ureq::Agent, Outcome> {
    use ureq::tls::{RootCerts, TlsConfig};
    let proxy = if proxy.trim().is_empty() {
        ureq::Proxy::try_from_env()
    } else {
        match ureq::Proxy::new(proxy.trim()) {
            Ok(p) => Some(p),
            Err(e) => return Err(Outcome::Fatal(tf("Proxy-Adresse ungültig: {e}", &[("e", &e)]))),
        }
    };
    Ok(ureq::Agent::config_builder()
        .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
        .proxy(proxy)
        .user_agent(concat!("Ladewerk/", env!("CARGO_PKG_VERSION")))
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        // Fehlercodes selbst auswerten, Weiterleitungen selbst verfolgen (Range bleibt dabei).
        .http_status_as_error(false)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .build()
        .into())
}

fn header(resp: &ureq::http::Response<ureq::Body>, name: &str) -> Option<String> {
    resp.headers().get(name).map(|v| String::from_utf8_lossy(v.as_bytes()).trim().to_string()).filter(|s| !s.is_empty())
}

/// Anfrage stellen und Weiterleitungen folgen. Liefert Antwort und endgültige Adresse.
fn request(agent: &ureq::Agent, url: &str, offset: u64, validator: &str) -> Result<(ureq::http::Response<ureq::Body>, String), Outcome> {
    let mut cur = url.trim().to_string();
    for _ in 0..12 {
        let lower = cur.to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            return Err(Outcome::Fatal(t("Nur http- und https-Links werden unterstützt.").to_string()));
        }
        let mut req = agent.get(&cur);
        if offset > 0 {
            req = req.header("Range", format!("bytes={offset}-"));
            if !validator.is_empty() {
                req = req.header("If-Range", validator);
            }
        }
        let resp = req.call().map_err(|e| Outcome::Retry(tf("Verbindungsfehler: {e}", &[("e", &e)])))?;
        let code = resp.status().as_u16();
        if plan::classify(code) == HttpClass::Redirect {
            let loc = header(&resp, "location").unwrap_or_default();
            match links::resolve_location(&cur, &loc) {
                Some(next) => {
                    cur = next;
                    continue;
                }
                None => return Err(Outcome::Fatal(tf("Ungültige Weiterleitung (HTTP {code})", &[("code", &code)]))),
            }
        }
        return Ok((resp, cur));
    }
    Err(Outcome::Fatal(t("Zu viele Weiterleitungen").to_string()))
}

/// Lädt einen Link herunter. Gibt nie auf, ohne die Teildatei zu behalten (ausser der Server
/// verlangt einen Neuanfang). Bestehende Dateien werden nie überschrieben.
pub fn download(job: &Job, run: &Run, limiter: &Limiter) -> Outcome {
    let mut reserved: Option<String> = None;
    let out = download_inner(job, run, limiter, &mut reserved);
    if let Some(name) = reserved {
        release(&job.dir, &name);
    }
    out
}

fn set_meta(run: &Run, f: impl FnOnce(&mut Meta)) {
    f(&mut lock(&run.meta));
}

fn download_inner(job: &Job, run: &Run, limiter: &Limiter, reserved: &mut Option<String>) -> Outcome {
    run.phase.store(PHASE_CONNECT, Ordering::Relaxed);
    if let Err(e) = fs::create_dir_all(&job.dir) {
        return Outcome::Fatal(tf("Ordner lässt sich nicht anlegen: {e}", &[("e", &e)]));
    }
    let agent = match agent(&job.proxy) {
        Ok(a) => a,
        Err(o) => return o,
    };

    // Fortsetzen: bekannte Teildatei und ihre Länge.
    let mut name = job.file_name.clone();
    let mut offset = 0u64;
    if !name.is_empty() {
        offset = fs::metadata(part_path(&job.dir, &name)).map(|m| m.len()).unwrap_or(0);
    }
    let validator = if !job.etag.is_empty() && !job.etag.starts_with("W/") { job.etag.as_str() } else { job.last_modified.as_str() };

    let (mut resp, final_url) = match request(&agent, &job.url, offset, validator) {
        Ok(r) => r,
        Err(o) => return o,
    };
    if run.cancel.load(Ordering::Relaxed) {
        return Outcome::Cancelled;
    }
    let code = resp.status().as_u16();
    let total: Option<u64>;
    let append: bool;
    match plan::classify(code) {
        HttpClass::Partial => {
            match header(&resp, "content-range").as_deref().and_then(plan::parse_content_range) {
                Some((start, _, tot)) if start == offset => {
                    append = true;
                    total = tot;
                }
                // Der Server liefert einen anderen Bereich: von vorne beginnen.
                _ => return Outcome::Retry(t("Server lieferte einen falschen Bereich – beginne neu").to_string()).restart(job, &name),
            }
        }
        HttpClass::Full => {
            total = header(&resp, "content-length").and_then(|v| v.parse().ok());
            // Server kann nicht fortsetzen (oder die Datei hat sich geändert): von vorne.
            append = false;
            offset = 0;
        }
        HttpClass::RangeNotSatisfiable => {
            let tot = header(&resp, "content-range").as_deref().and_then(plan::parse_content_range).and_then(|r| r.2);
            if offset > 0 && tot == Some(offset) {
                // Die Teildatei ist schon vollständig.
                total = tot;
                append = true;
            } else {
                return Outcome::Retry(t("Server lehnt das Fortsetzen ab – beginne neu").to_string()).restart(job, &name);
            }
        }
        HttpClass::Retry => return Outcome::Retry(tf("Server meldet HTTP {code}", &[("code", &code)])),
        HttpClass::Fatal | HttpClass::Redirect => {
            let msg = match code {
                401 | 403 => tf("Zugriff verweigert (HTTP {code})", &[("code", &code)]),
                404 | 410 => tf("Datei nicht gefunden (HTTP {code})", &[("code", &code)]),
                _ => tf("Server meldet HTTP {code}", &[("code", &code)]),
            };
            return Outcome::Fatal(msg);
        }
    }

    // Dateiname festlegen, falls noch unbekannt.
    if name.is_empty() {
        let wanted = header(&resp, "content-disposition")
            .and_then(|h| links::content_disposition_filename(&h))
            .or_else(|| links::file_name_from_url(&final_url))
            .or_else(|| links::file_name_from_url(&job.url))
            .unwrap_or_else(|| "download".to_string());
        name = reserve(&job.dir, &wanted);
        *reserved = Some(name.clone());
    }
    let etag = header(&resp, "etag").unwrap_or_default();
    let last_modified = header(&resp, "last-modified").unwrap_or_default();
    set_meta(run, |m| {
        m.file_name = name.clone();
        if code != 416 {
            m.etag = etag;
            m.last_modified = last_modified;
        }
    });
    run.total.store(total.unwrap_or(u64::MAX), Ordering::Relaxed);

    let part = part_path(&job.dir, &name);
    if code != 416 {
        let file = if append {
            OpenOptions::new().append(true).open(&part)
        } else {
            // Neu beginnen: Teildatei (nur unsere eigene .part) leeren.
            offset = 0;
            File::create(&part)
        };
        let mut file = match file {
            Ok(f) => f,
            Err(e) => return Outcome::Fatal(tf("Datei lässt sich nicht schreiben: {e}", &[("e", &e)])),
        };
        run.done.store(offset, Ordering::Relaxed);
        run.phase.store(PHASE_LOAD, Ordering::Relaxed);
        let mut reader = resp.body_mut().as_reader();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            if run.cancel.load(Ordering::Relaxed) {
                let _ = file.flush();
                return Outcome::Cancelled;
            }
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n.min(buf.len()),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    let _ = file.flush();
                    return Outcome::Retry(tf("Verbindung unterbrochen: {e}", &[("e", &e)]));
                }
            };
            limiter.take(n, &run.cancel);
            if let Err(e) = file.write_all(&buf[..n]) {
                return Outcome::Fatal(tf("Schreiben fehlgeschlagen (Speicher voll?): {e}", &[("e", &e)]));
            }
            offset = offset.saturating_add(n as u64);
            run.done.store(offset, Ordering::Relaxed);
        }
        if let Err(e) = file.sync_all() {
            return Outcome::Fatal(tf("Schreiben fehlgeschlagen (Speicher voll?): {e}", &[("e", &e)]));
        }
        drop(file);
        if let Some(tot) = total {
            if offset < tot {
                return Outcome::Retry(t("Verbindung vorzeitig beendet").to_string());
            }
        }
    }
    if total.is_none() {
        run.total.store(offset, Ordering::Relaxed);
    }

    // Teildatei unter dem endgültigen Namen ablegen – nie über eine bestehende Datei.
    let mut final_name = name.clone();
    if job.dir.join(&final_name).exists() {
        final_name = links::unique_name(&name, |n| job.dir.join(n).exists() || part_path(&job.dir, n).exists());
    }
    if let Err(e) = fs::rename(&part, job.dir.join(&final_name)) {
        return Outcome::Fatal(tf("Umbenennen fehlgeschlagen: {e}", &[("e", &e)]));
    }
    set_meta(run, |m| m.file_name = final_name.clone());

    // Prüfsumme.
    if let Some(expected) = links::parse_sha256(&job.sha256) {
        run.phase.store(PHASE_CHECK, Ordering::Relaxed);
        match sha256_file(&job.dir.join(&final_name), &run.cancel) {
            Ok(Some(got)) => {
                let ok = got == expected;
                set_meta(run, |m| m.sha_ok = Some(ok));
                if !ok {
                    return Outcome::Fatal(tf("Prüfsumme stimmt nicht (erhalten: {got})", &[("got", &got)]));
                }
            }
            Ok(None) => return Outcome::Cancelled,
            Err(e) => return Outcome::Fatal(tf("Prüfsumme nicht berechenbar: {e}", &[("e", &e)])),
        }
    }
    Outcome::Done
}

impl Outcome {
    /// Bei einem Neuanfang die eigene Teildatei verwerfen, damit der nächste Versuch bei 0 beginnt.
    fn restart(self, job: &Job, name: &str) -> Outcome {
        if !name.is_empty() {
            let _ = fs::remove_file(part_path(&job.dir, name));
        }
        self
    }
}

/// SHA-256 einer Datei; None bei Abbruch.
pub fn sha256_file(path: &Path, cancel: &AtomicBool) -> std::io::Result<Option<String>> {
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let n = match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n.min(buf.len()),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        h.update(&buf[..n]);
    }
    Ok(Some(h.finalize().iter().map(|b| format!("{b:02x}")).collect()))
}
