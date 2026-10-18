//! Ladewerk – Download-Manager im JDownloader-Stil, Teil der Werkbank.
//!
//! Dieser Teil ist der Kern: Linksammler, Downloads (über das curl von Windows/macOS/Linux),
//! Entpacken (eingebautes 7-Zip, sonst installiertes 7-Zip/WinRAR/tar), Hoster-Konten,
//! Click'n'Load und Zwischenablage. Die Oberfläche steht in app.rs (egui).
//! Mit `--browser` läuft statt des Fensters die Browser-Oberfläche (ui.html).
//!
//! Optionen: `--browser`, `--no-open` (nur mit --browser), `--port=9000`
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod sprache;
mod thema;

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const UA: &str = "Mozilla/5.0 (Ladewerk; Rust)";
const MB: u64 = 1024 * 1024;
const NORANGE: &str = "Server liefert keine Teilbereiche";
const ABORT: &str = "Abgebrochen";
const FILE_EXT: &str = " zip rar 7z tar gz bz2 xz tgz iso img dmg exe msi apk deb rpm appimage pdf epub mobi doc docx xls xlsx ppt pptx odt ods csv txt \
mp3 flac wav ogg m4a aac opus mp4 mkv avi mov webm wmv m4v mpg mpeg jpg jpeg png gif webp bmp svg tif tiff psd bin jar torrent srt ttf otf woff woff2 ";

/* ---------- JSON (klein, selbst geschrieben) ---------- */
#[derive(Clone, Debug)]
enum J {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}
static JNULL: J = J::Null;

impl J {
    fn get(&self, k: &str) -> &J {
        match self {
            J::Obj(v) => v.iter().find(|(a, _)| a == k).map(|(_, b)| b).unwrap_or(&JNULL),
            _ => &JNULL,
        }
    }
    fn str(&self) -> Option<&str> {
        if let J::Str(s) = self { Some(s) } else { None }
    }
    fn string(&self) -> String {
        self.str().unwrap_or("").to_string()
    }
    fn bool(&self) -> Option<bool> {
        if let J::Bool(b) = self { Some(*b) } else { None }
    }
    /// Zahl oder Zahl-als-Text (Formularfelder liefern Text).
    fn int(&self) -> Option<i64> {
        match self {
            J::Num(n) if n.is_finite() => Some(*n as i64),
            J::Str(s) => s.trim().parse::<f64>().ok().filter(|n| n.is_finite()).map(|n| n as i64),
            _ => None,
        }
    }
    fn u64(&self) -> Option<u64> {
        self.int().filter(|n| *n >= 0).map(|n| n as u64)
    }
    fn arr(&self) -> &[J] {
        if let J::Arr(a) = self { a } else { &[] }
    }
    fn dump(&self, o: &mut String) {
        match self {
            J::Null => o.push_str("null"),
            J::Bool(b) => o.push_str(if *b { "true" } else { "false" }),
            J::Num(n) => {
                if !n.is_finite() {
                    o.push_str("null")
                } else if n.fract() == 0.0 && n.abs() < 9e15 {
                    o.push_str(&format!("{}", *n as i64))
                } else {
                    o.push_str(&format!("{}", n))
                }
            }
            J::Str(s) => dump_str(s, o),
            J::Arr(a) => {
                o.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 { o.push(','); }
                    v.dump(o);
                }
                o.push(']');
            }
            J::Obj(m) => {
                o.push('{');
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 { o.push(','); }
                    dump_str(k, o);
                    o.push(':');
                    v.dump(o);
                }
                o.push('}');
            }
        }
    }
    fn text(&self) -> String {
        let mut s = String::new();
        self.dump(&mut s);
        s
    }
}
fn dump_str(s: &str, o: &mut String) {
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
}
fn obj(v: Vec<(&str, J)>) -> J {
    J::Obj(v.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}
fn js(s: &str) -> J {
    J::Str(s.to_string())
}
fn jn(n: u64) -> J {
    J::Num(n as f64)
}
fn jopt(n: Option<u64>) -> J {
    n.map(jn).unwrap_or(J::Null)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}
impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\n' | b'\r' | b'\t') {
            self.i += 1;
        }
    }
    fn lit(&mut self, w: &str, v: J) -> Result<J, ()> {
        if self.b[self.i..].starts_with(w.as_bytes()) {
            self.i += w.len();
            Ok(v)
        } else {
            Err(())
        }
    }
    fn hex4(&mut self) -> Result<u32, ()> {
        let s = self.b.get(self.i..self.i + 4).ok_or(())?;
        self.i += 4;
        u32::from_str_radix(std::str::from_utf8(s).map_err(|_| ())?, 16).map_err(|_| ())
    }
    fn string(&mut self) -> Result<String, ()> {
        self.i += 1; // oeffnendes "
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = *self.b.get(self.i).ok_or(())?;
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = *self.b.get(self.i).ok_or(())?;
                    self.i += 1;
                    let ch = match e {
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp) && self.b[self.i..].starts_with(b"\\u") {
                                self.i += 2;
                                let lo = self.hex4()?;
                                if (0xDC00..0xE000).contains(&lo) {
                                    cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                }
                            }
                            char::from_u32(cp).unwrap_or('\u{FFFD}')
                        }
                        other => other as char,
                    };
                    let mut tmp = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut tmp).as_bytes());
                }
                c => out.push(c),
            }
        }
        String::from_utf8(out).map_err(|_| ())
    }
    fn val(&mut self, depth: u32) -> Result<J, ()> {
        if depth > 64 {
            return Err(());
        }
        self.ws();
        match *self.b.get(self.i).ok_or(())? {
            b'{' => {
                self.i += 1;
                let mut m = Vec::new();
                loop {
                    self.ws();
                    match *self.b.get(self.i).ok_or(())? {
                        b'}' => { self.i += 1; break; }
                        b',' => { self.i += 1; }
                        b'"' => {
                            let k = self.string()?;
                            self.ws();
                            if self.b.get(self.i) != Some(&b':') { return Err(()); }
                            self.i += 1;
                            let v = self.val(depth + 1)?;
                            m.push((k, v));
                        }
                        _ => return Err(()),
                    }
                }
                Ok(J::Obj(m))
            }
            b'[' => {
                self.i += 1;
                let mut a = Vec::new();
                loop {
                    self.ws();
                    match *self.b.get(self.i).ok_or(())? {
                        b']' => { self.i += 1; break; }
                        b',' => { self.i += 1; }
                        _ => a.push(self.val(depth + 1)?),
                    }
                }
                Ok(J::Arr(a))
            }
            b'"' => Ok(J::Str(self.string()?)),
            b't' => self.lit("true", J::Bool(true)),
            b'f' => self.lit("false", J::Bool(false)),
            b'n' => self.lit("null", J::Null),
            _ => {
                let s = self.i;
                while self.i < self.b.len() && matches!(self.b[self.i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
                    self.i += 1;
                }
                std::str::from_utf8(&self.b[s..self.i]).ok().and_then(|t| t.parse::<f64>().ok()).map(J::Num).ok_or(())
            }
        }
    }
}
fn parse_json(b: &[u8]) -> Result<J, ()> {
    let mut p = Parser { b, i: 0 };
    let v = p.val(0)?;
    p.ws();
    if p.i == b.len() { Ok(v) } else { Err(()) }
}

/* ---------- Zustand ---------- */
#[derive(Clone)]
struct Seg {
    start: u64,
    end: Option<u64>,
    pos: u64,
    fin: bool,
}
impl Seg {
    fn done(&self) -> bool {
        match self.end {
            Some(e) => self.pos > e,
            None => self.fin,
        }
    }
}

struct Item {
    id: String,
    url: String,
    name: String,
    pkg: String,
    stage: String,  // grab | queue
    status: String, // checking ready offline | queued running paused done error
    error: Option<String>,
    size: Option<u64>,
    probed: bool,
    resumable: bool,
    dir: String,
    segs: Vec<Seg>,
    tries: u32,
    wait_until: u64,
    extract: String, // "" | wait | running | ok | error
    extract_msg: String,
    sha256: String,    // erwartete Prüfsumme (leer = keine)
    sha_state: String, // "" | running | ok | bad | info | error
    sha_got: String,   // berechnete Prüfsumme bzw. Fehlermeldung
}
impl Item {
    fn done_bytes(&self) -> u64 {
        if self.status == "done" {
            self.size.unwrap_or(0)
        } else {
            self.segs.iter().map(|s| s.pos - s.start).sum()
        }
    }
    fn part_path(&self) -> PathBuf {
        Path::new(&self.dir).join(format!("{}.{}.part", self.name, self.id))
    }
}

#[derive(Clone)]
struct Settings {
    dir: String,
    max_parallel: usize,
    chunks: usize,
    speed_limit_kb: u64,
    retries: u32,
    extract: bool,
    delete_archives: bool,
    passwords: Vec<String>,
    clipboard: bool,
    clicknload: bool,
    proxy: String,
}

/// Konto bei einem Hoster. Das Passwort liegt nur geschützt vor (Windows: DPAPI).
struct Account {
    host: String,
    user: String,
    secret: String,
    token: Option<String>, // Sitzung, nur im Speicher
    ok: Option<bool>,
    info: String,
}

struct State {
    active: bool,
    settings: Settings,
    items: Vec<Item>,
    accounts: Vec<Account>,
    pkg_passwords: HashMap<String, Vec<String>>,
}

/// Laufzeitdaten eines aktiven Downloads.
struct Rt {
    stop: AtomicBool,
    failed: AtomicBool,
    removed: AtomicBool,
    stop_status: Mutex<&'static str>,
    children: Mutex<Vec<Arc<Mutex<Child>>>>,
    part: Mutex<Option<PathBuf>>,
    bytes: AtomicU64,
    last: AtomicU64,
    speed: AtomicU64,
    conns: AtomicUsize,
}
impl Rt {
    fn new() -> Rt {
        Rt {
            stop: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            removed: AtomicBool::new(false),
            stop_status: Mutex::new("paused"),
            children: Mutex::new(Vec::new()),
            part: Mutex::new(None),
            bytes: AtomicU64::new(0),
            last: AtomicU64::new(0),
            speed: AtomicU64::new(0),
            conns: AtomicUsize::new(0),
        }
    }
    fn halt(&self) -> bool {
        self.stop.load(SeqCst) || self.failed.load(SeqCst)
    }
    fn kill_all(&self) {
        for c in lk(&self.children).iter() {
            let _ = lk(c).kill();
        }
    }
}

struct App {
    st: Mutex<State>,
    running: Mutex<HashMap<String, Arc<Rt>>>, // Sperr-Reihenfolge: st vor running, nie umgekehrt
    next_free: Mutex<f64>,
    limit: AtomicU64, // Bytes/s, 0 = aus
    dirty: AtomicBool,
    port: u16,
    data_dir: PathBuf,
    extract_q: Mutex<Vec<String>>,
    verify_q: Mutex<Vec<String>>,
}
static APP: OnceLock<App> = OnceLock::new();

fn app() -> &'static App {
    APP.get().expect("App nicht initialisiert")
}
fn lk<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
fn st() -> MutexGuard<'static, State> {
    lk(&app().st)
}
fn with_item<R>(id: &str, f: impl FnOnce(&mut Item) -> R) -> Option<R> {
    let mut s = st();
    s.items.iter_mut().find(|i| i.id == id).map(f)
}
fn mark_dirty() {
    app().dirty.store(true, SeqCst);
}
fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}
fn new_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let h = std::collections::hash_map::RandomState::new().build_hasher().finish();
    format!("{:012x}", h & 0xffff_ffff_ffff)
}
fn home_dir() -> PathBuf {
    let v = if cfg!(windows) { std::env::var_os("USERPROFILE") } else { std::env::var_os("HOME") };
    v.map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/* ---------- Speichern / Laden ---------- */
fn state_json(s: &State) -> J {
    let items = s
        .items
        .iter()
        .map(|it| {
            obj(vec![
                ("id", js(&it.id)),
                ("url", js(&it.url)),
                ("name", js(&it.name)),
                ("pkg", js(&it.pkg)),
                ("stage", js(&it.stage)),
                ("status", js(&it.status)),
                ("error", it.error.as_deref().map(js).unwrap_or(J::Null)),
                ("size", jopt(it.size)),
                ("probed", J::Bool(it.probed)),
                ("resumable", J::Bool(it.resumable)),
                ("dir", js(&it.dir)),
                ("extract", js(if it.extract == "ok" || it.extract == "error" { &it.extract } else { "" })),
                ("extractMsg", js(&it.extract_msg)),
                ("sha256", js(&it.sha256)),
                ("shaState", js(&it.sha_state)),
                ("shaGot", js(&it.sha_got)),
                (
                    "segs",
                    J::Arr(
                        it.segs
                            .iter()
                            .map(|g| obj(vec![("start", jn(g.start)), ("end", jopt(g.end)), ("pos", jn(g.pos)), ("fin", J::Bool(g.fin))]))
                            .collect(),
                    ),
                ),
            ])
        })
        .collect();
    let accounts = s.accounts.iter().map(|a| obj(vec![("host", js(&a.host)), ("user", js(&a.user)), ("secret", js(&a.secret))])).collect();
    obj(vec![
        ("active", J::Bool(s.active)),
        ("settings", settings_json(&s.settings)),
        ("accounts", J::Arr(accounts)),
        ("pkgPasswords", pkg_passwords_json(s)),
        ("items", J::Arr(items)),
    ])
}
fn pkg_passwords_json(s: &State) -> J {
    let mut keys: Vec<&String> = s.pkg_passwords.keys().collect();
    keys.sort();
    J::Obj(keys.into_iter().map(|k| (k.clone(), J::Arr(s.pkg_passwords[k].iter().map(|p| js(p)).collect()))).collect())
}
fn settings_json(s: &Settings) -> J {
    obj(vec![
        ("dir", js(&s.dir)),
        ("maxParallel", jn(s.max_parallel as u64)),
        ("chunks", jn(s.chunks as u64)),
        ("speedLimitKB", jn(s.speed_limit_kb)),
        ("retries", jn(s.retries as u64)),
        ("extract", J::Bool(s.extract)),
        ("deleteArchives", J::Bool(s.delete_archives)),
        ("passwords", J::Arr(s.passwords.iter().map(|p| js(p)).collect())),
        ("clipboard", J::Bool(s.clipboard)),
        ("clicknload", J::Bool(s.clicknload)),
        ("proxy", js(&s.proxy)),
    ])
}
fn clamp(v: Option<i64>, lo: i64, hi: i64, def: i64) -> i64 {
    v.map(|x| x.max(lo).min(hi)).unwrap_or(def)
}
fn load_state(data_dir: &Path) -> State {
    let mut s = State {
        active: true,
        settings: Settings {
            dir: home_dir().join("Downloads").join("Ladewerk").to_string_lossy().to_string(),
            max_parallel: 3,
            chunks: 4,
            speed_limit_kb: 0,
            retries: 3,
            extract: true,
            delete_archives: false,
            passwords: Vec::new(),
            clipboard: true,
            clicknload: true,
            proxy: String::new(),
        },
        items: Vec::new(),
        accounts: Vec::new(),
        pkg_passwords: HashMap::new(),
    };
    let Ok(raw) = fs::read(data_dir.join("state.json")) else { return s };
    let Ok(j) = parse_json(&raw) else { return s };
    if let Some(b) = j.get("active").bool() {
        s.active = b;
    }
    let se = j.get("settings");
    if let Some(d) = se.get("dir").str().filter(|d| Path::new(d).is_absolute()) {
        s.settings.dir = d.to_string();
    }
    s.settings.max_parallel = clamp(se.get("maxParallel").int(), 1, 10, 3) as usize;
    s.settings.chunks = clamp(se.get("chunks").int(), 1, 8, 4) as usize;
    s.settings.speed_limit_kb = clamp(se.get("speedLimitKB").int(), 0, 10_000_000, 0) as u64;
    s.settings.retries = clamp(se.get("retries").int(), 0, 10, 3) as u32;
    s.settings.extract = se.get("extract").bool().unwrap_or(true);
    s.settings.delete_archives = se.get("deleteArchives").bool().unwrap_or(false);
    s.settings.passwords = se.get("passwords").arr().iter().filter_map(|p| p.str()).map(String::from).collect();
    s.settings.clipboard = se.get("clipboard").bool().unwrap_or(true);
    s.settings.clicknload = se.get("clicknload").bool().unwrap_or(true);
    s.settings.proxy = se.get("proxy").string();
    if check_proxy(&s.settings.proxy).is_err() {
        s.settings.proxy.clear();
    }
    set_proxy(&s.settings.proxy);
    if let J::Obj(m) = j.get("pkgPasswords") {
        for (k, v) in m {
            let list: Vec<String> = v.arr().iter().filter_map(|x| x.str()).map(String::from).collect();
            if !list.is_empty() {
                s.pkg_passwords.insert(k.clone(), list);
            }
        }
    }
    for a in j.get("accounts").arr() {
        let (host, user, secret) = (a.get("host").string(), a.get("user").string(), a.get("secret").string());
        if !host.is_empty() && !user.is_empty() && !secret.is_empty() {
            s.accounts.push(Account { host, user, secret, token: None, ok: None, info: String::new() });
        }
    }
    for it in j.get("items").arr() {
        let mut item = Item {
            id: it.get("id").string(),
            url: it.get("url").string(),
            name: it.get("name").string(),
            pkg: it.get("pkg").string(),
            stage: it.get("stage").string(),
            status: it.get("status").string(),
            error: it.get("error").str().map(String::from),
            size: it.get("size").u64(),
            probed: it.get("probed").bool().unwrap_or(false),
            resumable: it.get("resumable").bool().unwrap_or(false),
            dir: it.get("dir").string(),
            segs: Vec::new(),
            tries: 0,
            wait_until: 0,
            extract: it.get("extract").string(),
            extract_msg: it.get("extractMsg").string(),
            sha256: it.get("sha256").str().and_then(parse_sha256).unwrap_or_default(),
            sha_state: it.get("shaState").string(),
            sha_got: it.get("shaGot").string(),
        };
        if item.sha_state == "running" {
            item.sha_state.clear();
        }
        if item.id.is_empty() || !is_http(&item.url) || (item.stage != "grab" && item.stage != "queue") {
            continue;
        }
        let mut ok = true;
        for g in it.get("segs").arr() {
            let (start, pos) = (g.get("start").u64().unwrap_or(0), g.get("pos").u64().unwrap_or(0));
            if pos < start {
                ok = false;
            }
            item.segs.push(Seg { start, end: g.get("end").u64(), pos, fin: g.get("fin").bool().unwrap_or(false) });
        }
        if !ok {
            item.segs.clear();
        }
        if item.status == "running" {
            item.status = "queued".into();
        }
        if item.status == "checking" {
            item.status = "offline".into();
            item.error = Some("Prüfung abgebrochen".into());
        }
        s.items.push(item);
    }
    s
}
fn save_now() {
    let text = state_json(&st()).text();
    let dir = &app().data_dir;
    let _ = fs::create_dir_all(dir);
    let tmp = dir.join("state.json.tmp");
    if fs::write(&tmp, text).is_ok() {
        let _ = fs::rename(&tmp, dir.join("state.json"));
    }
}

/* ---------- Text-/URL-Hilfen ---------- */
fn is_http(u: &str) -> bool {
    let l = u.to_ascii_lowercase();
    let rest = l.strip_prefix("http://").or_else(|| l.strip_prefix("https://"));
    matches!(rest, Some(r) if !r.is_empty() && !r.starts_with('/') && !r.starts_with('?') && !r.starts_with('#'))
}
/// Leerzeichen, Steuerzeichen und Nicht-ASCII so kodieren, dass curl die Adresse versteht.
fn encode_url(u: &str) -> String {
    let mut o = String::new();
    for b in u.bytes() {
        if b <= 0x20 || b >= 0x7f || matches!(b, b'"' | b'<' | b'>' | b'`' | b'\\' | b'{' | b'}' | b'|' | b'^') {
            o.push_str(&format!("%{:02X}", b));
        } else {
            o.push(b as char);
        }
    }
    o
}
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut o = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&String::from_utf8_lossy(&b[i + 1..i + 3]), 16) {
                o.push(v);
                i += 3;
                continue;
            }
        }
        o.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&o).to_string()
}
fn sanitize(name: &str, fallback: &str) -> String {
    let mut n: String = name
        .chars()
        .map(|c| if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || (c as u32) < 0x20 { '_' } else { c })
        .collect();
    n = n.trim_matches(|c: char| c == '.' || c.is_whitespace()).to_string();
    if n.chars().count() > 180 {
        let ext: String = match n.rfind('.') {
            Some(i) if n.len() - i <= 12 => n[i..].to_string(),
            _ => String::new(),
        };
        let keep = 180 - ext.chars().count();
        n = n.chars().take(keep).collect::<String>() + &ext;
    }
    let stem = n.split('.').next().unwrap_or("").to_ascii_lowercase();
    let reserved = matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || ((stem.starts_with("com") || stem.starts_with("lpt")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        n = format!("_{}", n);
    }
    if n.is_empty() { fallback.to_string() } else { n }
}
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let (base, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let mut p = dir.join(name);
    let mut i = 2;
    while p.exists() {
        p = dir.join(format!("{} ({}){}", base, i, ext));
        i += 1;
    }
    p
}
fn url_path(u: &str) -> &str {
    let rest = u.split_once("://").map(|x| x.1).unwrap_or(u);
    let rest = &rest[..rest.find(|c| c == '?' || c == '#').unwrap_or(rest.len())];
    match rest.find('/') {
        Some(i) => &rest[i..],
        None => "",
    }
}
fn name_from_url(u: &str) -> String {
    let last = url_path(u).rsplit('/').find(|s| !s.is_empty()).unwrap_or("");
    percent_decode(last)
}
fn extract_urls(text: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    let lb = lower.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lb.len() {
        let Some(off) = lower[i..].find("http") else { break };
        let s = i + off;
        let rest = &lower[s..];
        if !(rest.starts_with("http://") || rest.starts_with("https://")) {
            i = s + 4;
            continue;
        }
        let mut e = s;
        for (k, c) in text[s..].char_indices() {
            if c.is_whitespace() || c.is_control() || matches!(c, '"' | '\'' | '<' | '>' | '`') {
                break;
            }
            e = s + k + c.len_utf8();
        }
        let u = text[s..e].trim_end_matches(|c| matches!(c, ')' | ',' | '.' | ';' | ':' | '!' | '?' | ']'));
        if is_http(u) {
            let u = encode_url(u);
            if !out.contains(&u) {
                out.push(u);
            }
        }
        i = e.max(s + 4);
    }
    out
}
/// Relativen Link gegen die Seitenadresse auflösen.
fn resolve(base: &str, rel: &str) -> Option<String> {
    let rel = rel.trim();
    let rel = &rel[..rel.find('#').unwrap_or(rel.len())];
    if rel.is_empty() {
        return None;
    }
    if is_http(rel) {
        return Some(rel.to_string());
    }
    let (scheme, rest) = base.split_once("://")?;
    if rel.starts_with("//") {
        return Some(format!("{}:{}", scheme, rel));
    }
    // andere Schemata (mailto:, javascript:, data: ...) überspringen
    if let Some(c) = rel.find(':') {
        if !rel[..c].contains('/') && !rel[..c].contains('?') {
            return None;
        }
    }
    let cut = rest.find(|c| c == '/' || c == '?' || c == '#').unwrap_or(rest.len());
    let (host, tail) = rest.split_at(cut);
    let path = &tail[..tail.find(|c| c == '?' || c == '#').unwrap_or(tail.len())];
    let path = if path.is_empty() { "/" } else { path };
    let origin = format!("{}://{}", scheme, host);
    if rel.starts_with('?') {
        return Some(format!("{}{}{}", origin, path, rel));
    }
    let (rpath, rquery) = match rel.find('?') {
        Some(i) => rel.split_at(i),
        None => (rel, ""),
    };
    let joined = if rpath.starts_with('/') { rpath.to_string() } else { format!("{}{}", &path[..path.rfind('/').map(|i| i + 1).unwrap_or(0)], rpath) };
    let mut parts: Vec<&str> = Vec::new();
    for p in joined.split('/') {
        match p {
            "." => {}
            ".." => {
                if parts.len() > 1 {
                    parts.pop();
                }
            }
            p => parts.push(p),
        }
    }
    let mut norm = parts.join("/");
    if !norm.starts_with('/') {
        norm.insert(0, '/');
    }
    if (joined.ends_with("/.") || joined.ends_with("/..")) && !norm.ends_with('/') {
        norm.push('/');
    }
    Some(format!("{}{}{}", origin, norm, rquery))
}
fn has_file_ext(u: &str) -> bool {
    let last = url_path(u).rsplit('/').next().unwrap_or("");
    match last.rfind('.') {
        Some(i) => FILE_EXT.contains(&format!(" {} ", last[i + 1..].to_ascii_lowercase())),
        None => false,
    }
}

/* ---------- curl ---------- */
fn curl_cmd() -> Command {
    let mut exe = PathBuf::from("curl");
    if cfg!(windows) {
        // das mit Windows gelieferte curl verwenden, nicht irgendeines aus dem aktuellen Ordner
        let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Windows"));
        let sys = root.join("System32").join("curl.exe");
        exe = if sys.exists() { sys } else { PathBuf::from("curl.exe") };
    }
    let mut c = Command::new(exe);
    c.args([
        "-sS", "-g", "-L", "--max-redirs", "10", "--proto", "=http,https", "--proto-redir", "=http,https",
        "--suppress-connect-headers", "--connect-timeout", "20", "--speed-time", "30", "--speed-limit", "1",
        "-A", UA, "-H", "Accept-Encoding: identity",
    ]);
    let proxy = lk(&PROXY).clone();
    if !proxy.is_empty() {
        c.arg("-x").arg(proxy); // sonst nimmt curl HTTPS_PROXY/ALL_PROXY aus der Umgebung
    }
    c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    no_window(&mut c);
    c
}
/// Verhindert, dass Hilfsprogramme unter Windows ein Konsolenfenster aufblitzen lassen.
fn no_window(c: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}
fn spawn_err(e: std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::NotFound {
        "curl wurde nicht gefunden (gehört zu Windows 10/11, macOS und Linux)".to_string()
    } else {
        format!("curl lässt sich nicht starten: {}", e)
    }
}
fn curl_message(stderr: &str, fallback: &str) -> String {
    let line = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    let line = line.strip_prefix("curl: ").unwrap_or(line);
    let msg = match line.find(") ") {
        Some(i) if line.starts_with('(') => &line[i + 2..],
        _ => line,
    };
    if msg.is_empty() { fallback.to_string() } else { msg.to_string() }
}

struct Headers {
    status: u32,
    map: Vec<(String, String)>,
    location: Option<String>,
}
impl Headers {
    fn get(&self, k: &str) -> Option<&str> {
        self.map.iter().rev().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
    }
}
/// Liest die Kopfzeilen (curl -i) bis zur endgültigen Antwort; Weiterleitungen werden übersprungen.
/// Err("") bedeutet: curl hat ohne Antwort beendet (Grund steht in stderr).
fn read_headers<R: BufRead>(rd: &mut R) -> Result<Headers, String> {
    let mut location = None;
    loop {
        let mut line = Vec::new();
        if rd.read_until(b'\n', &mut line).map_err(|e| e.to_string())? == 0 {
            return Err(String::new());
        }
        let first = String::from_utf8_lossy(&line).trim().to_string();
        if !first.starts_with("HTTP/") {
            return Err("Unerwartete Antwort vom Server".into());
        }
        let status: u32 = first.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        let mut map = Vec::new();
        loop {
            line.clear();
            if rd.read_until(b'\n', &mut line).map_err(|e| e.to_string())? == 0 {
                return Err(String::new());
            }
            let l = String::from_utf8_lossy(&line);
            let l = l.trim_end_matches(|c| c == '\r' || c == '\n');
            if l.is_empty() {
                break;
            }
            if let Some((k, v)) = l.split_once(':') {
                map.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
            }
        }
        let h = Headers { status, map, location: None };
        let loc = h.get("location").map(String::from);
        let redirect = matches!(status, 301 | 302 | 303 | 307 | 308) && loc.is_some();
        if redirect {
            location = loc;
        }
        if redirect || status / 100 == 1 {
            continue;
        }
        return Ok(Headers { location, ..h });
    }
}
fn finish_child(mut child: Child, kill: bool) -> (bool, String) {
    if kill {
        let _ = child.kill();
    }
    let mut err = String::new();
    if !kill {
        if let Some(mut e) = child.stderr.take() {
            let _ = e.read_to_string(&mut err);
        }
    }
    let ok = child.wait().map(|s| s.success()).unwrap_or(false);
    (ok, err)
}
fn head(url: &str, range: bool) -> Result<Headers, String> {
    let mut cmd = curl_cmd();
    if range {
        cmd.args(["-r", "0-0"]);
    }
    cmd.args(["--max-time", "45", "-i"]).arg(url);
    let mut child = cmd.spawn().map_err(spawn_err)?;
    let mut rd = BufReader::new(child.stdout.take().ok_or("keine Ausgabe")?);
    match read_headers(&mut rd) {
        Ok(h) => {
            finish_child(child, true);
            Ok(h)
        }
        Err(e) if e.is_empty() => {
            let (_, err) = finish_child(child, false);
            Err(curl_message(&err, "Keine Antwort vom Server"))
        }
        Err(e) => {
            finish_child(child, true);
            Err(e)
        }
    }
}
fn disposition_name(cd: &str) -> Option<String> {
    let lower = cd.to_ascii_lowercase();
    if let Some(i) = lower.find("filename*=") {
        let v = cd[i + 10..].split(';').next().unwrap_or("").trim();
        if let Some(j) = v.find("''") {
            let n = percent_decode(v[j + 2..].trim_matches('"'));
            if !n.is_empty() {
                return Some(n);
            }
        }
    }
    let mut from = 0;
    while let Some(off) = lower[from..].find("filename=") {
        let i = from + off;
        let v = cd[i + 9..].trim_start();
        let n = if let Some(q) = v.strip_prefix('"') { q.split('"').next().unwrap_or("") } else { v.split(';').next().unwrap_or("").trim() };
        if !n.is_empty() {
            return Some(n.to_string());
        }
        from = i + 9;
    }
    None
}

struct Probe {
    size: Option<u64>,
    resumable: bool,
    name: String,
}
fn probe(url: &str) -> Result<Probe, String> {
    let mut h = head(url, true)?;
    if h.status == 416 {
        h = head(url, false)?;
    }
    if h.status != 200 && h.status != 206 {
        return Err(format!("HTTP {}", h.status));
    }
    let (mut size, mut resumable) = (None, false);
    if h.status == 206 {
        if let Some(n) = h.get("content-range").and_then(|c| c.rsplit('/').next()).and_then(|t| t.trim().parse::<u64>().ok()) {
            size = Some(n);
            resumable = true;
        }
    } else {
        size = h.get("content-length").and_then(|c| c.trim().parse::<u64>().ok());
    }
    let mut name = h.get("content-disposition").and_then(disposition_name).unwrap_or_default();
    if name.is_empty() {
        if let Some(l) = &h.location {
            name = name_from_url(l);
        }
    }
    if name.is_empty() {
        name = name_from_url(url);
    }
    Ok(Probe { size, resumable, name })
}
fn apply_probe(it: &mut Item, p: Probe) {
    it.size = p.size;
    it.resumable = p.resumable;
    it.probed = true;
    if it.name.is_empty() {
        it.name = sanitize(&p.name, &format!("download-{}", it.id));
    }
    // Archive bekommen automatisch ein eigenes Paket (= eigener Ordner), damit Entpacktes beisammen bleibt
    if it.pkg.is_empty() && it.stage == "grab" {
        if let Some((base, _)) = archive_info(&it.name) {
            it.pkg = sanitize(&base, "");
        }
    }
}
/// Prüft einen Link: Hoster mit Konto über deren Schnittstelle, alles andere direkt.
fn probe_any(url: &str) -> Result<Probe, String> {
    match rg_file_id(url) {
        Some(id) => {
            let (name, size) = rg_info(&id)?;
            Ok(Probe { size, resumable: true, name })
        }
        None => probe(url),
    }
}

/* ---------- Tempolimit (global) ---------- */
fn throttle(n: usize) {
    let lim = app().limit.load(SeqCst);
    if lim == 0 {
        return;
    }
    let now = now_ms() as f64;
    let wait = {
        let mut nf = lk(&app().next_free);
        *nf = nf.max(now) + n as f64 / lim as f64 * 1000.0;
        *nf - now
    };
    if wait > 4.0 {
        thread::sleep(Duration::from_millis(wait.min(5000.0) as u64));
    }
}

/* ---------- Download-Kern ---------- */
fn plan_segments(size: Option<u64>, resumable: bool, chunks: usize) -> Vec<Seg> {
    let single = || vec![Seg { start: 0, end: size.filter(|s| *s > 0).map(|s| s - 1), pos: 0, fin: false }];
    let Some(size) = size else { return single() };
    if !resumable || size < 2 * MB {
        return single();
    }
    let n = (chunks as u64).min(size / MB).max(1);
    if n == 1 {
        return single();
    }
    let step = size.div_ceil(n);
    (0..n)
        .map(|i| {
            let start = i * step;
            Seg { start, end: Some((start + step - 1).min(size - 1)), pos: start, fin: false }
        })
        .filter(|s| s.start < size)
        .collect()
}

enum Got {
    Complete,
    Eof,
}

fn fetch_segment(id: &str, idx: usize, url: &str, resumable: bool, part: &Path, rt: &Rt) -> Result<(), String> {
    let (start, end, mut pos) = with_item(id, |it| it.segs.get(idx).map(|s| (s.start, s.end, s.pos))).flatten().ok_or(ABORT)?;
    if !resumable {
        pos = start;
        with_item(id, |it| {
            it.segs[idx].pos = start;
            it.segs[idx].fin = false;
        });
    }
    let mut cmd = curl_cmd();
    if resumable {
        cmd.arg("-r").arg(format!("{}-{}", pos, end.map(|e| e.to_string()).unwrap_or_default()));
    }
    cmd.arg("-i").arg(url);
    let mut child = cmd.spawn().map_err(spawn_err)?;
    let stdout = child.stdout.take().ok_or("keine Ausgabe")?;
    let mut stderr = child.stderr.take();
    let child = Arc::new(Mutex::new(child));
    lk(&rt.children).push(child.clone());
    rt.conns.fetch_add(1, SeqCst);
    if rt.halt() {
        let _ = lk(&child).kill();
    }

    let work = (|| -> Result<Got, String> {
        let mut rd = BufReader::with_capacity(64 * 1024, stdout);
        let h = read_headers(&mut rd)?;
        if resumable && h.status == 200 {
            return Err(NORANGE.into());
        }
        if h.status != if resumable { 206 } else { 200 } {
            return Err(format!("HTTP {}", h.status));
        }
        let mut file = OpenOptions::new().write(true).open(part).map_err(|e| format!("Datei: {}", e))?;
        file.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let lim = app().limit.load(SeqCst) as usize;
            let want = if lim > 0 { (lim / 4).clamp(2048, buf.len()) } else { buf.len() };
            let mut n = rd.read(&mut buf[..want]).map_err(|e| e.to_string())?;
            if n == 0 {
                return Ok(Got::Eof);
            }
            if rt.halt() {
                return Err(ABORT.into());
            }
            if let Some(e) = end {
                n = n.min((e + 1).saturating_sub(pos) as usize);
            }
            file.write_all(&buf[..n]).map_err(|e| format!("Schreibfehler: {}", e))?;
            pos += n as u64;
            rt.bytes.fetch_add(n as u64, SeqCst);
            if with_item(id, |it| it.segs[idx].pos = pos).is_none() {
                return Err(ABORT.into());
            }
            if matches!(end, Some(e) if pos > e) {
                return Ok(Got::Complete);
            }
            throttle(n);
        }
    })();

    // aufräumen
    let complete = matches!(work, Ok(Got::Complete));
    let hard_error = matches!(&work, Err(e) if !e.is_empty());
    if complete || hard_error {
        let _ = lk(&child).kill();
    }
    let mut err_text = String::new();
    if !complete && !hard_error {
        if let Some(e) = stderr.as_mut() {
            let _ = e.read_to_string(&mut err_text);
        }
    }
    let ok = lk(&child).wait().map(|s| s.success()).unwrap_or(false);
    lk(&rt.children).retain(|c| !Arc::ptr_eq(c, &child));
    rt.conns.fetch_sub(1, SeqCst);

    match work {
        Ok(Got::Complete) => Ok(()),
        Ok(Got::Eof) => {
            if end.is_none() && ok {
                with_item(id, |it| it.segs[idx].fin = true);
                Ok(())
            } else {
                Err(curl_message(&err_text, "Verbindung vorzeitig beendet"))
            }
        }
        Err(e) if e.is_empty() => Err(curl_message(&err_text, "Keine Antwort vom Server")),
        Err(e) => Err(e),
    }
}

fn run_segment(id: &str, idx: usize, url: &str, resumable: bool, part: &Path, rt: &Rt, retries: u32) -> Result<(), String> {
    let mut tries = 0u32;
    loop {
        let Some((before, done)) = with_item(id, |it| it.segs.get(idx).map(|s| (s.pos, s.done()))).flatten() else { return Ok(()) };
        if done || rt.halt() {
            return Ok(());
        }
        if let Err(e) = fetch_segment(id, idx, url, resumable, part, rt) {
            if rt.halt() {
                return Ok(());
            }
            if e == NORANGE {
                return Err(e);
            }
            let after = with_item(id, |it| it.segs[idx].pos).unwrap_or(before);
            if after > before {
                tries = 0;
            }
            tries += 1;
            if tries > retries {
                return Err(e);
            }
            for _ in 0..(15 * tries) {
                if rt.halt() {
                    return Ok(());
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn run_inner(id: &str, rt: &Rt) -> Result<PathBuf, String> {
    let (page_url, probed) = with_item(id, |it| (it.url.clone(), it.probed)).ok_or(ABORT)?;
    if !probed {
        let p = probe_any(&page_url)?;
        with_item(id, |it| apply_probe(it, p));
    }
    // Hoster-Links: bei jedem Start eine frische Direktadresse holen (sie verfällt nach einiger Zeit)
    let hoster = rg_file_id(&page_url);
    let url = match &hoster {
        Some(fid) => rg_download_url(fid)?,
        None => page_url,
    };
    let (dir, name, resumable, n, retries, part) = {
        let mut s = st();
        let (mut chunks, retries) = (s.settings.chunks, s.settings.retries);
        if hoster.is_some() {
            chunks = chunks.min(4);
        }
        let it = s.items.iter_mut().find(|i| i.id == id).ok_or(ABORT)?;
        if it.segs.is_empty() {
            it.segs = plan_segments(it.size, it.resumable, chunks);
        }
        (PathBuf::from(&it.dir), it.name.clone(), it.resumable, it.segs.len(), retries, it.part_path())
    };
    fs::create_dir_all(&dir).map_err(|e| format!("Ordner lässt sich nicht anlegen: {}", e))?;
    *lk(&rt.part) = Some(part.clone());
    let untouched = with_item(id, |it| it.segs.iter().all(|s| s.pos == s.start && !s.fin)).ok_or(ABORT)?;
    if untouched || !part.exists() {
        with_item(id, |it| {
            for s in it.segs.iter_mut() {
                s.pos = s.start;
                s.fin = false;
            }
        });
        fs::File::create(&part).map_err(|e| format!("Datei lässt sich nicht anlegen: {}", e))?;
    }
    if rt.stop.load(SeqCst) {
        return Err(ABORT.into());
    }
    let first_err: Mutex<Option<String>> = Mutex::new(None);
    thread::scope(|sc| {
        for idx in 0..n {
            let (url, part, first_err) = (&url, &part, &first_err);
            sc.spawn(move || {
                if let Err(e) = run_segment(id, idx, url, resumable, part, rt, retries) {
                    let mut fe = lk(first_err);
                    if fe.is_none() {
                        *fe = Some(e);
                    }
                    rt.failed.store(true, SeqCst);
                    rt.kill_all();
                }
            });
        }
    });
    if rt.stop.load(SeqCst) {
        return Err(ABORT.into());
    }
    if let Some(e) = lk(&first_err).take() {
        return Err(e);
    }
    if !with_item(id, |it| it.segs.iter().all(|s| s.done())).unwrap_or(false) {
        return Err(ABORT.into());
    }
    let fin = unique_path(&dir, &name);
    fs::rename(&part, &fin).map_err(|e| format!("Umbenennen fehlgeschlagen: {}", e))?;
    Ok(fin)
}

fn run_item(id: String, rt: Arc<Rt>) {
    let res = run_inner(&id, &rt);
    {
        let mut s = st();
        let retries = s.settings.retries;
        if let Some(it) = s.items.iter_mut().find(|i| i.id == id) {
            match res {
                Ok(fin) => {
                    if let Some(n) = fin.file_name() {
                        it.name = n.to_string_lossy().to_string();
                    }
                    if it.size.is_none() {
                        it.size = Some(it.segs.iter().map(|g| g.pos - g.start).sum());
                    }
                    it.status = "done".into();
                    it.error = None;
                }
                Err(e) => {
                    if rt.stop.load(SeqCst) {
                        it.status = lk(&rt.stop_status).to_string();
                    } else if e == NORANGE {
                        it.resumable = false;
                        it.segs = plan_segments(it.size, false, 1);
                        it.status = "queued".into();
                    } else {
                        it.tries += 1;
                        it.error = Some(e);
                        if it.tries <= retries {
                            it.status = "queued".into();
                            it.wait_until = now_ms() + 3000;
                        } else {
                            it.status = "error".into();
                            it.tries = 0;
                        }
                    }
                }
            }
        }
    }
    if rt.removed.load(SeqCst) {
        if let Some(p) = lk(&rt.part).take() {
            let _ = fs::remove_file(p);
        }
    }
    lk(&app().running).remove(&id);
    mark_dirty();
    // mit erwarteter Prüfsumme erst prüfen; entpackt wird nur, wenn sie stimmt
    match with_item(&id, |it| (it.status == "done", !it.sha256.is_empty())) {
        Some((true, true)) => queue_verify(&id),
        _ => queue_extract(&id, false),
    }
}

fn schedule() {
    let mut s = st();
    if !s.active {
        return;
    }
    let max = s.settings.max_parallel;
    let now = now_ms();
    let mut running = lk(&app().running);
    for it in s.items.iter_mut() {
        if running.len() >= max {
            break;
        }
        if it.stage == "queue" && it.status == "queued" && !running.contains_key(&it.id) && it.wait_until <= now {
            it.status = "running".into();
            let rt = Arc::new(Rt::new());
            running.insert(it.id.clone(), rt.clone());
            let id = it.id.clone();
            thread::spawn(move || run_item(id, rt));
        }
    }
}
fn rt_of(id: &str) -> Option<Arc<Rt>> {
    lk(&app().running).get(id).cloned()
}
fn stop_running(rt: &Rt, status: &'static str) {
    *lk(&rt.stop_status) = status;
    rt.stop.store(true, SeqCst);
    rt.kill_all();
}

/* ---------- Linksammler ---------- */
fn scan_page(url: &str) -> Result<Vec<String>, String> {
    let mut cmd = curl_cmd();
    cmd.args(["--max-time", "40", "--max-filesize", "5000000", "-w", "\n@@LW@@%{http_code}\n%{content_type}\n%{url_effective}"]).arg(url);
    let out = cmd.output().map_err(spawn_err)?;
    let marker = b"\n@@LW@@";
    let Some(pos) = out.stdout.windows(marker.len()).rposition(|w| w == marker) else {
        return Err(curl_message(&String::from_utf8_lossy(&out.stderr), "Keine Antwort vom Server"));
    };
    let meta = String::from_utf8_lossy(&out.stdout[pos + marker.len()..]).to_string();
    let mut m = meta.lines();
    let code = m.next().unwrap_or("").trim().to_string();
    let ctype = m.next().unwrap_or("").to_ascii_lowercase();
    let fin = m.next().unwrap_or(url).trim().to_string();
    if code == "000" || !out.status.success() && code != "200" {
        return Err(curl_message(&String::from_utf8_lossy(&out.stderr), "Seite nicht lesbar"));
    }
    if code != "200" {
        return Err(format!("HTTP {}", code));
    }
    if !(ctype.contains("html") || ctype.contains("xml") || ctype.contains("text/plain")) {
        return Ok(vec![url.to_string()]); // der Link ist selbst schon eine Datei
    }
    let body = String::from_utf8_lossy(&out.stdout[..pos]).to_string();
    let base = if is_http(&fin) { fin } else { url.to_string() };
    let mut found: Vec<String> = Vec::new();
    let mut add = |raw: &str| {
        if let Some(u) = resolve(&base, &raw.replace("&amp;", "&")) {
            let u = encode_url(&u);
            if is_http(&u) && has_file_ext(&u) && !found.contains(&u) {
                found.push(u);
            }
        }
    };
    let lower = body.to_ascii_lowercase();
    let lb = lower.as_bytes();
    for key in ["href", "src"] {
        let mut i = 0;
        while let Some(off) = lower[i..].find(key) {
            let mut j = i + off + key.len();
            i = j;
            while j < lb.len() && lb[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= lb.len() || lb[j] != b'=' {
                continue;
            }
            j += 1;
            while j < lb.len() && lb[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= lb.len() || (lb[j] != b'"' && lb[j] != b'\'') {
                continue;
            }
            let q = lb[j];
            let s = j + 1;
            if let Some(len) = lb[s..].iter().position(|c| *c == q) {
                if len < 4000 {
                    add(&body[s..s + len]);
                }
                i = s + len;
            }
        }
    }
    for u in extract_urls(&body) {
        add(&u);
    }
    Ok(found)
}

fn probe_item(id: String) {
    let Some(url) = with_item(&id, |it| it.url.clone()) else { return };
    let res = probe_any(&url);
    with_item(&id, |it| match res {
        Ok(p) => {
            apply_probe(it, p);
            if it.stage == "grab" {
                it.status = "ready".into();
            }
            it.error = None;
        }
        Err(e) => {
            if it.stage == "grab" {
                it.status = "offline".into();
            }
            it.error = Some(e);
        }
    });
    mark_dirty();
}
/// Arbeitet eine Liste mit höchstens `n` Threads gleichzeitig ab.
fn pool<T: Send + 'static>(list: Vec<T>, n: usize, f: fn(T)) {
    let q = Arc::new(Mutex::new(list));
    for _ in 0..n.min(lk(&q).len()) {
        let q = q.clone();
        thread::spawn(move || loop {
            let next = lk(&q).pop();
            match next {
                Some(x) => f(x),
                None => break,
            }
        });
    }
}

fn grab(text: &str, pkg: &str, scan: bool) -> J {
    let with_hash = extract_links_hash(text);
    let mut urls: Vec<String> = with_hash.iter().map(|(u, _)| u.clone()).collect();
    let mut notes: Vec<J> = Vec::new();
    if scan {
        let results: Mutex<Vec<(usize, Result<Vec<String>, String>)>> = Mutex::new(Vec::new());
        let next = AtomicUsize::new(0);
        thread::scope(|sc| {
            for _ in 0..urls.len().min(4) {
                sc.spawn(|| loop {
                    let i = next.fetch_add(1, SeqCst);
                    if i >= urls.len() {
                        break;
                    }
                    let r = scan_page(&urls[i]);
                    lk(&results).push((i, r));
                });
            }
        });
        let mut res = results.into_inner().unwrap_or_else(|e| e.into_inner());
        res.sort_by_key(|r| r.0);
        let mut all: Vec<String> = Vec::new();
        for (i, r) in res {
            match r {
                Ok(list) => {
                    for u in list {
                        if !all.contains(&u) {
                            all.push(u);
                        }
                    }
                }
                Err(e) => notes.push(J::Str(format!("{}: {}", urls[i], e))),
            }
        }
        urls = all;
    }
    let found = urls.len();
    let hashes: HashMap<String, String> = with_hash.into_iter().filter_map(|(u, h)| h.map(|h| (u, h))).collect();
    let added = add_links(urls.into_iter().map(|u| { let h = hashes.get(&u).cloned(); (u, h) }).collect(), pkg);
    obj(vec![("found", jn(found as u64)), ("added", jn(added as u64)), ("duplicates", jn((found - added) as u64)), ("notes", J::Arr(notes))])
}
/// Nimmt neue Links in den Sammler auf und prüft sie im Hintergrund. Liefert die Anzahl neuer Einträge.
fn add_urls(urls: Vec<String>, pkg: &str) -> usize {
    add_links(urls.into_iter().map(|u| (u, None)).collect(), pkg)
}
/// Wie add_urls, je Link optional mit erwarteter SHA-256-Prüfsumme.
fn add_links(links: Vec<(String, Option<String>)>, pkg: &str) -> usize {
    let pkg = sanitize(pkg, "");
    let mut ids = Vec::new();
    {
        let mut s = st();
        for (url, hash) in links {
            if s.items.iter().any(|i| i.url == url) {
                continue;
            }
            let id = new_id();
            ids.push(id.clone());
            s.items.push(Item {
                id,
                url,
                name: String::new(),
                pkg: pkg.clone(),
                stage: "grab".into(),
                status: "checking".into(),
                error: None,
                size: None,
                probed: false,
                resumable: false,
                dir: String::new(),
                segs: Vec::new(),
                tries: 0,
                wait_until: 0,
                extract: String::new(),
                extract_msg: String::new(),
                sha256: hash.unwrap_or_default(),
                sha_state: String::new(),
                sha_got: String::new(),
            });
        }
    }
    let added = ids.len();
    ids.reverse(); // pool() nimmt von hinten
    pool(ids, 6, probe_item);
    mark_dirty();
    added
}

/* ---------- Windows-Systemaufrufe (Passwortschutz, Zwischenablage) ---------- */
#[cfg(windows)]
mod sys {
    use std::ffi::c_void;
    #[repr(C)]
    pub struct Blob {
        pub cb: u32,
        pub pb: *mut u8,
    }
    #[link(name = "crypt32")]
    extern "system" {
        pub fn CryptProtectData(input: *const Blob, descr: *const u16, entropy: *const Blob, reserved: *mut c_void, prompt: *const c_void, flags: u32, out: *mut Blob) -> i32;
        pub fn CryptUnprotectData(input: *const Blob, descr: *mut *mut u16, entropy: *const Blob, reserved: *mut c_void, prompt: *const c_void, flags: u32, out: *mut Blob) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        pub fn LocalFree(h: *mut c_void) -> *mut c_void;
        pub fn GlobalLock(h: *mut c_void) -> *mut c_void;
        pub fn GlobalUnlock(h: *mut c_void) -> i32;
        pub fn GlobalSize(h: *mut c_void) -> usize;
    }
    #[link(name = "user32")]
    extern "system" {
        pub fn OpenClipboard(hwnd: *mut c_void) -> i32;
        pub fn CloseClipboard() -> i32;
        pub fn GetClipboardData(format: u32) -> *mut c_void;
        pub fn GetClipboardSequenceNumber() -> u32;
        pub fn IsClipboardFormatAvailable(format: u32) -> i32;
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}
fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()).collect()
}

/// Passwort für die Ablage schützen. Windows: DPAPI – nur dieses Windows-Benutzerkonto kann es wieder lesen.
#[cfg(windows)]
fn protect(plain: &str) -> String {
    use std::ptr::{null, null_mut};
    unsafe {
        let input = sys::Blob { cb: plain.len() as u32, pb: plain.as_ptr() as *mut u8 };
        let mut out = sys::Blob { cb: 0, pb: null_mut() };
        if sys::CryptProtectData(&input, null(), null(), null_mut(), null(), 1, &mut out) != 0 && !out.pb.is_null() {
            let v = std::slice::from_raw_parts(out.pb, out.cb as usize).to_vec();
            sys::LocalFree(out.pb as *mut std::ffi::c_void);
            return format!("dpapi:{}", hex(&v));
        }
    }
    format!("plain:{}", hex(plain.as_bytes()))
}
#[cfg(not(windows))]
fn protect(plain: &str) -> String {
    format!("plain:{}", hex(plain.as_bytes()))
}
fn unprotect(secret: &str) -> Option<String> {
    if let Some(h) = secret.strip_prefix("plain:") {
        return String::from_utf8(unhex(h)?).ok();
    }
    #[cfg(windows)]
    if let Some(h) = secret.strip_prefix("dpapi:") {
        use std::ptr::{null, null_mut};
        let mut data = unhex(h)?;
        unsafe {
            let input = sys::Blob { cb: data.len() as u32, pb: data.as_mut_ptr() };
            let mut out = sys::Blob { cb: 0, pb: null_mut() };
            if sys::CryptUnprotectData(&input, null_mut(), null(), null_mut(), null(), 1, &mut out) != 0 && !out.pb.is_null() {
                let v = std::slice::from_raw_parts(out.pb, out.cb as usize).to_vec();
                sys::LocalFree(out.pb as *mut std::ffi::c_void);
                return String::from_utf8(v).ok();
            }
        }
    }
    None
}

static CLIP_SEQ: AtomicU64 = AtomicU64::new(u64::MAX);
/// Liefert den Text der Zwischenablage, wenn er sich seit dem letzten Aufruf geändert hat.
/// Mit `only_sync` wird nur der Stand gemerkt (nichts gelesen).
#[cfg(windows)]
fn clipboard_text(only_sync: bool) -> Option<String> {
    unsafe {
        let seq = sys::GetClipboardSequenceNumber() as u64;
        if CLIP_SEQ.swap(seq, SeqCst) == seq || only_sync {
            return None;
        }
        if sys::IsClipboardFormatAvailable(13) == 0 {
            return None; // kein Text (CF_UNICODETEXT)
        }
        if sys::OpenClipboard(std::ptr::null_mut()) == 0 {
            CLIP_SEQ.store(u64::MAX, SeqCst); // gerade belegt – beim nächsten Mal nochmal versuchen
            return None;
        }
        let mut text = None;
        let h = sys::GetClipboardData(13);
        if !h.is_null() {
            let p = sys::GlobalLock(h) as *const u16;
            if !p.is_null() {
                let max = (sys::GlobalSize(h) / 2).min(400_000);
                let mut n = 0;
                while n < max && *p.add(n) != 0 {
                    n += 1;
                }
                text = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, n)));
                sys::GlobalUnlock(h);
            }
        }
        sys::CloseClipboard();
        text
    }
}
#[cfg(not(windows))]
fn clipboard_text(_only_sync: bool) -> Option<String> {
    // nur zum Testen ausserhalb von Windows: Datei als «Zwischenablage»
    let p = std::env::var_os("LADEWERK_CLIPFILE")?;
    let t = fs::read_to_string(p).ok()?;
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.hash(&mut h);
    let seq = h.finish() >> 1;
    if CLIP_SEQ.swap(seq, SeqCst) == seq || _only_sync {
        return None;
    }
    Some(t)
}
fn clipboard_loop() {
    clipboard_text(true);
    loop {
        thread::sleep(Duration::from_millis(1000));
        let on = st().settings.clipboard;
        let Some(text) = clipboard_text(!on) else { continue };
        // nur Datei-Links und unterstützte Hoster, keine gewöhnlichen Webseiten
        let urls: Vec<String> = extract_urls(&text).into_iter().filter(|u| has_file_ext(u) || rg_file_id(u).is_some()).collect();
        if !urls.is_empty() {
            add_urls(urls, "");
        }
    }
}

/* ---------- Hoster: Rapidgator (offizielle Schnittstelle, Premium-Konto) ---------- */
const NO_RG_ACCOUNT: &str = "Rapidgator-Konto fehlt – bitte unter Einstellungen → Konten eintragen";

fn url_enc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{:02X}", b));
        }
    }
    o
}
fn host_of(u: &str) -> String {
    let rest = u.split_once("://").map(|x| x.1).unwrap_or(u);
    let host = &rest[..rest.find(|c| c == '/' || c == '?' || c == '#').unwrap_or(rest.len())];
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
    host.strip_prefix("www.").map(String::from).unwrap_or(host)
}
/// Datei-Kennung aus einem Rapidgator-Link (https://rapidgator.net/file/<id>/name.html).
fn rg_file_id(u: &str) -> Option<String> {
    if !matches!(host_of(u).as_str(), "rapidgator.net" | "rg.to" | "rapidgator.asia") {
        return None;
    }
    let mut seg = url_path(u).split('/').filter(|s| !s.is_empty());
    if seg.next()? != "file" {
        return None;
    }
    let id = seg.next()?;
    if !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric()) { Some(id.to_string()) } else { None }
}
/// GET auf eine JSON-Schnittstelle. Die Adresse geht über die Standardeingabe an curl,
/// damit Zugangsdaten nicht in der Prozessliste auftauchen.
fn api_get(url: &str) -> Result<J, String> {
    let mut cmd = curl_cmd();
    cmd.args(["--max-time", "45", "-K", "-"]).stdin(Stdio::piped());
    let mut child = cmd.spawn().map_err(spawn_err)?;
    if let Some(mut i) = child.stdin.take() {
        let _ = i.write_all(format!("url = \"{}\"\n", url).as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.stdout.iter().all(|b| b.is_ascii_whitespace()) {
        return Err(curl_message(&String::from_utf8_lossy(&out.stderr), "Keine Antwort vom Hoster"));
    }
    parse_json(&out.stdout).map_err(|_| "Unerwartete Antwort vom Hoster".to_string())
}
fn truthy(j: &J) -> bool {
    match j {
        J::Bool(b) => *b,
        J::Num(n) => *n != 0.0,
        J::Str(s) => s == "1" || s == "true",
        _ => false,
    }
}
struct ApiErr {
    code: i64,
    msg: String,
}
fn rg_call(path: &str, query: &str) -> Result<J, ApiErr> {
    let api = std::env::var("LADEWERK_RG_API").unwrap_or_else(|_| "https://rapidgator.net/api/v2".to_string());
    let j = api_get(&format!("{}/{}?{}", api, path, query)).map_err(|msg| ApiErr { code: 0, msg })?;
    let code = j.get("status").int().unwrap_or(0);
    if code == 200 {
        return Ok(j.get("response").clone());
    }
    let d = j.get("details").str().unwrap_or("").trim().to_string();
    Err(ApiErr { code, msg: if d.is_empty() { format!("Rapidgator meldet Fehler {}", code) } else { format!("Rapidgator: {}", d) } })
}
fn set_account(host: &str, f: impl FnOnce(&mut Account)) {
    if let Some(a) = st().accounts.iter_mut().find(|a| a.host == host) {
        f(a);
    }
}
/// Sitzung holen; `fresh` erzwingt eine neue Anmeldung.
fn rg_token(fresh: bool) -> Result<String, String> {
    let (user, secret, token) = {
        let s = st();
        let a = s.accounts.iter().find(|a| a.host == "rapidgator.net").ok_or(NO_RG_ACCOUNT)?;
        (a.user.clone(), a.secret.clone(), a.token.clone())
    };
    if !fresh {
        if let Some(t) = token {
            return Ok(t);
        }
    }
    let fail = |msg: String| -> Result<String, String> {
        set_account("rapidgator.net", |a| {
            a.token = None;
            a.ok = Some(false);
            a.info = msg.clone();
        });
        Err(msg)
    };
    let Some(pw) = unprotect(&secret) else { return fail("Gespeichertes Passwort ist nicht lesbar – Konto bitte neu eintragen".into()) };
    let resp = match rg_call("user/login", &format!("login={}&password={}", url_enc(&user), url_enc(&pw))) {
        Ok(r) => r,
        Err(e) => return fail(e.msg),
    };
    let t = resp.get("token").str().or(resp.get("session_id").str()).unwrap_or("").to_string();
    if t.is_empty() {
        return fail("Rapidgator-Anmeldung: keine Sitzung erhalten".into());
    }
    let u = resp.get("user");
    let premium = truthy(u.get("is_premium"));
    let mut info = if premium { "Premium".to_string() } else { "Angemeldet, aber kein Premium".to_string() };
    if let Some(end) = u.get("premium_end_time").u64().filter(|e| *e > now_ms() / 1000) {
        info.push_str(&format!(", noch {} Tage", (end - now_ms() / 1000) / 86400));
    }
    if let Some(left) = u.get("traffic").get("left").u64() {
        info.push_str(&format!(", {:.1} GB Volumen übrig", left as f64 / 1073741824.0));
    }
    set_account("rapidgator.net", |a| {
        a.token = Some(t.clone());
        a.ok = Some(premium);
        a.info = info;
    });
    Ok(t)
}
fn rg_authed(path: &str, extra: &str) -> Result<J, String> {
    let t = rg_token(false)?;
    match rg_call(path, &format!("token={}&{}", url_enc(&t), extra)) {
        Ok(j) => Ok(j),
        Err(e) if e.code == 401 || e.msg.to_ascii_lowercase().contains("session") => {
            let t = rg_token(true)?; // Sitzung abgelaufen – einmal neu anmelden
            rg_call(path, &format!("token={}&{}", url_enc(&t), extra)).map_err(|e| e.msg)
        }
        Err(e) => Err(e.msg),
    }
}
fn rg_info(file_id: &str) -> Result<(String, Option<u64>), String> {
    let r = rg_authed("file/info", &format!("file_id={}", url_enc(file_id)))?;
    let f = r.get("file");
    let name = f.get("name").string();
    if name.is_empty() {
        return Err("Rapidgator: Datei nicht gefunden".into());
    }
    Ok((name, f.get("size").u64()))
}
fn rg_download_url(file_id: &str) -> Result<String, String> {
    let r = rg_authed("file/download", &format!("file_id={}", url_enc(file_id)))?;
    let u = r.get("download_url").str().or(r.get("url").str()).unwrap_or("").to_string();
    if is_http(&u) { Ok(encode_url(&u)) } else { Err("Rapidgator hat keine Download-Adresse geliefert (Premium aktiv?)".into()) }
}

/* ---------- Entpacken ---------- */
/// Erkennt Archive. Liefert (Grundname des Archivs, ist dies der erste Teil?).
fn archive_info(name: &str) -> Option<(String, bool)> {
    let l = name.to_ascii_lowercase();
    let ext_of = |s: &str| s.rfind('.').map(|i| s[i + 1..].to_string()).unwrap_or_default();
    let ext = ext_of(&l);
    let stem = |n: usize| name[..name.len() - n].to_string();
    if ext == "rar" {
        let st = &l[..l.len() - 4];
        if let Some(i) = st.rfind(".part") {
            let num = &st[i + 5..];
            if !num.is_empty() && num.len() <= 4 && num.chars().all(|c| c.is_ascii_digit()) {
                return Some((name[..i].to_string(), num.parse::<u32>() == Ok(1)));
            }
        }
        return Some((stem(4), true));
    }
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    // alte RAR-Teile .r00 .r01 … und geteilte ZIPs .z01 … gehören zum Archiv, sind aber nie der erste Teil
    if ext.len() == 3 && (ext.starts_with('r') || ext.starts_with('z')) && digits(&ext[1..]) {
        return Some((stem(4), false));
    }
    // geteilte Archive: name.7z.001, name.zip.001
    if ext.len() == 3 && digits(&ext) {
        let inner = &l[..l.len() - 4];
        for e in [".7z", ".zip"] {
            if inner.ends_with(e) {
                return Some((name[..inner.len() - e.len()].to_string(), ext == "001"));
            }
        }
        return None;
    }
    for e in [".tar.gz", ".tar.bz2", ".tar.xz", ".tgz", ".zip", ".7z", ".tar"] {
        if l.ends_with(e) && l.len() > e.len() {
            return Some((stem(e.len()), true));
        }
    }
    None
}
fn which(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) { format!("{}.exe", name) } else { name.to_string() };
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(&exe)).find(|p| p.is_file())
}
fn win_program(rel: &str) -> Option<PathBuf> {
    ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)", "LOCALAPPDATA"]
        .iter()
        .filter_map(|e| std::env::var_os(e))
        .map(|d| PathBuf::from(d).join(rel))
        .find(|p| p.is_file())
}
struct Tools {
    seven: Option<PathBuf>,
    unrar: Option<PathBuf>,
    tar: Option<PathBuf>,
    unzip: Option<PathBuf>,
}
fn tools() -> Tools {
    let forced = std::env::var("LADEWERK_EXTRACTOR").unwrap_or_default();
    let mut t = if cfg!(windows) {
        let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Windows"));
        Tools {
            seven: embedded_7z().or_else(|| win_program("7-Zip\\7z.exe")).or_else(|| which("7z")),
            unrar: win_program("WinRAR\\UnRAR.exe").or_else(|| which("unrar")),
            tar: Some(root.join("System32").join("tar.exe")).filter(|p| p.is_file()),
            unzip: None,
        }
    } else {
        Tools { seven: which("7z").or_else(|| which("7zz")).or_else(|| which("7za")), unrar: which("unrar"), tar: which("tar"), unzip: which("unzip") }
    };
    if forced == "tar" {
        t.seven = None;
        t.unrar = None;
    }
    t
}
fn extractor_label() -> String {
    let t = tools();
    if t.seven.is_some() && t.seven == embedded_7z() {
        "7-Zip (eingebaut)".into()
    } else if t.seven.is_some() {
        "7-Zip".into()
    } else if t.unrar.is_some() {
        "WinRAR (RAR) und Windows (ZIP, 7z)".into()
    } else if t.tar.is_some() {
        if cfg!(windows) { "Windows-Bordmittel (für mehrteilige oder verschlüsselte RAR bitte 7-Zip installieren)".into() } else { "tar/unzip".into() }
    } else {
        String::new()
    }
}
fn run_tool(cmd: &mut Command) -> (bool, String) {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    match no_window(cmd).output() {
        Ok(o) => {
            let all = format!("{}\n{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
            (o.status.success(), all)
        }
        Err(e) => (false, format!("Entpacker lässt sich nicht starten: {}", e)),
    }
}
fn last_line(s: &str) -> String {
    let l = s.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("Entpacken fehlgeschlagen");
    l.chars().take(200).collect()
}
fn extract_archive(file: &Path, dest: &Path, passwords: &[String]) -> Result<(), String> {
    let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let l = name.to_ascii_lowercase();
    let is_rar = l.ends_with(".rar");
    let is_zip = l.ends_with(".zip");
    let multi = (is_rar && l[..l.len() - 4].contains(".part")) || l.ends_with(".001");
    let t = tools();
    // (Passwort, nur testen?) -> (Erfolg, Ausgabe)
    let attempt: Box<dyn Fn(&str, bool) -> (bool, String)>;
    let can_test;
    if let Some(z) = t.seven.clone() {
        can_test = true;
        let (file, dest) = (file.to_path_buf(), dest.to_path_buf());
        attempt = Box::new(move |pw, test| {
            let mut c = Command::new(&z);
            if test {
                c.args(["t", "-bd"]);
            } else {
                c.args(["x", "-y", "-aoa", "-bd"]).arg(format!("-o{}", dest.display()));
            }
            c.arg(format!("-p{}", pw)).arg(&file);
            run_tool(&mut c)
        });
    } else if let (true, Some(u)) = (is_rar, t.unrar.clone()) {
        can_test = true;
        let (file, dest) = (file.to_path_buf(), dest.to_path_buf());
        attempt = Box::new(move |pw, test| {
            let mut c = Command::new(&u);
            c.arg(if test { "t" } else { "x" }).args(["-y", "-o+"]);
            c.arg(if pw.is_empty() { "-p-".to_string() } else { format!("-p{}", pw) }).arg(&file);
            if !test {
                let mut d = dest.clone().into_os_string();
                d.push(std::path::MAIN_SEPARATOR.to_string());
                c.arg(d);
            }
            run_tool(&mut c)
        });
    } else if let (true, Some(u)) = (is_zip && !cfg!(windows), t.unzip.clone()) {
        can_test = false;
        let (file, dest) = (file.to_path_buf(), dest.to_path_buf());
        attempt = Box::new(move |pw, _| {
            let mut c = Command::new(&u);
            c.args(["-o", "-q"]);
            if !pw.is_empty() {
                c.arg("-P").arg(pw);
            }
            c.arg(&file).arg("-d").arg(&dest);
            run_tool(&mut c)
        });
    } else if let Some(tar) = t.tar.clone() {
        if multi {
            return Err("Mehrteilige Archive brauchen 7-Zip – bitte installieren (7-zip.org), dann «Entpacken» drücken".into());
        }
        can_test = false;
        let (file, dest) = (file.to_path_buf(), dest.to_path_buf());
        attempt = Box::new(move |pw, _| {
            let mut c = Command::new(&tar);
            c.arg("-xf").arg(&file).arg("-C").arg(&dest);
            if !pw.is_empty() {
                c.arg("--passphrase").arg(pw);
            }
            run_tool(&mut c)
        });
    } else {
        return Err("Kein Entpacker gefunden – bitte 7-Zip installieren (7-zip.org)".into());
    }

    let (ok, out) = attempt("", false);
    if ok {
        return Ok(());
    }
    let lo = out.to_ascii_lowercase();
    if lo.contains("password") || lo.contains("passphrase") || lo.contains("encrypted") {
        for pw in passwords {
            if can_test && !attempt(pw, true).0 {
                continue;
            }
            if attempt(pw, false).0 {
                return Ok(());
            }
        }
        return Err(if passwords.is_empty() {
            "Archiv ist passwortgeschützt – Passwort in den Einstellungen eintragen, dann «Entpacken» drücken".into()
        } else {
            "Keines der gespeicherten Passwörter passt".into()
        });
    }
    if is_rar && t.seven.is_none() && t.unrar.is_none() {
        return Err(format!("{} – für RAR-Archive am besten 7-Zip installieren (7-zip.org)", last_line(&out)));
    }
    Err(last_line(&out))
}
/// Alle Einträge, die zum selben Archiv gehören wie `id` (gleicher Ordner, gleicher Grundname).
fn archive_set(s: &State, id: &str) -> Vec<usize> {
    let Some(it) = s.items.iter().find(|i| i.id == id) else { return Vec::new() };
    let Some((key, _)) = archive_info(&it.name) else { return Vec::new() };
    let key = key.to_lowercase();
    s.items
        .iter()
        .enumerate()
        .filter(|(_, o)| o.stage == "queue" && o.dir == it.dir && matches!(archive_info(&o.name), Some((k, _)) if k.to_lowercase() == key))
        .map(|(i, _)| i)
        .collect()
}
/// Reiht das Archiv zum Entpacken ein, sobald alle seine Teile fertig geladen sind.
fn queue_extract(id: &str, manual: bool) {
    let mut s = st();
    if !manual && !s.settings.extract {
        return;
    }
    let set = archive_set(&s, id);
    if set.is_empty() || set.iter().any(|&i| s.items[i].status != "done") {
        return;
    }
    let Some(&first) = set.iter().find(|&&i| matches!(archive_info(&s.items[i].name), Some((_, true)))) else { return };
    let it = &mut s.items[first];
    if it.extract == "wait" || it.extract == "running" || (!manual && !it.extract.is_empty()) {
        return;
    }
    it.extract = "wait".into();
    it.extract_msg.clear();
    lk(&app().extract_q).push(it.id.clone());
}
fn extract_loop() {
    loop {
        thread::sleep(Duration::from_millis(400));
        let next = {
            let mut q = lk(&app().extract_q);
            if q.is_empty() { None } else { Some(q.remove(0)) }
        };
        let Some(id) = next else { continue };
        let job = {
            let mut s = st();
            let pkg = s.items.iter().find(|i| i.id == id).map(|i| i.pkg.clone()).unwrap_or_default();
            let (pws, del) = (passwords_for(&s, &pkg), s.settings.delete_archives);
            let parts: Vec<PathBuf> = archive_set(&s, &id).iter().map(|&i| Path::new(&s.items[i].dir).join(&s.items[i].name)).collect();
            s.items.iter_mut().find(|i| i.id == id).map(|it| {
                it.extract = "running".into();
                (Path::new(&it.dir).join(&it.name), PathBuf::from(&it.dir), pws, del, parts)
            })
        };
        let Some((file, dest, pws, del, parts)) = job else { continue };
        let res = if file.is_file() { extract_archive(&file, &dest, &pws) } else { Err("Archivdatei nicht mehr vorhanden".into()) };
        let ok = res.is_ok();
        with_item(&id, |it| match res {
            Ok(()) => {
                it.extract = "ok".into();
                it.extract_msg.clear();
            }
            Err(e) => {
                it.extract = "error".into();
                it.extract_msg = e;
            }
        });
        if ok && del {
            for p in parts {
                let _ = fs::remove_file(p);
            }
        }
        mark_dirty();
    }
}

/* ---------- Beenden ---------- */
/// Laufende Downloads anhalten, Stand sichern, beenden.
fn shutdown() -> ! {
    let rts: Vec<Arc<Rt>> = lk(&app().running).values().cloned().collect();
    for rt in &rts {
        stop_running(rt, "queued");
    }
    for _ in 0..20 {
        if lk(&app().running).is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    save_now();
    std::process::exit(0)
}

/* ---------- Click'n'Load (wie JDownloader, Port 9666) ---------- */
// Webseiten mit Click'n'Load-Knopf schicken ihre Links an http://127.0.0.1:9666.
// Die Links landen nur im Linksammler – geladen wird erst, wenn du es bestätigst.
const CNL_PORT: u16 = 9666;
static CNL_STATUS: Mutex<String> = Mutex::new(String::new());

fn cnl_status() -> String {
    lk(&CNL_STATUS).clone()
}
fn set_cnl_status(s: &str) {
    *lk(&CNL_STATUS) = s.to_string();
}

/// AES-128 nur zum Entschlüsseln (CNL2 nutzt AES-128-CBC, IV = Schlüssel).
mod aes {
    const SBOX: [u8; 256] = {
        // S-Box nach FIPS-197, zur Kompilierzeit berechnet
        let mut sbox = [0u8; 256];
        let (mut p, mut q) = (1u8, 1u8);
        loop {
            // p mit 3 multiplizieren
            p = p ^ (p << 1) ^ (if p & 0x80 != 0 { 0x1B } else { 0 });
            // q durch 3 teilen
            q ^= q << 1;
            q ^= q << 2;
            q ^= q << 4;
            if q & 0x80 != 0 {
                q ^= 0x09;
            }
            let x = q ^ q.rotate_left(1) ^ q.rotate_left(2) ^ q.rotate_left(3) ^ q.rotate_left(4);
            sbox[p as usize] = x ^ 0x63;
            if p == 1 {
                break;
            }
        }
        sbox[0] = 0x63;
        sbox
    };
    const INV: [u8; 256] = {
        let mut inv = [0u8; 256];
        let mut i = 0;
        while i < 256 {
            inv[SBOX[i] as usize] = i as u8;
            i += 1;
        }
        inv
    };
    fn xt(b: u8) -> u8 {
        (b << 1) ^ if b & 0x80 != 0 { 0x1B } else { 0 }
    }
    fn mul(mut a: u8, mut b: u8) -> u8 {
        let mut r = 0;
        while b != 0 {
            if b & 1 != 0 {
                r ^= a;
            }
            a = xt(a);
            b >>= 1;
        }
        r
    }
    fn expand(key: &[u8; 16]) -> [[u8; 16]; 11] {
        let mut w = [[0u8; 4]; 44];
        for i in 0..4 {
            w[i].copy_from_slice(&key[4 * i..4 * i + 4]);
        }
        let mut rcon = 1u8;
        for i in 4..44 {
            let mut t = w[i - 1];
            if i % 4 == 0 {
                t = [SBOX[t[1] as usize] ^ rcon, SBOX[t[2] as usize], SBOX[t[3] as usize], SBOX[t[0] as usize]];
                rcon = xt(rcon);
            }
            for j in 0..4 {
                w[i][j] = w[i - 4][j] ^ t[j];
            }
        }
        let mut rk = [[0u8; 16]; 11];
        for r in 0..11 {
            for c in 0..4 {
                rk[r][4 * c..4 * c + 4].copy_from_slice(&w[4 * r + c]);
            }
        }
        rk
    }
    fn decrypt_block(rk: &[[u8; 16]; 11], b: &mut [u8; 16]) {
        let add = |b: &mut [u8; 16], k: &[u8; 16]| {
            for i in 0..16 {
                b[i] ^= k[i];
            }
        };
        let inv_shift_sub = |b: &mut [u8; 16]| {
            let s = *b;
            for c in 0..4 {
                for r in 0..4 {
                    b[4 * ((c + r) % 4) + r] = INV[s[4 * c + r] as usize];
                }
            }
        };
        add(b, &rk[10]);
        for round in (1..10).rev() {
            inv_shift_sub(b);
            add(b, &rk[round]);
            for c in 0..4 {
                let a = [b[4 * c], b[4 * c + 1], b[4 * c + 2], b[4 * c + 3]];
                for r in 0..4 {
                    b[4 * c + r] = mul(a[r], 14) ^ mul(a[(r + 1) % 4], 11) ^ mul(a[(r + 2) % 4], 13) ^ mul(a[(r + 3) % 4], 9);
                }
            }
        }
        inv_shift_sub(b);
        add(b, &rk[0]);
    }
    /// AES-128-CBC entschlüsseln (ohne Polsterung zu entfernen).
    pub fn cbc_decrypt(key: &[u8; 16], iv: &[u8; 16], data: &[u8]) -> Option<Vec<u8>> {
        if data.is_empty() || data.len() % 16 != 0 {
            return None;
        }
        let rk = expand(key);
        let mut prev = *iv;
        let mut out = Vec::with_capacity(data.len());
        for chunk in data.chunks(16) {
            let mut b = [0u8; 16];
            b.copy_from_slice(chunk);
            let c = b;
            decrypt_block(&rk, &mut b);
            for i in 0..16 {
                b[i] ^= prev[i];
            }
            out.extend_from_slice(&b);
            prev = c;
        }
        Some(out)
    }
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b' ' | b'\r' | b'\n' | b'\t' => continue,
            _ => return None,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}
fn form_decode(s: &str) -> String {
    percent_decode(&s.replace('+', " "))
}
fn parse_form(body: &[u8], query: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    let text = String::from_utf8_lossy(body).to_string();
    for part in query.split('&').chain(text.split('&')) {
        if let Some((k, v)) = part.split_once('=') {
            m.insert(form_decode(k), form_decode(v));
        }
    }
    m
}
/// Schlüssel aus dem «jk»-Feld: eine kleine JavaScript-Funktion, die den Schlüssel als Hex-Text liefert.
fn cnl_key(jk: &str) -> Option<[u8; 16]> {
    let hex_str = jk
        .split(|c| c == '\'' || c == '"')
        .map(str::trim)
        .find(|t| t.len() == 32 && t.chars().all(|c| c.is_ascii_hexdigit()))
        .or_else(|| Some(jk.trim()).filter(|t| t.len() == 32 && t.chars().all(|c| c.is_ascii_hexdigit())))?;
    let v = unhex(&hex_str.to_ascii_lowercase())?;
    v.try_into().ok()
}
fn cnl_decrypt(crypted: &str, jk: &str) -> Result<String, String> {
    let key = cnl_key(jk).ok_or("Schlüssel nicht lesbar")?;
    let data = base64_decode(crypted).ok_or("Daten nicht lesbar")?;
    let plain = aes::cbc_decrypt(&key, &key, &data).ok_or("Daten haben falsche Länge")?;
    let end = plain.iter().rposition(|b| *b != 0).map(|i| i + 1).unwrap_or(0);
    Ok(String::from_utf8_lossy(&plain[..end]).to_string())
}
/// Übernimmt Links aus Click'n'Load in den Linksammler.
fn cnl_take(f: &HashMap<String, String>, links: &str) -> usize {
    let urls = extract_urls(links);
    if urls.is_empty() {
        return 0;
    }
    let pkg = sanitize(&f.get("package").or(f.get("source").filter(|s| !s.starts_with("http"))).cloned().unwrap_or_default(), "");
    if let Some(pws) = f.get("passwords").filter(|p| !p.trim().is_empty()) {
        let mut s = st();
        for pw in pws.lines().map(str::trim).filter(|p| !p.is_empty()) {
            add_pkg_password(&mut s, &pkg, pw);
        }
    }
    add_urls(urls, &pkg)
}
fn cnl_route(method: &str, path: &str, body: &[u8]) -> (u16, &'static str, Vec<u8>) {
    let (p, query) = path.split_once('?').unwrap_or((path, ""));
    let text = |code: u16, t: &str| (code, "text/plain; charset=utf-8", t.as_bytes().to_vec());
    if !st().settings.clicknload {
        return text(503, "Click'n'Load ist in Ladewerk ausgeschaltet\r\n");
    }
    match (method, p.trim_end_matches('/')) {
        ("GET", "/jdcheck.js") => (200, "text/javascript", b"jdownloader=true;\nvar version='ladewerk';\n".to_vec()),
        ("GET", "/crossdomain.xml") => (
            200,
            "text/xml",
            b"<?xml version=\"1.0\"?>\n<cross-domain-policy>\n<allow-access-from domain=\"*\" />\n</cross-domain-policy>\n".to_vec(),
        ),
        ("GET", "/flash") | ("GET", "") => text(200, "JDownloader\r\n"),
        (_, "/flash/add") => {
            let f = parse_form(body, query);
            let n = cnl_take(&f, f.get("urls").map(String::as_str).unwrap_or(""));
            text(if n > 0 || f.contains_key("urls") { 200 } else { 400 }, "success\r\n")
        }
        (_, "/flash/addcrypted2") => {
            let f = parse_form(body, query);
            match cnl_decrypt(f.get("crypted").map(String::as_str).unwrap_or(""), f.get("jk").map(String::as_str).unwrap_or("")) {
                Ok(links) => {
                    cnl_take(&f, &links);
                    text(200, "success\r\n")
                }
                Err(e) => text(400, &format!("failed {}\r\n", e)),
            }
        }
        _ => text(404, "Nicht gefunden\r\n"),
    }
}
fn cnl_handle(mut s: TcpStream) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(10)));
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 8192];
    let hend = loop {
        match s.read(&mut tmp) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
        if let Some(i) = find(&buf, b"\r\n\r\n") {
            break i;
        }
        if buf.len() > 64 * 1024 {
            return;
        }
    };
    let head = String::from_utf8_lossy(&buf[..hend]).to_string();
    let mut rl = head.lines().next().unwrap_or("").split(' ');
    let (method, path) = (rl.next().unwrap_or("").to_string(), rl.next().unwrap_or("").to_string());
    let clen: usize = head
        .lines()
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse().ok())
        .unwrap_or(0);
    if clen > 2 * 1024 * 1024 {
        return;
    }
    let mut body = buf[hend + 4..].to_vec();
    while body.len() < clen {
        match s.read(&mut tmp) {
            Ok(0) | Err(_) => return,
            Ok(n) => body.extend_from_slice(&tmp[..n]),
        }
    }
    body.truncate(clen);
    let (code, ctype, out) = cnl_route(&method, &path, &body);
    let hdr = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        code,
        if code == 200 { "OK" } else { "Fehler" },
        ctype,
        out.len()
    );
    let _ = s.write_all(hdr.as_bytes());
    let _ = s.write_all(&out);
}
fn cnl_loop() {
    let port = std::env::var("LADEWERK_CNL_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(CNL_PORT);
    loop {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => {
                set_cnl_status(&format!("bereit (Port {})", port));
                for conn in l.incoming().flatten() {
                    thread::spawn(move || cnl_handle(conn));
                }
            }
            Err(_) => {
                set_cnl_status(&format!("Port {} ist belegt – läuft JDownloader noch? Ladewerk versucht es weiter.", port));
                thread::sleep(Duration::from_secs(10));
            }
        }
    }
}

/* ---------- Eingebautes 7-Zip ---------- */
// Der Windows-Build legt 7z.exe und 7z.dll von 7-zip.org in die Programmdatei
// (rustc --cfg embedded_7z, Pfade in LW_7Z_EXE / LW_7Z_DLL / LW_7Z_LICENSE).
#[cfg(embedded_7z)]
const SEVEN_FILES: &[(&str, &[u8])] = &[
    ("7z.exe", include_bytes!(env!("LW_7Z_EXE"))),
    ("7z.dll", include_bytes!(env!("LW_7Z_DLL"))),
    ("License.txt", include_bytes!(env!("LW_7Z_LICENSE"))),
];
#[cfg(not(embedded_7z))]
const SEVEN_FILES: &[(&str, &[u8])] = &[];

static SEVEN_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
/// Legt das eingebaute 7-Zip beim ersten Gebrauch in den Datenordner und liefert den Pfad zu 7z.exe.
fn embedded_7z() -> Option<PathBuf> {
    SEVEN_PATH
        .get_or_init(|| {
            if SEVEN_FILES.is_empty() {
                return None;
            }
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            for (n, b) in SEVEN_FILES {
                n.hash(&mut h);
                b.len().hash(&mut h);
                b[..b.len().min(4096)].hash(&mut h);
            }
            let dir = app().data_dir.join("7zip").join(format!("{:016x}", h.finish()));
            fs::create_dir_all(&dir).ok()?;
            for (name, bytes) in SEVEN_FILES {
                let p = dir.join(name);
                if fs::metadata(&p).map(|m| m.len() as usize != bytes.len()).unwrap_or(true) {
                    let tmp = dir.join(format!("{}.tmp", name));
                    fs::write(&tmp, bytes).ok()?;
                    fs::rename(&tmp, &p).ok()?;
                }
            }
            Some(dir.join("7z.exe"))
        })
        .clone()
}

/* ---------- SHA-256 (Prüfsumme nach dem Download) ---------- */
mod sha256 {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
        0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
        0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    pub struct Hasher {
        h: [u32; 8],
        buf: [u8; 64],
        len: usize,
        total: u64,
    }
    impl Hasher {
        pub fn new() -> Self {
            Hasher {
                h: [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19],
                buf: [0; 64],
                len: 0,
                total: 0,
            }
        }
        fn block(&mut self, b: &[u8]) {
            let mut w = [0u32; 64];
            for i in 0..16 {
                w[i] = u32::from_be_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]]);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
            }
            let [mut a, mut b2, mut c, mut d, mut e, mut f, mut g, mut h] = self.h;
            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ (!e & g);
                let t1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b2) ^ (a & c) ^ (b2 & c);
                let t2 = s0.wrapping_add(maj);
                h = g;
                g = f;
                f = e;
                e = d.wrapping_add(t1);
                d = c;
                c = b2;
                b2 = a;
                a = t1.wrapping_add(t2);
            }
            for (x, y) in self.h.iter_mut().zip([a, b2, c, d, e, f, g, h]) {
                *x = x.wrapping_add(y);
            }
        }
        pub fn update(&mut self, mut data: &[u8]) {
            self.total += data.len() as u64;
            if self.len > 0 {
                let n = (64 - self.len).min(data.len());
                self.buf[self.len..self.len + n].copy_from_slice(&data[..n]);
                self.len += n;
                data = &data[n..];
                if self.len < 64 {
                    return; // alles im Puffer, Block noch nicht voll
                }
                let b = self.buf;
                self.block(&b);
                self.len = 0;
            }
            while data.len() >= 64 {
                self.block(&data[..64]);
                data = &data[64..];
            }
            self.buf[..data.len()].copy_from_slice(data);
            self.len = data.len();
        }
        pub fn hex(mut self) -> String {
            let bits = self.total * 8;
            self.update(&[0x80]);
            while self.len != 56 {
                self.update(&[0]);
            }
            self.update(&bits.to_be_bytes());
            self.h.iter().map(|x| format!("{:08x}", x)).collect()
        }
    }
}
/// SHA-256 einer Datei (läuft im Hintergrund, auch bei grossen Dateien).
fn sha256_file(p: &Path) -> Result<String, String> {
    let mut f = fs::File::open(p).map_err(|e| format!("Datei lässt sich nicht lesen: {}", e))?;
    let mut h = sha256::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.hex())
}
/// 64 Hex-Zeichen → Prüfsumme in Kleinbuchstaben.
fn parse_sha256(s: &str) -> Option<String> {
    let s = s.trim();
    (s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())).then(|| s.to_ascii_lowercase())
}
/// Links aus einem Text, je mit einer SHA-256-Prüfsumme, falls in derselben Zeile hinter dem
/// Link eine steht (z. B. «https://…/x.iso  sha256: 3a5f…»).
fn extract_links_hash(text: &str) -> Vec<(String, Option<String>)> {
    let mut out: Vec<(String, Option<String>)> = Vec::new();
    for line in text.lines() {
        let urls = extract_urls(line);
        if urls.is_empty() {
            continue;
        }
        // Prüfsummen in der Zeile mit ihrer Position; zugeordnet wird dem letzten Link davor
        let mut hashes: Vec<(usize, String)> = Vec::new();
        let b = line.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i].is_ascii_hexdigit() && (i == 0 || !b[i - 1].is_ascii_alphanumeric()) {
                let mut j = i;
                while j < b.len() && b[j].is_ascii_hexdigit() {
                    j += 1;
                }
                if j - i == 64 && (j == b.len() || !b[j].is_ascii_alphanumeric()) {
                    hashes.push((i, line[i..j].to_ascii_lowercase()));
                }
                i = j.max(i + 1);
            } else {
                i += 1;
            }
        }
        let lower = line.to_ascii_lowercase();
        let mut pos: Vec<(usize, String)> = Vec::new();
        let mut from = 0;
        for u in &urls {
            let head = &u[..u.len().min(12)].to_ascii_lowercase();
            let at = lower[from..].find(head.as_str()).map(|p| p + from).unwrap_or(from);
            pos.push((at, u.clone()));
            from = at + 1;
        }
        for (k, (at, u)) in pos.iter().enumerate() {
            let next = pos.get(k + 1).map(|p| p.0).unwrap_or(usize::MAX);
            let h = hashes.iter().find(|(hp, _)| hp > at && *hp < next).map(|(_, h)| h.clone());
            match out.iter_mut().find(|(x, _)| x == u) {
                Some(e) => {
                    if e.1.is_none() {
                        e.1 = h;
                    }
                }
                None => out.push((u.clone(), h)),
            }
        }
    }
    out
}
/// Reiht die Prüfsummen-Kontrolle eines fertigen Downloads ein.
fn queue_verify(id: &str) {
    let mut q = lk(&app().verify_q);
    if !q.iter().any(|x| x == id) {
        q.push(id.to_string());
    }
}
fn verify_loop() {
    loop {
        thread::sleep(Duration::from_millis(300));
        let next = {
            let mut q = lk(&app().verify_q);
            if q.is_empty() { None } else { Some(q.remove(0)) }
        };
        let Some(id) = next else { continue };
        let Some(path) = with_item(&id, |it| {
            it.sha_state = "running".into();
            Path::new(&it.dir).join(&it.name)
        }) else {
            continue;
        };
        mark_dirty();
        let res = sha256_file(&path);
        let mut good = false;
        with_item(&id, |it| match res {
            Ok(got) => {
                it.sha_got = got.clone();
                if it.sha256.is_empty() {
                    it.sha_state = "info".into();
                    good = true;
                } else if it.sha256 == got {
                    it.sha_state = "ok".into();
                    good = true;
                } else {
                    it.sha_state = "bad".into();
                }
            }
            Err(e) => {
                it.sha_state = "error".into();
                it.sha_got = e;
            }
        });
        mark_dirty();
        if good {
            queue_extract(&id, false);
        }
    }
}
/// Erwartete Prüfsumme eines Eintrags setzen (leer = keine) und bei fertigen Dateien gleich prüfen.
fn set_hash(id: &str, hash: &str) -> Result<(), String> {
    let h = hash.trim();
    let h = if h.is_empty() {
        String::new()
    } else {
        // «sha256:» o. ä. davor zulassen
        let last = h.split(|c: char| !c.is_ascii_hexdigit()).filter(|t| !t.is_empty()).last().unwrap_or("");
        parse_sha256(last).ok_or("Eine SHA-256-Prüfsumme hat 64 Zeichen (0–9, a–f)")?
    };
    let done = with_item(id, |it| {
        it.sha256 = h.clone();
        if it.sha256.is_empty() && it.sha_state != "info" {
            it.sha_state.clear();
        }
        it.status == "done"
    })
    .ok_or("Eintrag nicht gefunden")?;
    if done && !h.is_empty() {
        queue_verify(id);
    }
    mark_dirty();
    Ok(())
}

/* ---------- Proxy ---------- */
static PROXY: Mutex<String> = Mutex::new(String::new());
fn set_proxy(p: &str) {
    *lk(&PROXY) = p.trim().to_string();
}
fn check_proxy(p: &str) -> Result<(), String> {
    let p = p.trim();
    if p.is_empty() {
        return Ok(());
    }
    let rest = ["http://", "https://", "socks4://", "socks4a://", "socks5://", "socks5h://"]
        .iter()
        .find_map(|s| p.to_ascii_lowercase().starts_with(s).then(|| &p[s.len()..]))
        .ok_or("Proxy bitte als http://host:port oder socks5://host:port angeben")?;
    let hostport = rest.trim_end_matches('/').rsplit('@').next().unwrap_or("");
    let port = hostport.rsplit(':').next().unwrap_or("");
    if hostport.contains(':') && !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) && !p.contains(char::is_whitespace) {
        Ok(())
    } else {
        Err("Proxy bitte als http://host:port oder socks5://host:port angeben".into())
    }
}

/* ---------- Passwörter je Paket ---------- */
/// Passwörter eines Pakets zuerst, danach die allgemeine Liste (ohne Doppelte).
fn passwords_for(s: &State, pkg: &str) -> Vec<String> {
    let mut v: Vec<String> = s.pkg_passwords.get(pkg).cloned().unwrap_or_default();
    for p in &s.settings.passwords {
        if !v.contains(p) {
            v.push(p.clone());
        }
    }
    v
}
fn set_pkg_passwords(pkg: &str, text: &str) {
    let mut list: Vec<String> = Vec::new();
    for l in text.lines().map(|l| l.trim_end_matches('\r')).filter(|l| !l.is_empty()) {
        if !list.iter().any(|x| x == l) && list.len() < 200 {
            list.push(l.to_string());
        }
    }
    let mut s = st();
    if list.is_empty() {
        s.pkg_passwords.remove(pkg);
    } else {
        s.pkg_passwords.insert(pkg.to_string(), list);
    }
    drop(s);
    mark_dirty();
}
fn add_pkg_password(s: &mut State, pkg: &str, pw: &str) {
    if pkg.is_empty() {
        if !s.settings.passwords.iter().any(|x| x == pw) && s.settings.passwords.len() < 200 {
            s.settings.passwords.push(pw.to_string());
        }
    } else {
        let v = s.pkg_passwords.entry(pkg.to_string()).or_default();
        if !v.iter().any(|x| x == pw) && v.len() < 200 {
            v.push(pw.to_string());
        }
    }
}

/* ---------- API ---------- */
fn public_state() -> J {
    let rts: HashMap<String, Arc<Rt>> = lk(&app().running).clone();
    let s = st();
    let mut total = 0u64;
    let items = s
        .items
        .iter()
        .map(|it| {
            let rt = rts.get(&it.id);
            let speed = rt.map(|r| r.speed.load(SeqCst)).unwrap_or(0);
            total += speed;
            obj(vec![
                ("id", js(&it.id)),
                ("url", js(&it.url)),
                ("name", js(if it.name.is_empty() { &it.url } else { &it.name })),
                ("pkg", js(&it.pkg)),
                ("stage", js(&it.stage)),
                ("status", js(&it.status)),
                ("error", it.error.as_deref().map(js).unwrap_or(J::Null)),
                ("size", jopt(it.size)),
                ("done", jn(it.done_bytes())),
                ("resumable", J::Bool(it.resumable)),
                ("conns", jn(rt.map(|r| r.conns.load(SeqCst)).unwrap_or(0) as u64)),
                ("speed", jn(speed)),
                ("extract", js(&it.extract)),
                ("extractMsg", js(&it.extract_msg)),
                ("archive", J::Bool(matches!(archive_info(&it.name), Some((_, true))))),
                ("sha256", js(&it.sha256)),
                ("shaState", js(&it.sha_state)),
                ("shaGot", js(&it.sha_got)),
            ])
        })
        .collect();
    let accounts = s
        .accounts
        .iter()
        .map(|a| obj(vec![("host", js(&a.host)), ("user", js(&a.user)), ("ok", a.ok.map(J::Bool).unwrap_or(J::Null)), ("info", js(&a.info))]))
        .collect();
    obj(vec![
        ("active", J::Bool(s.active)),
        ("settings", settings_json(&s.settings)),
        ("speed", jn(total)),
        ("items", J::Arr(items)),
        ("accounts", J::Arr(accounts)),
        ("hosters", J::Arr(vec![js("rapidgator.net")])),
        ("extractor", js(&extractor_label())),
        ("cnl", js(&cnl_status())),
        ("pkgPasswords", pkg_passwords_json(&s)),
    ])
}

fn do_action(action: &str, ids: &[J]) -> Result<(), String> {
    let ids: Vec<&str> = ids.iter().filter_map(|j| j.str()).collect();
    if action == "extract" {
        for id in &ids {
            queue_extract(id, true);
        }
        return Ok(());
    }
    if action == "verify" {
        for id in &ids {
            if with_item(id, |it| it.status == "done").unwrap_or(false) {
                queue_verify(id);
            }
        }
        return Ok(());
    }
    let mut recheck = Vec::new();
    let mut to_stop: Vec<(String, &'static str, bool)> = Vec::new();
    {
        let mut s = st();
        let base = s.settings.dir.clone();
        for it in s.items.iter_mut().filter(|i| ids.contains(&i.id.as_str())) {
            match action {
                "confirm" => {
                    if it.stage == "grab" && it.status == "ready" {
                        it.stage = "queue".into();
                        it.status = "queued".into();
                        let d = if it.pkg.is_empty() { PathBuf::from(&base) } else { Path::new(&base).join(&it.pkg) };
                        it.dir = d.to_string_lossy().to_string();
                    }
                }
                "pause" => {
                    if it.stage == "queue" && it.status != "done" {
                        to_stop.push((it.id.clone(), "paused", false));
                        if matches!(it.status.as_str(), "queued" | "error") {
                            it.status = "paused".into();
                        }
                    }
                }
                "resume" => {
                    if it.stage == "queue" && matches!(it.status.as_str(), "paused" | "error") {
                        it.status = "queued".into();
                        it.error = None;
                        it.tries = 0;
                        it.wait_until = 0;
                    }
                }
                "recheck" => {
                    if it.stage == "grab" && it.status == "offline" {
                        it.status = "checking".into();
                        it.error = None;
                        recheck.push(it.id.clone());
                    }
                }
                "remove" => {
                    to_stop.push((it.id.clone(), "paused", true));
                    if it.stage == "queue" && it.status != "done" && it.status != "running" && !it.dir.is_empty() {
                        let _ = fs::remove_file(it.part_path());
                    }
                }
                _ => return Err("Unbekannte Aktion".into()),
            }
        }
        if action == "remove" {
            s.items.retain(|i| !ids.contains(&i.id.as_str()));
        }
    }
    for (id, status, removed) in to_stop {
        if let Some(rt) = rt_of(&id) {
            if removed {
                rt.removed.store(true, SeqCst);
            }
            stop_running(&rt, status);
        }
    }
    pool(recheck, 6, probe_item);
    mark_dirty();
    Ok(())
}

fn apply_settings(b: &J) -> Result<(), String> {
    let mut dir: Option<String> = None;
    if let Some(d) = b.get("dir").str().map(str::trim).filter(|d| !d.is_empty()) {
        let mut p = PathBuf::from(d);
        if d == "~" {
            p = home_dir();
        } else if let Some(r) = d.strip_prefix("~/").or_else(|| d.strip_prefix("~\\")) {
            p = home_dir().join(r);
        }
        if !p.is_absolute() {
            return Err("Bitte einen vollständigen Ordnerpfad angeben".into());
        }
        fs::create_dir_all(&p).map_err(|e| format!("Ordner lässt sich nicht anlegen: {}", e))?;
        dir = Some(p.to_string_lossy().to_string());
    }
    if let Some(p) = b.get("proxy").str() {
        check_proxy(p)?;
    }
    let mut s = st();
    let se = &mut s.settings;
    if let Some(d) = dir {
        se.dir = d;
    }
    se.max_parallel = clamp(b.get("maxParallel").int(), 1, 10, se.max_parallel as i64) as usize;
    se.chunks = clamp(b.get("chunks").int(), 1, 8, se.chunks as i64) as usize;
    se.speed_limit_kb = clamp(b.get("speedLimitKB").int(), 0, 10_000_000, se.speed_limit_kb as i64) as u64;
    se.retries = clamp(b.get("retries").int(), 0, 10, se.retries as i64) as u32;
    if let Some(v) = b.get("extract").bool() {
        se.extract = v;
    }
    if let Some(v) = b.get("deleteArchives").bool() {
        se.delete_archives = v;
    }
    if let Some(v) = b.get("clipboard").bool() {
        se.clipboard = v;
    }
    if let Some(v) = b.get("clicknload").bool() {
        se.clicknload = v;
    }
    if let Some(p) = b.get("proxy").str() {
        se.proxy = p.trim().to_string();
        set_proxy(&se.proxy);
    }
    if let Some(t) = b.get("passwords").str() {
        let mut list: Vec<String> = Vec::new();
        for l in t.lines().map(|l| l.trim_end_matches('\r')).filter(|l| !l.is_empty()) {
            if !list.iter().any(|x| x == l) && list.len() < 200 {
                list.push(l.to_string());
            }
        }
        se.passwords = list;
    }
    app().limit.store(se.speed_limit_kb * 1024, SeqCst);
    *lk(&app().next_free) = 0.0;
    mark_dirty();
    Ok(())
}

fn open_external(target: &str, is_folder: bool) {
    let r = if cfg!(windows) {
        if is_folder {
            Command::new("explorer").arg(target).spawn()
        } else {
            let mut c = Command::new("cmd");
            c.args(["/C", "start", "", target]);
            no_window(&mut c).spawn()
        }
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(target).spawn()
    } else {
        Command::new("xdg-open").arg(target).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
    };
    if let Ok(mut c) = r {
        thread::spawn(move || {
            let _ = c.wait();
        });
    }
}

/* ---------- Aktionen (für Fenster und Browser-Oberfläche) ---------- */
fn set_active(active: bool) {
    st().active = active;
    if !active {
        let rts: Vec<Arc<Rt>> = lk(&app().running).values().cloned().collect();
        for rt in rts {
            stop_running(&rt, "queued");
        }
    }
    mark_dirty();
}
/// Fertige Downloads (`done`) bzw. nicht erreichbare Links aus den Listen nehmen.
fn clear_items(done: bool) {
    st().items.retain(|i| if done { !(i.stage == "queue" && i.status == "done") } else { !(i.stage == "grab" && i.status == "offline") });
    mark_dirty();
}
const HOSTERS: &[&str] = &["rapidgator.net"];
/// Konto speichern und gleich prüfen (Anmeldung beim Hoster).
fn account_save(host: &str, user: &str, pw: &str) -> Result<(), String> {
    let host = host.trim().to_ascii_lowercase();
    let user = user.trim().to_string();
    if !HOSTERS.contains(&host.as_str()) {
        return Err("Dieser Hoster wird noch nicht unterstützt".into());
    }
    if user.is_empty() || pw.is_empty() {
        return Err("Bitte Benutzername und Passwort angeben".into());
    }
    let secret = protect(pw);
    {
        let mut s = st();
        s.accounts.retain(|a| a.host != host);
        s.accounts.push(Account { host: host.clone(), user, secret, token: None, ok: None, info: String::new() });
    }
    mark_dirty();
    let _ = rg_token(true);
    Ok(())
}
fn account_remove(host: &str) {
    st().accounts.retain(|a| a.host != host);
    mark_dirty();
}
fn open_download_dir() {
    let dir = st().settings.dir.clone();
    let _ = fs::create_dir_all(&dir);
    open_external(&dir, true);
}
fn ids_json(ids: &[String]) -> Vec<J> {
    ids.iter().map(|i| J::Str(i.clone())).collect()
}

/* ---------- HTTP-Server für die Browser-Oberfläche ---------- */
fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}
fn jerr(code: u16, msg: &str) -> (u16, &'static str, Vec<u8>) {
    (code, "application/json; charset=utf-8", obj(vec![("error", js(msg))]).text().into_bytes())
}
fn jok(j: J) -> (u16, &'static str, Vec<u8>) {
    (200, "application/json; charset=utf-8", j.text().into_bytes())
}

fn route(method: &str, path: &str, headers: &HashMap<String, String>, body: &[u8]) -> (u16, &'static str, Vec<u8>) {
    let port = app().port;
    let hosts = [format!("127.0.0.1:{}", port), format!("localhost:{}", port)];
    // Schutz: nur lokale Aufrufe von der eigenen Oberfläche
    let host = headers.get("host").map(String::as_str).unwrap_or("");
    if !hosts.iter().any(|h| h == host) {
        return jerr(403, "Nicht erlaubt");
    }
    let path = path.split('?').next().unwrap_or("");
    if method == "GET" && path == "/" {
        return (200, "text/html; charset=utf-8", HTML.as_bytes().to_vec());
    }
    if method == "GET" && path == "/api/state" {
        return jok(public_state());
    }
    if method != "POST" {
        return jerr(404, "Nicht gefunden");
    }
    if let Some(o) = headers.get("origin") {
        let o = o.strip_prefix("http://").or_else(|| o.strip_prefix("https://")).unwrap_or(o);
        if !hosts.iter().any(|h| h == o) {
            return jerr(403, "Nicht erlaubt");
        }
    }
    if !headers.get("content-type").map(|c| c.starts_with("application/json")).unwrap_or(false) {
        return jerr(415, "Nur JSON");
    }
    let Ok(b) = parse_json(if body.is_empty() { b"{}" } else { body }) else { return jerr(400, "Ungültiges JSON") };
    let okj = || jok(obj(vec![("ok", J::Bool(true))]));
    match path {
        "/api/grab" => jok(grab(b.get("text").str().unwrap_or(""), b.get("pkg").str().unwrap_or(""), b.get("scan").bool().unwrap_or(false))),
        "/api/action" => match do_action(b.get("action").str().unwrap_or(""), b.get("ids").arr()) {
            Ok(()) => okj(),
            Err(e) => jerr(400, &e),
        },
        "/api/settings" => match apply_settings(&b) {
            Ok(()) => okj(),
            Err(e) => jerr(400, &e),
        },
        "/api/control" => {
            set_active(b.get("active").bool().unwrap_or(false));
            okj()
        }
        "/api/clear" => {
            clear_items(b.get("what").str() == Some("done"));
            okj()
        }
        "/api/account" => match account_save(b.get("host").str().unwrap_or(""), b.get("user").str().unwrap_or(""), b.get("password").str().unwrap_or("")) {
            Ok(()) => okj(),
            Err(e) => jerr(400, &e),
        },
        "/api/hash" => match set_hash(b.get("id").str().unwrap_or(""), b.get("sha256").str().unwrap_or("")) {
            Ok(()) => okj(),
            Err(e) => jerr(400, &e),
        },
        "/api/pkgPasswords" => {
            set_pkg_passwords(&b.get("pkg").string(), b.get("passwords").str().unwrap_or(""));
            okj()
        }
        "/api/accountCheck" => {
            let _ = rg_token(true);
            okj()
        }
        "/api/accountRemove" => {
            account_remove(&b.get("host").string());
            okj()
        }
        "/api/quit" => {
            thread::spawn(|| {
                thread::sleep(Duration::from_millis(300));
                shutdown();
            });
            okj()
        }
        "/api/openFolder" => {
            open_download_dir();
            okj()
        }
        _ => jerr(404, "Nicht gefunden"),
    }
}

fn handle(mut s: TcpStream) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(10)));
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 8192];
    let hend = loop {
        match s.read(&mut tmp) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
        if let Some(i) = find(&buf, b"\r\n\r\n") {
            break i;
        }
        if buf.len() > 64 * 1024 {
            return;
        }
    };
    let head = String::from_utf8_lossy(&buf[..hend]).to_string();
    let mut lines = head.split("\r\n");
    let mut rl = lines.next().unwrap_or("").split(' ');
    let (method, path) = (rl.next().unwrap_or(""), rl.next().unwrap_or(""));
    let mut headers = HashMap::new();
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    let clen: usize = headers.get("content-length").and_then(|c| c.parse().ok()).unwrap_or(0);
    let (code, ctype, out) = if clen > 4 * 1024 * 1024 {
        jerr(413, "Anfrage zu gross")
    } else {
        let mut body = buf[hend + 4..].to_vec();
        while body.len() < clen {
            match s.read(&mut tmp) {
                Ok(0) | Err(_) => return,
                Ok(n) => body.extend_from_slice(&tmp[..n]),
            }
        }
        body.truncate(clen);
        route(method, path, &headers, &body)
    };
    let reason = if code == 200 { "OK" } else { "Fehler" };
    let hdr = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Frame-Options: DENY\r\nConnection: close\r\n\r\n",
        code,
        reason,
        ctype,
        out.len()
    );
    let _ = s.write_all(hdr.as_bytes());
    let _ = s.write_all(&out);
    let _ = s.flush();
}

/* ---------- Ansicht für das Fenster ---------- */
/// Ein Eintrag, wie ihn die Oberfläche zeigt (Kopie, damit beim Zeichnen nichts gesperrt bleibt).
#[derive(Clone)]
struct ItemView {
    id: String,
    url: String,
    name: String,
    pkg: String,
    stage: String,
    status: String,
    error: Option<String>,
    size: Option<u64>,
    done: u64,
    conns: usize,
    speed: u64,
    extract: String,
    extract_msg: String,
    archive: bool,
    dir: String,
    sha256: String,
    sha_state: String,
    sha_got: String,
}
#[derive(Clone)]
struct AccountView {
    host: String,
    user: String,
    ok: Option<bool>,
    info: String,
}
#[derive(Clone)]
struct Snapshot {
    active: bool,
    settings: Settings,
    speed: u64,
    items: Vec<ItemView>,
    accounts: Vec<AccountView>,
    cnl: String,
    pkg_passwords: HashMap<String, Vec<String>>,
}
fn snapshot() -> Snapshot {
    let rts: HashMap<String, Arc<Rt>> = lk(&app().running).clone();
    let s = st();
    let mut total = 0;
    let items = s
        .items
        .iter()
        .map(|it| {
            let rt = rts.get(&it.id);
            let speed = rt.map(|r| r.speed.load(SeqCst)).unwrap_or(0);
            total += speed;
            ItemView {
                id: it.id.clone(),
                url: it.url.clone(),
                name: if it.name.is_empty() { it.url.clone() } else { it.name.clone() },
                pkg: it.pkg.clone(),
                stage: it.stage.clone(),
                status: it.status.clone(),
                error: it.error.clone(),
                size: it.size,
                done: it.done_bytes(),
                conns: rt.map(|r| r.conns.load(SeqCst)).unwrap_or(0),
                speed,
                extract: it.extract.clone(),
                extract_msg: it.extract_msg.clone(),
                archive: matches!(archive_info(&it.name), Some((_, true))),
                dir: it.dir.clone(),
                sha256: it.sha256.clone(),
                sha_state: it.sha_state.clone(),
                sha_got: it.sha_got.clone(),
            }
        })
        .collect();
    let accounts = s.accounts.iter().map(|a| AccountView { host: a.host.clone(), user: a.user.clone(), ok: a.ok, info: a.info.clone() }).collect();
    Snapshot { active: s.active, settings: s.settings.clone(), speed: total, items, accounts, cnl: cnl_status(), pkg_passwords: s.pkg_passwords.clone() }
}

/* ---------- Start ---------- */
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let port: u16 = args
        .iter()
        .find_map(|a| a.strip_prefix("--port=").and_then(|p| p.parse().ok()))
        .or_else(|| std::env::var("PORT").ok().and_then(|p| p.parse().ok()))
        .unwrap_or(8765);
    let in_browser = args.iter().any(|a| a == "--browser");
    let no_open = args.iter().any(|a| a == "--no-open");
    let url = format!("http://127.0.0.1:{}", port);
    let data_dir = std::env::var_os("LADEWERK_DATA").map(PathBuf::from).unwrap_or_else(|| home_dir().join(".ladewerk"));

    // Der Port dient zugleich als Sperre: Ladewerk läuft nur einmal.
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(_) => {
            if in_browser {
                if !no_open {
                    open_external(&url, false);
                }
            } else {
                rfd::MessageDialog::new()
                    .set_title("Ladewerk")
                    .set_description(sprache::t("Ladewerk läuft bereits. Das offene Fenster findest du in der Taskleiste."))
                    .set_level(rfd::MessageLevel::Info)
                    .show();
            }
            return;
        }
    };

    let state = load_state(&data_dir);
    let limit = state.settings.speed_limit_kb * 1024;
    println!("Ladewerk läuft: {}", url);
    println!("Download-Ordner: {}", state.settings.dir);
    let _ = APP.set(App {
        st: Mutex::new(state),
        running: Mutex::new(HashMap::new()),
        next_free: Mutex::new(0.0),
        limit: AtomicU64::new(limit),
        dirty: AtomicBool::new(false),
        port,
        data_dir,
        extract_q: Mutex::new(Vec::new()),
        verify_q: Mutex::new(Vec::new()),
    });

    // Taktgeber: Downloads starten, Tempo messen, Zustand sichern
    thread::spawn(|| {
        let mut tick = 0u64;
        loop {
            thread::sleep(Duration::from_millis(250));
            tick += 1;
            schedule();
            let busy = !lk(&app().running).is_empty();
            if tick % 4 == 0 {
                for rt in lk(&app().running).values() {
                    let b = rt.bytes.load(SeqCst);
                    let d = b - rt.last.swap(b, SeqCst);
                    let old = rt.speed.load(SeqCst);
                    rt.speed.store(if old > 0 { (old + d) / 2 } else { d }, SeqCst);
                }
            }
            if app().dirty.swap(false, SeqCst) || (busy && tick % 8 == 0) {
                save_now();
            }
        }
    });
    thread::spawn(extract_loop);
    thread::spawn(verify_loop);
    thread::spawn(clipboard_loop);
    thread::spawn(cnl_loop);
    thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            thread::spawn(move || handle(conn));
        }
    });

    if in_browser {
        if !no_open {
            open_external(&url, false);
        }
        // läuft, bis in der Browser-Oberfläche «Beenden» gedrückt wird
        loop {
            thread::sleep(Duration::from_secs(3600));
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([860.0, 560.0])
            .with_title("Ladewerk"),
        ..Default::default()
    };
    let res = eframe::run_native("Ladewerk", options, Box::new(|cc| Ok(Box::new(app::Gui::new(cc)))));
    if let Err(e) = res {
        eprintln!("Fenster-Fehler: {}", e);
        rfd::MessageDialog::new()
            .set_title("Ladewerk")
            .set_description(format!("{}\n\n{}", sprache::t("Das Fenster lässt sich nicht öffnen. Mit «ladewerk --browser» läuft Ladewerk im Browser."), e))
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
    shutdown();
}

/* ---------- Oberfläche ---------- */
const HTML: &str = include_str!("ui.html");
