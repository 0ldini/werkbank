//! Gespeicherter Zustand (ohne egui, nur std): Einstellungen, Pakete und Links.
//!
//! Format: einfache Textdatei, eine Angabe pro Zeile `schlüssel<TAB>wert` (Tabulator,
//! Zeilenumbruch und `\` im Wert maskiert). `package` beginnt ein Paket, `item` einen Link
//! darin. Unbekannte Zeilen werden übergangen, damit kaputte oder neuere Dateien nie zum
//! Absturz führen. Gespeichert wird über eine temporäre Datei und Umbenennen.
//!
//! Prüfung: `rustc --edition 2021 --test src/store.rs -o /tmp/store && /tmp/store`

#![allow(dead_code)]

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Waiting,
    Running,
    Paused,
    Done,
    Failed,
}

impl Status {
    fn key(self) -> &'static str {
        match self {
            // Laufende Downloads werden beim nächsten Start fortgesetzt.
            Status::Waiting | Status::Running => "waiting",
            Status::Paused => "paused",
            Status::Done => "done",
            Status::Failed => "failed",
        }
    }
    fn from_key(s: &str) -> Status {
        match s {
            "paused" => Status::Paused,
            "done" => Status::Done,
            "failed" => Status::Failed,
            _ => Status::Waiting,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Unpack {
    Idle,
    Running,
    Done,
    Failed,
}

impl Unpack {
    fn key(self) -> &'static str {
        match self {
            Unpack::Idle | Unpack::Running => "idle",
            Unpack::Done => "done",
            Unpack::Failed => "failed",
        }
    }
    fn from_key(s: &str) -> Unpack {
        match s {
            "done" => Unpack::Done,
            "failed" => Unpack::Failed,
            _ => Unpack::Idle,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Settings {
    /// Gleichzeitige Downloads (1–20).
    pub max_parallel: u32,
    /// Gesamtgrenze in KB/s, 0 = unbegrenzt.
    pub speed_limit_kib: u64,
    /// Wiederholungen bei Fehlern.
    pub retries: u32,
    /// Grundwartezeit vor einer Wiederholung (Sekunden, wird verdoppelt).
    pub retry_delay_s: u32,
    pub download_dir: String,
    /// Jedes Paket in einen eigenen Unterordner.
    pub package_subfolder: bool,
    /// Downloads beim Programmstart sofort fortsetzen.
    pub autostart: bool,
    pub watch_clipboard: bool,
    /// Vorgabe für neue Pakete.
    pub auto_extract: bool,
    /// Jedes Archiv in einen eigenen Ordner entpacken.
    pub extract_subfolder: bool,
    /// Archive nach erfolgreichem Entpacken löschen.
    pub delete_archives: bool,
    /// Passwortliste für Archive.
    pub passwords: Vec<String>,
    /// Eigener Proxy (leer = aus der Umgebung, z. B. HTTPS_PROXY).
    pub proxy: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            max_parallel: 3,
            speed_limit_kib: 0,
            retries: 5,
            retry_delay_s: 5,
            download_dir: default_download_dir(),
            package_subfolder: true,
            autostart: true,
            watch_clipboard: false,
            auto_extract: true,
            extract_subfolder: true,
            delete_archives: false,
            passwords: Vec::new(),
            proxy: String::new(),
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Item {
    pub id: u64,
    pub url: String,
    /// Endgültiger Dateiname (leer, solange der Server noch nicht gefragt wurde).
    pub file_name: String,
    pub size: Option<u64>,
    pub done: u64,
    pub status: Status,
    pub error: String,
    /// Erwartete SHA-256-Prüfsumme (leer = keine Prüfung).
    pub sha256: String,
    /// Ergebnis der Prüfung: None = nicht geprüft.
    pub sha_ok: Option<bool>,
    pub etag: String,
    pub last_modified: String,
    /// Bisherige Fehlversuche seit dem letzten Start.
    pub tries: u32,
}

impl Item {
    pub fn new(id: u64, url: &str) -> Item {
        Item {
            id,
            url: url.to_string(),
            file_name: String::new(),
            size: None,
            done: 0,
            status: Status::Waiting,
            error: String::new(),
            sha256: String::new(),
            sha_ok: None,
            etag: String::new(),
            last_modified: String::new(),
            tries: 0,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Package {
    pub id: u64,
    pub name: String,
    /// Zielordner der Dateien.
    pub dir: String,
    pub auto_extract: bool,
    pub passwords: Vec<String>,
    pub unpack: Unpack,
    pub unpack_msg: String,
    pub expanded: bool,
    pub items: Vec<Item>,
}

impl Package {
    pub fn new(id: u64, name: &str, dir: &str) -> Package {
        Package {
            id,
            name: name.to_string(),
            dir: dir.to_string(),
            auto_extract: true,
            passwords: Vec::new(),
            unpack: Unpack::Idle,
            unpack_msg: String::new(),
            expanded: true,
            items: Vec::new(),
        }
    }
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct State {
    pub settings: Settings,
    pub packages: Vec<Package>,
    pub next_id: u64,
}

pub fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '\t' => o.push_str("\\t"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            c => o.push(c),
        }
    }
    o
}

pub fn unescape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            o.push(c);
            continue;
        }
        match it.next() {
            Some('t') => o.push('\t'),
            Some('n') => o.push('\n'),
            Some('r') => o.push('\r'),
            Some(x) => o.push(x),
            None => o.push('\\'),
        }
    }
    o
}

fn b(v: bool) -> &'static str {
    if v {
        "1"
    } else {
        "0"
    }
}

impl State {
    pub fn new_id(&mut self) -> u64 {
        self.next_id = self.next_id.max(1);
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    /// Position (Paket, Link) eines Links.
    pub fn find_item(&self, id: u64) -> Option<(usize, usize)> {
        self.packages
            .iter()
            .enumerate()
            .find_map(|(p, pk)| pk.items.iter().position(|i| i.id == id).map(|i| (p, i)))
    }

    pub fn item_mut(&mut self, id: u64) -> Option<&mut Item> {
        let (p, i) = self.find_item(id)?;
        self.packages.get_mut(p)?.items.get_mut(i)
    }

    pub fn to_text(&self) -> String {
        let mut o = String::from("ladewerk\t1\n");
        let mut kv = |k: &str, v: &str| {
            o.push_str(k);
            o.push('\t');
            o.push_str(&escape(v));
            o.push('\n');
        };
        let s = &self.settings;
        kv("s.max_parallel", &s.max_parallel.to_string());
        kv("s.speed_limit_kib", &s.speed_limit_kib.to_string());
        kv("s.retries", &s.retries.to_string());
        kv("s.retry_delay_s", &s.retry_delay_s.to_string());
        kv("s.download_dir", &s.download_dir);
        kv("s.package_subfolder", b(s.package_subfolder));
        kv("s.autostart", b(s.autostart));
        kv("s.watch_clipboard", b(s.watch_clipboard));
        kv("s.auto_extract", b(s.auto_extract));
        kv("s.extract_subfolder", b(s.extract_subfolder));
        kv("s.delete_archives", b(s.delete_archives));
        for p in &s.passwords {
            kv("s.password", p);
        }
        kv("s.proxy", &s.proxy);
        for p in &self.packages {
            kv("package", &p.id.to_string());
            kv("p.name", &p.name);
            kv("p.dir", &p.dir);
            kv("p.auto_extract", b(p.auto_extract));
            for pw in &p.passwords {
                kv("p.password", pw);
            }
            kv("p.unpack", p.unpack.key());
            kv("p.unpack_msg", &p.unpack_msg);
            kv("p.expanded", b(p.expanded));
            for i in &p.items {
                kv("item", &i.id.to_string());
                kv("i.url", &i.url);
                kv("i.file_name", &i.file_name);
                if let Some(sz) = i.size {
                    kv("i.size", &sz.to_string());
                }
                kv("i.done", &i.done.to_string());
                kv("i.status", i.status.key());
                kv("i.error", &i.error);
                kv("i.sha256", &i.sha256);
                if let Some(ok) = i.sha_ok {
                    kv("i.sha_ok", b(ok));
                }
                kv("i.etag", &i.etag);
                kv("i.last_modified", &i.last_modified);
            }
        }
        o
    }

    /// Liest den Zustand nachsichtig: Unbekanntes und Fehlerhaftes wird übergangen.
    pub fn from_text(text: &str) -> State {
        let mut st = State { settings: Settings::default(), packages: Vec::new(), next_id: 1 };
        let mut pw_reset = false;
        for line in text.lines() {
            let Some((k, v)) = line.trim_end_matches('\r').split_once('\t') else { continue };
            let v = unescape(v);
            let num = |v: &str| v.trim().parse::<u64>().ok();
            let flag = |v: &str| v.trim() == "1";
            let s = &mut st.settings;
            match k {
                "s.max_parallel" => s.max_parallel = num(&v).map(|n| n.clamp(1, 20) as u32).unwrap_or(s.max_parallel),
                "s.speed_limit_kib" => s.speed_limit_kib = num(&v).unwrap_or(0).min(10_000_000),
                "s.retries" => s.retries = num(&v).map(|n| n.min(100) as u32).unwrap_or(s.retries),
                "s.retry_delay_s" => s.retry_delay_s = num(&v).map(|n| n.clamp(1, 3600) as u32).unwrap_or(s.retry_delay_s),
                "s.download_dir" if !v.trim().is_empty() => s.download_dir = v,
                "s.package_subfolder" => s.package_subfolder = flag(&v),
                "s.autostart" => s.autostart = flag(&v),
                "s.watch_clipboard" => s.watch_clipboard = flag(&v),
                "s.auto_extract" => s.auto_extract = flag(&v),
                "s.extract_subfolder" => s.extract_subfolder = flag(&v),
                "s.delete_archives" => s.delete_archives = flag(&v),
                "s.password" => {
                    if !pw_reset {
                        s.passwords.clear();
                        pw_reset = true;
                    }
                    if !v.is_empty() && s.passwords.len() < 10_000 {
                        s.passwords.push(v);
                    }
                }
                "s.proxy" => s.proxy = v,
                "package" => {
                    let id = num(&v).unwrap_or(0);
                    st.packages.push(Package::new(id, "", ""));
                }
                "item" => {
                    let id = num(&v).unwrap_or(0);
                    if let Some(p) = st.packages.last_mut() {
                        p.items.push(Item::new(id, ""));
                    }
                }
                _ if k.starts_with("p.") => {
                    let Some(p) = st.packages.last_mut() else { continue };
                    match k {
                        "p.name" => p.name = v,
                        "p.dir" => p.dir = v,
                        "p.auto_extract" => p.auto_extract = flag(&v),
                        "p.password" if !v.is_empty() => p.passwords.push(v),
                        "p.unpack" => p.unpack = Unpack::from_key(&v),
                        "p.unpack_msg" => p.unpack_msg = v,
                        "p.expanded" => p.expanded = flag(&v),
                        _ => {}
                    }
                }
                _ if k.starts_with("i.") => {
                    let Some(i) = st.packages.last_mut().and_then(|p| p.items.last_mut()) else { continue };
                    match k {
                        "i.url" => i.url = v,
                        "i.file_name" => i.file_name = v,
                        "i.size" => i.size = num(&v),
                        "i.done" => i.done = num(&v).unwrap_or(0),
                        "i.status" => i.status = Status::from_key(&v),
                        "i.error" => i.error = v,
                        "i.sha256" => i.sha256 = v,
                        "i.sha_ok" => i.sha_ok = Some(flag(&v)),
                        "i.etag" => i.etag = v,
                        "i.last_modified" => i.last_modified = v,
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        // Links ohne Adresse und Pakete ohne Links sind wertlos; doppelte Kennungen neu vergeben.
        for p in &mut st.packages {
            p.items.retain(|i| !i.url.trim().is_empty());
        }
        st.packages.retain(|p| !p.items.is_empty());
        let mut seen = std::collections::HashSet::new();
        let mut max = 0u64;
        let mut need_new = Vec::new();
        for (pi, p) in st.packages.iter().enumerate() {
            if p.id == 0 || !seen.insert(p.id) {
                need_new.push((pi, None));
            }
            max = max.max(p.id);
            for (ii, i) in p.items.iter().enumerate() {
                if i.id == 0 || !seen.insert(i.id) {
                    need_new.push((pi, Some(ii)));
                }
                max = max.max(i.id);
            }
        }
        st.next_id = max.saturating_add(1);
        for (pi, ii) in need_new {
            let id = st.new_id();
            if let Some(p) = st.packages.get_mut(pi) {
                match ii {
                    None => p.id = id,
                    Some(ii) => {
                        if let Some(i) = p.items.get_mut(ii) {
                            i.id = id;
                        }
                    }
                }
            }
        }
        st
    }
}

/// Einstellungsordner: Windows `%APPDATA%\Ladewerk`, sonst `~/.config/ladewerk`.
pub fn config_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        let base = std::env::var_os("APPDATA")?;
        Some(PathBuf::from(base).join("Ladewerk"))
    } else {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("ladewerk"))
    }
}

pub fn state_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("ladewerk.txt"))
}

/// Download-Ordner des Benutzers.
pub fn default_download_dir() -> String {
    let home = if cfg!(windows) { std::env::var_os("USERPROFILE") } else { std::env::var_os("HOME") };
    match home {
        Some(h) => PathBuf::from(h).join("Downloads").to_string_lossy().into_owned(),
        None => ".".to_string(),
    }
}

/// Lädt den Zustand. Ist die Datei unlesbar, wird sie als `.defekt` beiseitegelegt
/// (nicht gelöscht) und mit dem Grundzustand begonnen.
pub fn load(path: &Path) -> State {
    match std::fs::read(path) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) if text.starts_with("ladewerk\t") => State::from_text(&text),
            _ => {
                let _ = std::fs::rename(path, path.with_extension("txt.defekt"));
                State::from_text("")
            }
        },
        Err(_) => State::from_text(""),
    }
}

/// Speichert über eine temporäre Datei, damit ein Absturz nie eine halbe Datei hinterlässt.
pub fn save(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("txt.neu");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> State {
        let mut st = State::from_text("");
        st.settings.max_parallel = 7;
        st.settings.passwords = vec!["geheim".into(), "mit\ttab\nund zeile\\".into()];
        st.settings.proxy = "http://proxy:3128".into();
        let id = st.new_id();
        let mut p = Package::new(id, "Urlaub 2026", "/tmp/Urlaub 2026");
        p.passwords.push("pw".into());
        p.unpack = Unpack::Done;
        let mut i = Item::new(st.new_id(), "https://e.com/a.zip");
        i.file_name = "a.zip".into();
        i.size = Some(1000);
        i.done = 500;
        i.status = Status::Running;
        i.sha_ok = Some(false);
        i.etag = "\"abc\"".into();
        p.items.push(i);
        p.items.push(Item::new(st.new_id(), "https://e.com/b.zip"));
        st.packages.push(p);
        st
    }

    #[test]
    fn hin_und_zurueck() {
        let st = sample();
        let back = State::from_text(&st.to_text());
        let mut expected = st.clone();
        // Laufendes wird als wartend gespeichert.
        expected.packages[0].items[0].status = Status::Waiting;
        assert_eq!(back, expected);
    }

    #[test]
    fn kaputte_eingaben() {
        let st = State::from_text("ladewerk\t1\ni.url\tohne paket\nitem\t5\np.name\tx\npackage\tabc\nitem\tzz\ni.url\thttps://e.com\ni.size\t-1\ni.done\t\ns.max_parallel\t99999\n\u{0}\t\t\t\n\\");
        assert_eq!(st.packages.len(), 1);
        assert_eq!(st.packages[0].items[0].url, "https://e.com");
        assert_eq!(st.packages[0].items[0].size, None);
        assert_eq!(st.settings.max_parallel, 20);
        // Kennungen 0 wurden neu vergeben und sind verschieden.
        assert_ne!(st.packages[0].id, 0);
        assert_ne!(st.packages[0].id, st.packages[0].items[0].id);
        assert!(st.next_id > st.packages[0].items[0].id);
    }

    #[test]
    fn doppelte_kennungen() {
        let st = State::from_text("ladewerk\t1\npackage\t3\nitem\t3\ni.url\tx\nitem\t3\ni.url\ty\n");
        let ids: Vec<u64> = std::iter::once(st.packages[0].id).chain(st.packages[0].items.iter().map(|i| i.id)).collect();
        assert_eq!(ids.len(), 3);
        assert!(ids[0] != ids[1] && ids[1] != ids[2] && ids[0] != ids[2]);
    }

    #[test]
    fn maskierung() {
        for s in ["", "a\\b", "\\t", "x\ty\nz\r", "\\"] {
            assert_eq!(unescape(&escape(s)), s);
        }
    }

    #[test]
    fn datei_speichern_und_laden() {
        let dir = std::env::temp_dir().join(format!("ladewerk-test-{}", std::process::id()));
        let path = dir.join("ladewerk.txt");
        let st = sample();
        save(&path, &st.to_text()).unwrap();
        let back = load(&path);
        assert_eq!(back.packages[0].items.len(), 2);
        std::fs::write(&path, b"\xff\xfe kaputt").unwrap();
        let fresh = load(&path);
        assert!(fresh.packages.is_empty());
        assert!(path.with_extension("txt.defekt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn suchen() {
        let mut st = sample();
        let id = st.packages[0].items[1].id;
        assert_eq!(st.find_item(id), Some((0, 1)));
        st.item_mut(id).unwrap().done = 9;
        assert_eq!(st.packages[0].items[1].done, 9);
        assert_eq!(st.find_item(999), None);
    }
}
