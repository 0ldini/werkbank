//! Kernlogik: Programmliste, Versionen ermitteln, herunterladen, prüfen, entpacken, starten.
//! Unabhängig von der Oberfläche, damit GUI und Kommandozeile dasselbe tun.

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const DEFAULT_MANIFEST: &str = include_str!("../apps.toml");

#[derive(Deserialize, Clone, Debug)]
pub struct Manifest {
    #[serde(default = "default_title")]
    pub title: String,
    #[serde(default)]
    pub app: Vec<App>,
    /// Adresse für Spenden (z. B. Buy Me a Coffee); leer = kein Spenden-Knopf.
    #[serde(default)]
    pub spenden: Option<String>,
    /// Fremdprogramme (z. B. Brave): bekannte Programme anderer Hersteller, die der Launcher
    /// mit ihrem offiziellen Installationsprogramm einrichtet und startet, aber nicht selbst
    /// baut oder aktualisiert. Eigene Liste, damit ältere Launcher sie nicht sehen.
    #[serde(default)]
    pub fremd: Vec<Fremd>,
}

fn default_title() -> String {
    "Werkbank".into()
}

#[derive(Deserialize, Clone, Debug)]
pub struct App {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub about: String,
    /// Rubrik: buero, foto, video oder system (optional; sonst eingebaut).
    #[serde(default)]
    pub category: Option<String>,
    /// Beschreibung auf Englisch (optional).
    #[serde(default)]
    pub about_en: Option<String>,
    pub repo: String,
    pub asset: Option<String>,
    pub exe: Option<String>,
    pub pin: Option<String>,
    pub color: Option<String>,
    /// Anfang der Release-Tags, z. B. "pdfcraft-v" (Standard: "v").
    pub tag: Option<String>,
    /// Name des eingebauten Icons (Standard: id).
    pub icon: Option<String>,
    /// Dateitypen (".xlsx"), die das Programm öffnen kann.
    #[serde(default)]
    pub types: Vec<String>,
    /// true: keine Browser-Version.
    #[serde(default)]
    pub no_web: bool,
    /// true: nur Browser-Version (z. B. eigene HTML5-Apps), keine Desktop-Programmdatei.
    #[serde(default)]
    pub web_only: bool,
    /// Nur auf diesen Systemen anbieten ("windows", "linux", "macos"); leer: überall.
    #[serde(default)]
    pub systeme: Vec<String>,
    /// Gesetzt bei Fremdprogrammen (aus `[[fremd]]`), nie aus der Datei gelesen.
    #[serde(skip)]
    pub fremd: Option<Box<Fremd>>,
}

/// Ein Fremdprogramm aus `[[fremd]]` in apps.toml. Neue Programme brauchen nur Daten.
#[derive(Deserialize, Clone, Debug, Default)]
pub struct Fremd {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub about: String,
    #[serde(default)]
    pub about_en: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    /// Webseite des Herstellers.
    #[serde(default)]
    pub homepage: Option<String>,
    /// Windows: Adresse des offiziellen Installationsprogramms (.exe).
    #[serde(default)]
    pub windows: Option<String>,
    /// macOS: Adresse des offiziellen Disk-Images (.dmg) mit dem .app-Bündel.
    #[serde(default)]
    pub macos: Option<String>,
    /// Linux: Anleitung des Herstellers (Installation über die Paketverwaltung).
    #[serde(default)]
    pub linux_hilfe: Option<String>,
    /// Windows: mögliche Orte der Programmdatei, mit %VARIABLEN%.
    #[serde(default)]
    pub pfade_windows: Vec<String>,
    /// macOS: mögliche Orte des .app-Bündels, ~ für den Benutzerordner.
    #[serde(default)]
    pub pfade_macos: Vec<String>,
    /// Linux: Befehle, unter denen das Programm im PATH liegen kann.
    #[serde(default)]
    pub befehle_linux: Vec<String>,
    /// Nur auf diesen Systemen anbieten; leer: überall, wo es Angaben gibt.
    #[serde(default)]
    pub systeme: Vec<String>,
    /// GitHub-Repository («besitzer/name»): das neueste Release liefert die Datei, deren Name
    /// nach `windows_datei`/`macos_datei`/`linux_datei` gebildet wird (`{v}` = Version).
    #[serde(default)]
    pub github: Option<String>,
    #[serde(default)]
    pub windows_datei: Option<String>,
    #[serde(default)]
    pub macos_datei: Option<String>,
    /// Linux: ein AppImage, das die Werkbank im Datenordner ablegt und startet.
    #[serde(default)]
    pub linux_datei: Option<String>,
    /// Endung der Prüfsummen-Datei neben dem Download (z. B. ".DIGEST"); gibt es sie, wird geprüft.
    #[serde(default)]
    pub digest: Option<String>,
}

impl Fremd {
    /// Auf diesem System anbieten?
    pub fn available(&self) -> bool {
        let os = std::env::consts::OS;
        if !self.systeme.is_empty() && !self.systeme.iter().any(|s| s == os) {
            return false;
        }
        match os {
            "windows" => self.windows.is_some() || !self.pfade_windows.is_empty() || self.datei().is_some(),
            "macos" => self.macos.is_some() || !self.pfade_macos.is_empty() || self.datei().is_some(),
            _ => self.linux_hilfe.is_some() || !self.befehle_linux.is_empty() || self.datei().is_some(),
        }
    }

    /// Dateimuster des Releases für dieses System (nur mit `github`).
    pub fn datei(&self) -> Option<&str> {
        self.github.as_ref()?;
        match std::env::consts::OS {
            "windows" => self.windows_datei.as_deref(),
            "macos" => self.macos_datei.as_deref(),
            _ => self.linux_datei.as_deref(),
        }
        .filter(|p| !p.trim().is_empty())
    }

    /// Kann der Launcher es auf diesem System selbst installieren?
    pub fn can_install(&self) -> bool {
        if self.datei().is_some() {
            return true;
        }
        match std::env::consts::OS {
            "windows" => self.windows.as_deref().is_some_and(|u| u.starts_with("https://")),
            "macos" => self.macos.as_deref().is_some_and(|u| u.starts_with("https://")),
            _ => false,
        }
    }

    /// Als Eintrag der Programmliste, damit es überall erscheint, wo Programme stehen.
    fn as_app(&self) -> App {
        App {
            id: self.id.clone(),
            name: self.name.clone(),
            about: self.about.clone(),
            category: self.category.clone(),
            about_en: self.about_en.clone(),
            repo: String::new(),
            asset: None,
            exe: None,
            pin: None,
            color: self.color.clone(),
            tag: None,
            icon: self.icon.clone(),
            types: Vec::new(),
            no_web: true,
            web_only: false,
            systeme: Vec::new(),
            fremd: Some(Box::new(self.clone())),
        }
    }
}

impl App {
    /// Beschreibung in der gewählten Sprache.
    pub fn about_text(&self) -> &str {
        match &self.about_en {
            Some(en) if crate::sprache::is_en() => en,
            _ => &self.about,
        }
    }

    fn asset(&self) -> &str {
        self.asset.as_deref().unwrap_or(&self.id)
    }
    fn exe(&self) -> &str {
        self.exe.as_deref().unwrap_or(&self.id)
    }
    fn tag(&self) -> &str {
        self.tag.as_deref().unwrap_or("v")
    }
    fn repo(&self) -> &str {
        self.repo.trim_end_matches('/').trim_end_matches(".git")
    }
}

/// Native Desktop-Version oder Web-Version (WebAssembly) einer App.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Native,
    Web,
}

impl Kind {
    fn key(self) -> &'static str {
        match self {
            Kind::Native => "native",
            Kind::Web => "web",
        }
    }
}

pub type Version = (u64, u64, u64);

pub fn parse_version(s: &str) -> Option<Version> {
    let mut it = s.trim().trim_start_matches('v').split('.');
    let v = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    it.next().is_none().then_some(v)
}

pub fn fmt_version(v: Version) -> String {
    format!("{}.{}.{}", v.0, v.1, v.2)
}

/// Kleine Einstellungsdatei des Launchers (nicht im Datenordner, denn die legt den Datenordner fest).
fn config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "Werkbank").map(|p| p.config_local_dir().join("config.toml"))
}

#[derive(Deserialize, serde::Serialize, Default, Clone)]
pub struct Config {
    pub data_dir: Option<PathBuf>,
    /// Zugeordnete Dateitypen je Programm (Stand, der im System eingetragen ist).
    #[serde(default)]
    pub types: std::collections::BTreeMap<String, Vec<String>>,
    /// true, sobald die Ersteinrichtung die Dateitypen einmal gefragt hat.
    #[serde(default)]
    pub types_asked: bool,
    /// Ort für das Wetter auf der Startseite.
    #[serde(default)]
    pub place: Option<Place>,
    /// Was die Startseite zeigt.
    #[serde(default)]
    pub home: Home,
    /// Zuletzt über die Werkbank geöffnete Dateien, neueste zuerst.
    #[serde(default)]
    pub recent: Vec<Recent>,
    /// Persönlicher GitHub-Zugriffsschlüssel (für ein privates Repository).
    #[serde(default)]
    pub github_token: Option<String>,
    /// Widget «Notizen».
    #[serde(default)]
    pub notes: Vec<String>,
    /// Widget «Ordner» (None: Downloads, Dokumente, Bilder).
    #[serde(default)]
    pub folders: Option<Vec<PathBuf>>,
    /// Widget «Termine»: Kalender-Adresse (ICS, auch webcal://).
    #[serde(default)]
    pub calendar_url: Option<String>,
    /// Widget «Nachrichten»: RSS-Adresse (None: SRF News).
    #[serde(default)]
    pub news_url: Option<String>,
    /// true: beim Start nicht zählen lassen (siehe meldung.rs).
    #[serde(default)]
    pub nicht_melden: bool,
}

/// Ein Widget der Startseite: Art und Grösse in Rasterfeldern (6 Spalten, Zeilen à ~92 px).
#[derive(Deserialize, serde::Serialize, Clone, Debug, PartialEq)]
pub struct WidgetCfg {
    pub kind: String,
    #[serde(default = "two")]
    pub w: u8,
    #[serde(default = "two")]
    pub h: u8,
}

fn two() -> u8 {
    2
}

/// Alle Widget-Arten in der Reihenfolge des Menüs «Widget hinzufügen».
pub const WIDGET_KINDS: [&str; 10] = ["uhr", "wetter", "mail", "termine", "zuletzt", "notizen", "nachrichten", "timer", "ordner", "computer"];

pub fn default_widgets() -> Vec<WidgetCfg> {
    [("uhr", 2), ("wetter", 3), ("mail", 1), ("termine", 2), ("zuletzt", 2), ("notizen", 2), ("nachrichten", 3), ("timer", 1), ("ordner", 1), ("computer", 1)]
        .iter()
        .map(|(k, w)| WidgetCfg { kind: k.to_string(), w: *w, h: 2 })
        .collect()
}

#[derive(Deserialize, serde::Serialize, Clone, Debug, PartialEq)]
pub struct Place {
    pub name: String,
    #[serde(default)]
    pub region: Option<String>,
    pub lat: f64,
    pub lon: f64,
}

impl Place {
    pub fn sarnen() -> Self {
        Place { name: "Sarnen".into(), region: Some("Obwalden, Schweiz".into()), lat: 46.8961, lon: 8.2457 }
    }
}

#[derive(Deserialize, serde::Serialize, Clone, Debug)]
pub struct Home {
    #[serde(default = "yes")]
    pub clock: bool,
    #[serde(default = "yes")]
    pub weather: bool,
    #[serde(default = "yes")]
    pub recent: bool,
    /// «Meine Programme» nach Rubriken gruppiert (Kacheln je Kategorie).
    #[serde(default = "yes")]
    pub grouped: bool,
    /// Widgets in ihrer Reihenfolge; leer = Vorgabe.
    #[serde(default)]
    pub widgets: Vec<WidgetCfg>,
    /// Widget-Leiste ausgeklappt (sonst nur die schmale Zeile).
    #[serde(default)]
    pub bar_open: bool,
    /// Programme im Schnellzugriff in ihrer Reihenfolge; None = Vorgabe.
    #[serde(default)]
    pub quick: Option<Vec<String>>,
    /// Programmnamen unter den Symbolen.
    #[serde(default = "yes")]
    pub names: bool,
    /// Grösse der Programm-Symbole: 0 klein, 1 mittel, 2 gross.
    #[serde(default = "one")]
    pub icon_size: u8,
}

fn one() -> u8 {
    1
}

fn yes() -> bool {
    true
}

impl Default for Home {
    fn default() -> Self {
        Home { clock: true, weather: true, recent: true, grouped: true, widgets: Vec::new(), bar_open: false, quick: None, names: true, icon_size: 1 }
    }
}

#[derive(Deserialize, serde::Serialize, Clone, Debug)]
pub struct Recent {
    pub path: PathBuf,
    pub app: String,
    /// Unix-Sekunden.
    pub when: u64,
}

/// Merkt sich eine geöffnete Datei für «Zuletzt geöffnet».
pub fn remember_recent(path: &Path, app: &str) {
    let mut cfg = load_config();
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    cfg.recent.retain(|r| r.path != path);
    let when = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    cfg.recent.insert(0, Recent { path, app: app.to_string(), when });
    cfg.recent.truncate(12);
    let _ = save_config(&cfg);
}

pub fn save_config(cfg: &Config) -> Result<()> {
    let path = config_path().ok_or_else(|| anyhow!("kein Ort für die Einstellungen gefunden"))?;
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)?;
    }
    fs::write(&path, toml::to_string(cfg)?)?;
    Ok(())
}

pub fn load_config() -> Config {
    config_path()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default()
}

/// Datenordner, der ohne Nachfrage feststeht: Umgebungsvariable, `portable.txt` neben dem
/// Launcher (Ordner `data` daneben, z. B. USB-Stick) oder die gespeicherte Wahl des Benutzers.
pub fn configured_data_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("WERKBANK_DATA") {
        return Some(PathBuf::from(d));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        if dir.join("portable.txt").exists() {
            return Some(dir.join("data"));
        }
    }
    load_config().data_dir
}

/// Vorschlag für die Ersteinrichtung.
pub fn default_data_dir() -> PathBuf {
    if cfg!(windows) {
        let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
        return PathBuf::from(pf).join("Werkbank");
    }
    directories::ProjectDirs::from("", "", "Werkbank")
        .map(|p| p.data_local_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("werkbank-data"))
}

/// Datenordner; vor der Ersteinrichtung der Vorschlag.
pub fn data_dir() -> PathBuf {
    configured_data_dir().unwrap_or_else(default_data_dir)
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(".werkbank-schreibtest");
    let ok = fs::write(&probe, b"").is_ok();
    let _ = fs::remove_file(&probe);
    ok
}

/// Legt den Datenordner an und merkt ihn sich. Unter Windows wird ein Ordner, den der Benutzer
/// nicht beschreiben darf (z. B. unter C:\Program Files), einmalig mit Administratorrechten
/// angelegt und dem Benutzer freigegeben; danach braucht der Launcher keine Rechte mehr.
pub fn set_data_dir(dir: &Path) -> Result<()> {
    let dir = if dir.is_absolute() { dir.to_path_buf() } else { bail!("bitte einen vollständigen Pfad angeben") };
    if fs::create_dir_all(&dir).is_err() || !writable(&dir) {
        if cfg!(windows) {
            create_dir_elevated(&dir)?;
        }
        if !writable(&dir) {
            bail!("{} lässt sich nicht beschreiben", dir.display());
        }
    }
    let mut cfg = load_config();
    cfg.data_dir = Some(dir);
    save_config(&cfg)
}

/// Windows: startet den Launcher selbst mit Administratorrechten (UAC-Abfrage), der den Ordner
/// anlegt und dem aktuellen Benutzer Vollzugriff gibt (`--mkdir-elevated`).
fn create_dir_elevated(dir: &Path) -> Result<()> {
    let exe = std::env::current_exe()?;
    let user = format!(
        "{}\\{}",
        std::env::var("USERDOMAIN").unwrap_or_default(),
        std::env::var("USERNAME").unwrap_or_default()
    );
    let args = format!("--mkdir-elevated \"{}\" \"{}\"", dir.display(), user);
    let script = format!(
        "$p = Start-Process -FilePath '{}' -ArgumentList '{}' -Verb RunAs -Wait -PassThru; exit $p.ExitCode",
        exe.display().to_string().replace('\'', "''"),
        args.replace('\'', "''")
    );
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()
        .context("PowerShell für die Rechteabfrage starten")?;
    if !status.success() {
        bail!("Ordner konnte nicht mit Administratorrechten angelegt werden (abgebrochen?)");
    }
    Ok(())
}

/// Läuft in der erhöhten Instanz: Ordner anlegen und dem Benutzer freigeben.
pub fn mkdir_elevated(dir: &Path, user: &str) -> Result<()> {
    fs::create_dir_all(dir)?;
    if cfg!(windows) && !user.is_empty() && user != "\\" {
        let status = std::process::Command::new("icacls")
            .arg(dir)
            .args(["/grant", &format!("{user}:(OI)(CI)F"), "/T", "/Q"])
            .status()
            .context("icacls")?;
        if !status.success() {
            bail!("icacls fehlgeschlagen");
        }
    }
    Ok(())
}

/// Wo die aktuelle Programmliste liegt. Neue Programme erscheinen so ohne neuen Launcher.
pub const MANIFEST_URL: &str = "https://raw.githubusercontent.com/0ldini/werkbank/main/launcher/apps.toml";

/// Lädt die Programmliste. Reihenfolge: eigene `apps.toml` im Datenordner (falls vorhanden),
/// sonst die aktuelle Liste aus dem Netz, sonst deren letzte gespeicherte Fassung, sonst die
/// in den Launcher eingebaute.
pub fn load_manifest() -> Result<Manifest> {
    let mut m = load_manifest_all()?;
    m.app.retain(|a| a.systeme.is_empty() || a.systeme.iter().any(|s| s == std::env::consts::OS));
    add_fremd(&mut m);
    Ok(m)
}

/// Hängt die Fremdprogramme dieses Systems als Einträge an (ohne doppelte Kennungen).
pub fn add_fremd(m: &mut Manifest) {
    for f in &m.fremd {
        if f.available() && !f.id.is_empty() && !m.app.iter().any(|a| a.id == f.id) {
            m.app.push(f.as_app());
        }
    }
}

fn load_manifest_all() -> Result<Manifest> {
    let dir = data_dir();
    fs::create_dir_all(&dir).with_context(|| format!("Datenordner {} anlegen", dir.display()))?;
    let own = dir.join("apps.toml");
    if own.exists() {
        let text = fs::read_to_string(&own)?;
        return toml::from_str(&text).with_context(|| format!("{} ist fehlerhaft", own.display()));
    }
    let cache = dir.join("apps-online.toml");
    let url = std::env::var("WERKBANK_MANIFEST").unwrap_or_else(|_| MANIFEST_URL.to_string());
    if let Ok(text) = http_text(&url, 1 << 20) {
        if let Ok(manifest) = toml::from_str::<Manifest>(&text) {
            let _ = fs::write(&cache, &text);
            return Ok(manifest);
        }
    }
    if let Some(manifest) = fs::read_to_string(&cache).ok().and_then(|t| toml::from_str(&t).ok()) {
        return Ok(manifest);
    }
    toml::from_str(DEFAULT_MANIFEST).context("eingebaute Programmliste ist fehlerhaft")
}

fn agent() -> ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig};
    ureq::Agent::config_builder()
        .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
        .user_agent(concat!("werkbank/", env!("CARGO_PKG_VERSION")))
        .timeout_connect(Some(std::time::Duration::from_secs(8)))
        // Weiterleitungen folgt fetch() selbst, siehe dort.
        .max_redirects(0)
        .max_redirects_will_error(false)
        .build()
        .into()
}

// ---------------------------------------------------------------------------------------------
// Zugang zu einem privaten GitHub-Repository: persönlicher Zugriffsschlüssel (Token, nur Lesen).

static TOKEN: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

/// Schlüssel setzen (aus den Einstellungen oder WERKBANK_GITHUB_TOKEN).
pub fn set_token(token: Option<String>) {
    let t = token.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    if let Ok(mut g) = TOKEN.write() {
        *g = t;
    }
}

pub fn has_token() -> bool {
    token().is_some()
}

fn token() -> Option<String> {
    TOKEN.read().ok().and_then(|g| g.clone())
}

/// Liest den Schlüssel beim Start: Umgebung vor Einstellungen.
pub fn init_token() {
    let t = std::env::var("WERKBANK_GITHUB_TOKEN").ok().or_else(|| load_config().github_token);
    set_token(t);
}

fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in input.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Kopfzeile mit dem Schlüssel, nur für GitHub selbst (nie für den Download-Server dahinter).
fn auth_header(url: &str) -> Option<String> {
    let t = token()?;
    let host = url.strip_prefix("https://")?.split('/').next()?;
    match host {
        // Git-Abfragen (Versionen) erwarten Basic-Anmeldung.
        "github.com" => Some(format!("Basic {}", base64(format!("x-access-token:{t}").as_bytes()))),
        "api.github.com" | "raw.githubusercontent.com" => Some(format!("Bearer {t}")),
        _ => None,
    }
}

/// GET mit eigener Behandlung von Weiterleitungen. GitHub leitet Release-Downloads auf einen
/// anderen Server um; der Inhalt der Umleitungs-Antwort wird hier gar nicht erst gelesen, weil
/// manche Proxys ihn fehlerhaft übertragen. Gefolgt wird nur zu https-Adressen.
fn fetch(url: &str) -> Result<ureq::http::Response<ureq::Body>> {
    fetch_accept(url, None)
}

fn fetch_accept(url: &str, accept: Option<&str>) -> Result<ureq::http::Response<ureq::Body>> {
    fetch_with(url, accept, true)
}

/// Wie `fetch_accept`; `auth` false schickt den GitHub-Schlüssel nie mit (fremde Downloads).
fn fetch_with(url: &str, accept: Option<&str>, auth: bool) -> Result<ureq::http::Response<ureq::Body>> {
    let agent = agent();
    let mut url = url.to_string();
    for _ in 0..6 {
        let mut req = agent.get(&url);
        if let Some(a) = auth_header(&url).filter(|_| auth) {
            req = req.header("Authorization", a);
        }
        if let Some(a) = accept {
            req = req.header("Accept", a);
        }
        let resp = req.call().with_context(|| format!("Abruf von {}", url.split('?').next().unwrap_or(&url)))?;
        if !resp.status().is_redirection() {
            return Ok(resp);
        }
        let target = resp.headers().get("location").and_then(|h| h.to_str().ok()).unwrap_or_default().to_string();
        if !target.starts_with("https://") {
            bail!("unerwartete Weiterleitung von {url}");
        }
        url = target;
    }
    bail!("zu viele Weiterleitungen: {url}")
}

pub fn http_text(url: &str, limit: u64) -> Result<String> {
    let mut resp = fetch(url)?;
    let mut buf = Vec::new();
    resp.body_mut().as_reader().take(limit).read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// JSON an eine Adresse schicken und die Antwort als Text lesen (für den Zähler).
/// Weiterleitungen (z. B. auf www.) werden bis zu dreimal mit derselben Anfrage befolgt.
pub fn post_json(url: &str, body: &str) -> Result<String> {
    let mut url = url.to_string();
    for _ in 0..4 {
        let mut resp = agent().post(&url).header("Content-Type", "application/json").send(body)?;
        if resp.status().is_redirection() {
            let next = resp.headers().get("location").and_then(|h| h.to_str().ok()).ok_or_else(|| anyhow!("Weiterleitung ohne Ziel"))?;
            url = if next.starts_with("http") {
                next.to_string()
            } else {
                let base: String = url.splitn(4, '/').take(3).collect::<Vec<_>>().join("/");
                format!("{base}{next}")
            };
            continue;
        }
        let mut buf = Vec::new();
        resp.body_mut().as_reader().take(1 << 20).read_to_end(&mut buf)?;
        return Ok(String::from_utf8_lossy(&buf).into_owned());
    }
    bail!("zu viele Weiterleitungen")
}

/// Alle Versions-Tags (`vX.Y.Z`) eines Repositories, neueste zuerst.
/// Liest die Git-Referenzen direkt, braucht also kein Konto und kennt kein API-Limit.
pub fn remote_versions(app: &App) -> Result<Vec<Version>> {
    let url = format!("{}.git/info/refs?service=git-upload-pack", app.repo());
    let body = http_text(&url, 8 << 20)?;
    let mut out: Vec<Version> = body
        .split("refs/tags/")
        .skip(1)
        .filter_map(|rest| {
            let tag: String = rest.chars().take_while(|c| !c.is_whitespace() && *c != '^').collect();
            tag.strip_prefix(app.tag()).and_then(parse_version)
        })
        .collect();
    out.sort();
    out.dedup();
    out.reverse();
    Ok(out)
}

fn release_url(app: &App, v: Version, file: &str) -> String {
    format!("{}/releases/download/{}{}/{}", app.repo(), app.tag(), fmt_version(v), file)
}

/// Datei eines Releases holen. Mit Schlüssel über die GitHub-API (nötig bei privaten
/// Repositories), sonst über die öffentliche Download-Adresse.
fn release_file(app: &App, v: Version, file: &str) -> Result<ureq::http::Response<ureq::Body>> {
    let repo = app.repo();
    if let (true, Some(path)) = (has_token(), repo.strip_prefix("https://github.com/")) {
        let tag = format!("{}{}", app.tag(), fmt_version(v));
        let api = format!("https://api.github.com/repos/{path}/releases/tags/{tag}");
        let body = {
            let mut resp = fetch_accept(&api, Some("application/vnd.github+json"))?;
            let mut buf = Vec::new();
            resp.body_mut().as_reader().take(4 << 20).read_to_end(&mut buf)?;
            buf
        };
        let release: serde_json::Value = serde_json::from_slice(&body).context("Antwort der GitHub-API")?;
        let asset = release["assets"]
            .as_array()
            .and_then(|a| a.iter().find(|x| x["name"].as_str() == Some(file)))
            .and_then(|x| x["url"].as_str())
            .ok_or_else(|| anyhow!("{file} fehlt im Release {tag}"))?
            .to_string();
        return fetch_accept(&asset, Some("application/octet-stream"));
    }
    fetch(&release_url(app, v, file))
}

fn release_text(app: &App, v: Version, file: &str) -> Result<String> {
    let mut resp = release_file(app, v, file)?;
    let mut buf = Vec::new();
    resp.body_mut().as_reader().take(1 << 20).read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Neueste Version, zu der tatsächlich ein Release mit Dateien existiert.
pub fn latest_version(app: &App) -> Result<Version> {
    if app.fremd.is_some() {
        bail!("{} aktualisiert sich selbst", app.name);
    }
    if let Some(pin) = &app.pin {
        return parse_version(pin).ok_or_else(|| anyhow!("pin \"{pin}\" ist keine Version"));
    }
    for v in remote_versions(app)?.into_iter().take(5) {
        if release_text(app, v, "SHA256SUMS.txt").is_ok() {
            return Ok(v);
        }
    }
    bail!("kein Release gefunden unter {}", app.repo())
}

/// Dateiname des Release-Archivs für dieses System.
pub fn asset_name(app: &App, kind: Kind, v: Version) -> Result<String> {
    let ver = fmt_version(v);
    let a = app.asset();
    if kind == Kind::Web {
        return Ok(format!("{a}-web-{ver}.zip"));
    }
    let suffix = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "windows-x64-portable.zip",
        ("windows", "aarch64") => "windows-arm64-portable.zip",
        ("windows", "x86") => "windows-x86-portable.zip",
        ("linux", "x86_64") => "linux-x86_64.tar.gz",
        ("linux", "aarch64") => "linux-aarch64.tar.gz",
        ("linux", "riscv64") => "linux-riscv64.tar.gz",
        ("freebsd", "x86_64") => "freebsd-x86_64.tar.gz",
        ("macos", "aarch64") => "macos-aarch64.tar.gz",
        (os, arch) => bail!("für {os}/{arch} gibt es keine Desktop-Version; nutze die Browser-Version"),
    };
    Ok(format!("{a}-{ver}-{suffix}"))
}

fn app_dir(app: &App) -> PathBuf {
    data_dir().join("apps").join(&app.id)
}

fn version_dir(app: &App, kind: Kind, v: Version) -> PathBuf {
    app_dir(app).join(format!("{}-{}", kind.key(), fmt_version(v)))
}

fn current_file(app: &App, kind: Kind) -> PathBuf {
    app_dir(app).join(format!("current-{}.txt", kind.key()))
}

pub fn installed_version(app: &App, kind: Kind) -> Option<Version> {
    if app.fremd.is_some() {
        return None;
    }
    let v = parse_version(&fs::read_to_string(current_file(app, kind)).ok()?)?;
    version_dir(app, kind, v).is_dir().then_some(v)
}

/// Der Ordner im entpackten Archiv, der die App enthält (Archive haben genau einen Hauptordner).
fn content_root(dir: &Path) -> PathBuf {
    let subs: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    match subs.as_slice() {
        [one] if one.is_dir() => one.clone(),
        _ => dir.to_path_buf(),
    }
}

pub fn web_root(app: &App) -> Option<PathBuf> {
    let v = installed_version(app, Kind::Web)?;
    Some(content_root(&version_dir(app, Kind::Web, v)))
}

pub fn exe_path(app: &App) -> Option<PathBuf> {
    let v = installed_version(app, Kind::Native)?;
    let root = content_root(&version_dir(app, Kind::Native, v));
    let exe = if cfg!(windows) { format!("{}.exe", app.exe()) } else { app.exe().to_string() };
    [root.join(&exe), root.join("bin").join(&exe)]
        .into_iter()
        .find(|p| p.is_file())
        .or_else(|| bundle_exe(&root, &exe))
}

/// macOS: Programmdatei in einem .app-Bündel (`Name.app/Contents/MacOS/…`). Bevorzugt die Datei
/// mit dem Namen des Programms, sonst die einzige Datei im Ordner.
fn bundle_exe(root: &Path, exe: &str) -> Option<PathBuf> {
    let mut bundles: Vec<PathBuf> = if root.extension().is_some_and(|e| e == "app") {
        vec![root.to_path_buf()]
    } else {
        fs::read_dir(root).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir() && p.extension().is_some_and(|e| e == "app")).collect()
    };
    bundles.sort();
    for b in bundles {
        let dir = b.join("Contents").join("MacOS");
        let files: Vec<PathBuf> = fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
        let named = files.iter().find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(exe)));
        if let Some(p) = named.or(if files.len() == 1 { files.first() } else { None }) {
            return Some(p.clone());
        }
    }
    None
}

/// macOS: das .app-Bündel, in dem eine Programmdatei liegt (…/Name.app/Contents/MacOS/datei).
pub fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let b = exe.parent()?.parent()?.parent()?;
    (cfg!(target_os = "macos") && b.extension().is_some_and(|e| e == "app")).then(|| b.to_path_buf())
}

/// Fortschritt: Text und Anteil 0..1.
pub type Progress<'a> = &'a (dyn Fn(&str, f32) + Sync);

fn download(mut resp: ureq::http::Response<ureq::Body>, dest: &Path, progress: Progress) -> Result<String> {
    let total: f32 = resp
        .headers()
        .get("content-length")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    let mut reader = resp.body_mut().as_reader();
    let mut file = fs::File::create(dest)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut done = 0f32;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        done += n as f32;
        let mb = done / 1_048_576.0;
        progress(&crate::sprache::tf("Lade herunter … {mb} MB", &[("mb", &format!("{mb:.0}"))]), if total > 0.0 { done / total } else { 0.0 });
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut f = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut f, &mut hasher)?;
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn extract(archive: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    let name = archive.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    if name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(fs::File::open(archive)?)?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i)?;
            // enclosed_name weist Pfade ab, die aus dem Zielordner herausführen würden.
            let rel = entry.enclosed_name().ok_or_else(|| anyhow!("unsicherer Pfad im Archiv"))?;
            let out = dest.join(rel);
            if entry.is_dir() {
                fs::create_dir_all(&out)?;
                continue;
            }
            if let Some(p) = out.parent() {
                fs::create_dir_all(p)?;
            }
            let mut f = fs::File::create(&out)?;
            std::io::copy(&mut entry, &mut f)?;
            #[cfg(unix)]
            if let Some(mode) = entry.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&out, fs::Permissions::from_mode(mode))?;
            }
        }
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        let gz = flate2::read::GzDecoder::new(fs::File::open(archive)?);
        let mut tar = tar::Archive::new(gz);
        for entry in tar.entries()? {
            // unpack_in weist Einträge ab, die aus dem Zielordner herausführen würden.
            if !entry?.unpack_in(dest)? {
                bail!("unsicherer Pfad im Archiv");
            }
        }
    } else {
        bail!("unbekanntes Archivformat: {name}");
    }
    Ok(())
}

/// Entpackt ein geprüftes Archiv als neue Version und schaltet danach um.
/// Die alte Version bleibt bis zum Umschalten unangetastet; schlägt etwas fehl, läuft sie weiter.
fn activate(app: &App, kind: Kind, v: Version, archive: &Path, progress: Progress) -> Result<()> {
    let final_dir = version_dir(app, kind, v);
    let tmp_dir = final_dir.with_extension("teil");
    let _ = fs::remove_dir_all(&tmp_dir);
    progress(crate::sprache::t("Entpacke …"), 1.0);
    extract(archive, &tmp_dir)?;
    if kind == Kind::Native {
        // Die Windows-Portable-Builds speichern Einstellungen neben der .exe, solange
        // portable.txt existiert. Ohne sie landen Einstellungen im Benutzerordner und
        // überleben so jedes Update.
        let _ = fs::remove_file(content_root(&tmp_dir).join("portable.txt"));
    }
    let _ = fs::remove_dir_all(&final_dir);
    fs::rename(&tmp_dir, &final_dir)?;
    fs::write(current_file(app, kind), fmt_version(v))?;
    // Alte Versionen dieser Art aufräumen. Läuft die alte App noch, misslingt das unter
    // Windows; dann wird beim nächsten Update erneut aufgeräumt.
    let prefix = format!("{}-", kind.key());
    for e in fs::read_dir(app_dir(app))?.flatten() {
        let p = e.path();
        let n = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() && n.starts_with(&prefix) && p != final_dir {
            let _ = fs::remove_dir_all(&p);
        }
    }
    Ok(())
}

/// Installiert oder aktualisiert genau eine App auf die neueste Version.
/// Gibt die nun installierte Version zurück.
pub fn install(app: &App, kind: Kind, progress: Progress) -> Result<Version> {
    if app.fremd.is_some() {
        bail!("{} ist ein Fremdprogramm und wird mit fremd_install eingerichtet", app.name);
    }
    progress(crate::sprache::t("Suche neueste Version …"), 0.0);
    let v = latest_version(app)?;
    if installed_version(app, kind) == Some(v) {
        return Ok(v);
    }
    let file = asset_name(app, kind, v)?;
    let sums = release_text(app, v, "SHA256SUMS.txt")?;
    let expected = sums
        .lines()
        .find_map(|l| {
            let (sum, name) = l.split_once(char::is_whitespace)?;
            (name.trim().trim_start_matches('*') == file).then(|| sum.to_lowercase())
        })
        .ok_or_else(|| anyhow!("{file} fehlt im Release v{}", fmt_version(v)))?;

    fs::create_dir_all(app_dir(app))?;
    let archive = app_dir(app).join(&file);
    let actual = download(release_file(app, v, &file)?, &archive, progress)?;
    if actual != expected {
        let _ = fs::remove_file(&archive);
        bail!("Prüfsumme von {file} stimmt nicht; Download verworfen");
    }
    let result = activate(app, kind, v, &archive, progress);
    let _ = fs::remove_file(&archive);
    result.map(|_| v)
}

// ---------------------------------------------------------------------------------------------
// Der Launcher selbst

/// Der Launcher als «Programm»: Releases `werkbank-vX.Y.Z` im Werkbank-Repository.
fn launcher_app() -> Result<App> {
    toml::from_str("id = \"werkbank\"\nname = \"Werkbank\"\nrepo = \"https://github.com/0ldini/werkbank\"\ntag = \"werkbank-v\"")
        .context("Launcher-Eintrag")
}

/// Eigene Version (bei Probe-Builds ohne Nummer: None, dann gibt es kein Selbst-Update).
pub fn own_version() -> Option<Version> {
    parse_version(env!("CARGO_PKG_VERSION"))
}

/// Pfad der eigenen Programmdatei, beim Start festgehalten (nach dem Umbenennen beim Update
/// meldet das System sonst den neuen Namen der alten Datei).
static OWN_EXE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();

fn own_exe() -> Option<PathBuf> {
    OWN_EXE.get_or_init(|| std::env::current_exe().ok()).clone()
}

/// Beim Start: Pfad merken und die alte Programmdatei eines Updates wegräumen.
pub fn launcher_startup() {
    if let Some(me) = own_exe() {
        let _ = fs::remove_file(me.with_extension("old"));
    }
}

/// Neuere veröffentlichte Werkbank-Version, falls es eine gibt.
pub fn launcher_update_available() -> Result<Option<Version>> {
    let Some(own) = own_version() else { return Ok(None) };
    let latest = latest_version(&launcher_app()?)?;
    Ok((latest > own).then_some(latest))
}

fn launcher_asset() -> &'static str {
    if cfg!(windows) {
        "werkbank-windows-x64.zip"
    } else if cfg!(target_os = "macos") {
        "werkbank-macos-aarch64.zip"
    } else {
        "werkbank-linux-x86_64.tar.gz"
    }
}

/// Prüfsumme einer Datei aus SHA256SUMS.txt.
fn expected_sum(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (sum, name) = l.split_once(char::is_whitespace)?;
        (name.trim().trim_start_matches('*') == file).then(|| sum.to_lowercase())
    })
}

/// Lädt Werkbank `v`, prüft die Prüfsumme und tauscht die eigene Programmdatei aus. Die
/// laufende Datei wird zu «.old» umbenannt (das erlaubt auch Windows) und beim nächsten Start
/// gelöscht. Geht etwas schief, bleibt die bisherige Datei an ihrem Platz.
pub fn update_launcher(v: Version, progress: Progress) -> Result<()> {
    let app = launcher_app()?;
    let file = launcher_asset();
    let sums = release_text(&app, v, "SHA256SUMS.txt")?;
    let expected = expected_sum(&sums, file).ok_or_else(|| anyhow!("{file} fehlt im Release werkbank-v{}", fmt_version(v)))?;
    let work = data_dir().join("werkbank-update");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work)?;
    let archive = work.join(file);
    let actual = download(release_file(&app, v, file)?, &archive, progress)?;
    if actual != expected {
        let _ = fs::remove_dir_all(&work);
        bail!("Prüfsumme von {file} stimmt nicht; Download verworfen");
    }
    progress(crate::sprache::t("Entpacke …"), 1.0);
    let out = work.join("neu");
    extract(&archive, &out)?;
    let name = if cfg!(windows) { "werkbank.exe" } else { "werkbank" };
    let new_exe = [out.join("werkbank").join(name), out.join(name), out.join("Werkbank.app/Contents/MacOS").join(name)]
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| anyhow!("{name} fehlt im Archiv"))?;
    let me = own_exe().ok_or_else(|| anyhow!("eigene Programmdatei nicht gefunden"))?;
    let staged = me.with_extension("neu");
    fs::copy(&new_exe, &staged).with_context(|| format!("{} schreiben", staged.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))?;
    }
    let old = me.with_extension("old");
    let _ = fs::remove_file(&old);
    fs::rename(&me, &old).with_context(|| format!("{} umbenennen", me.display()))?;
    if let Err(e) = fs::rename(&staged, &me) {
        let _ = fs::rename(&old, &me);
        return Err(e).context("neue Programmdatei einsetzen");
    }
    let _ = fs::remove_dir_all(&work);
    // Unix: die laufende alte Datei darf gleich weg (unter macOS muss sie aus dem Bündel heraus,
    // bevor es neu versiegelt wird).
    #[cfg(unix)]
    let _ = fs::remove_file(&old);
    // macOS: das Bündel neu (ad hoc) versiegeln, sonst passt seine Signatur nicht mehr zur Datei.
    if let Some(bundle) = bundle_of(&me) {
        let _ = std::process::Command::new("/usr/bin/codesign").args(["--force", "--deep", "--sign", "-"]).arg(&bundle).status();
    }
    Ok(())
}

/// Startet die (neue) Programmdatei und beendet diesen Launcher.
pub fn restart_launcher() -> Result<()> {
    let me = own_exe().ok_or_else(|| anyhow!("eigene Programmdatei nicht gefunden"))?;
    std::process::Command::new(&me).spawn().with_context(|| format!("{} starten", me.display()))?;
    std::process::exit(0);
}

/// Installiert ein selbst gebautes Archiv (z. B. deine angepasste Version) ohne Download.
/// Die Version wird aus dem Dateinamen gelesen, etwa `pdfcraft-0.7.2-linux-x86_64.tar.gz`.
pub fn install_file(app: &App, archive: &Path, progress: Progress) -> Result<(Kind, Version)> {
    let name = archive.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let v = name
        .split('-')
        .find_map(|part| parse_version(part.trim_end_matches(".zip")))
        .ok_or_else(|| anyhow!("keine Version im Dateinamen {name} gefunden"))?;
    let kind = if name.contains("-web-") { Kind::Web } else { Kind::Native };
    progress(&crate::sprache::tf("Prüfsumme: {s}", &[("s", &sha256_file(archive)?[..12].to_string())]), 0.5);
    fs::create_dir_all(app_dir(app))?;
    activate(app, kind, v, archive, progress)?;
    Ok((kind, v))
}

/// Ein Archiv im Bündel-Ordner des Offline-Setups: Programm, Art, Version, Datei.
pub struct BundleEntry {
    pub app: App,
    pub kind: Kind,
    pub version: Version,
    pub path: PathBuf,
}

/// Liest den Bündel-Ordner (`<app>-<ver>-…-portable.zip`, `<app>-<ver>-linux-….tar.gz`, `<app>-web-<ver>.zip`).
pub fn bundle_entries(manifest: &Manifest, dir: &Path) -> Vec<BundleEntry> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(version) = name.split('-').find_map(|part| parse_version(part.trim_end_matches(".zip"))) else { continue };
        let kind = if name.contains("-web-") { Kind::Web } else { Kind::Native };
        let Some(app) = manifest.app.iter().filter(|a| a.fremd.is_none()).find(|a| name.starts_with(&format!("{}-", a.asset()))) else { continue };
        if name.ends_with(".zip") || name.ends_with(".tar.gz") {
            out.push(BundleEntry { app: app.clone(), kind, version, path });
        }
    }
    out.sort_by(|a, b| a.app.id.cmp(&b.app.id).then(a.kind.key().cmp(b.kind.key())));
    out
}

/// Richtet ein Bündel ein. Mit `online` wird pro Programm zuerst nach einer neueren Version im
/// Netz gesucht und diese genommen; sonst (oder ohne Netz) kommt die Version aus dem Bündel.
/// `report` erhält je Programm Fortschritt, `done` das Ergebnis. Eingerichtete Archive werden
/// gelöscht, der leere Bündel-Ordner zum Schluss auch.
pub fn install_bundle(
    manifest: &Manifest,
    dir: &Path,
    online: bool,
    report: &(dyn Fn(&str, &str, f32) + Sync),
    done: &(dyn Fn(&App, Kind, Result<Version>) + Sync),
) {
    for e in bundle_entries(manifest, dir) {
        let id = e.app.id.clone();
        let progress = |msg: &str, frac: f32| report(&id, msg, frac);
        let mut result = None;
        if online {
            report(&id, "Suche neuere Version …", 0.0);
            if let Ok(latest) = latest_version(&e.app) {
                if latest > e.version {
                    report(&id, &format!("Neuere Version {} gefunden", fmt_version(latest)), 0.0);
                    result = install(&e.app, e.kind, &progress).ok();
                }
            }
        }
        let result = match result {
            Some(v) => Ok(v),
            None => install_file(&e.app, &e.path, &progress).map(|(_, v)| v),
        };
        if result.is_ok() {
            let _ = fs::remove_file(&e.path);
        }
        done(&e.app, e.kind, result);
    }
    let _ = fs::remove_dir(dir);
}

pub fn uninstall(app: &App) -> Result<()> {
    if app.fremd.is_some() {
        // Nie selbst löschen: Fremdprogramme deinstalliert das System.
        bail!("{} wird in den Einstellungen des Systems deinstalliert", app.name);
    }
    let dir = app_dir(app);
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

pub fn run(app: &App) -> Result<()> {
    run_with(app, None)
}

/// Startet das Programm, optional mit einer zu öffnenden Datei.
pub fn run_with(app: &App, file: Option<&Path>) -> Result<()> {
    if let Some(f) = &app.fremd {
        return fremd_run(f, file);
    }
    let exe = exe_path(app).ok_or_else(|| anyhow!("{} ist nicht installiert", app.name))?;
    let file = file.map(|f| std::path::absolute(f).unwrap_or_else(|_| f.to_path_buf()));
    if let Some(f) = &file {
        remember_recent(f, &app.id);
    }
    std::process::Command::new(&exe)
        .args(&file)
        // Sprache der Oberfläche, wie im Launcher gewählt.
        .env("WERKBANK_LANG", crate::sprache::lang().code())
        .env("WERKBANK_THEME", crate::gui::theme_code())
        .current_dir(exe.parent().unwrap_or(Path::new(".")))
        .spawn()
        .with_context(|| format!("Start von {}", exe.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Fremdprogramme: finden, mit dem offiziellen Installationsprogramm einrichten, starten.
// Der Launcher aktualisiert und löscht sie nie; das tun die Programme bzw. das System selbst.

/// Setzt `%NAME%` durch Umgebungsvariablen. None, wenn eine fehlt oder ein `%` offen bleibt.
pub fn expand_vars(s: &str, var: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let parts: Vec<&str> = s.split('%').collect();
    if parts.len() % 2 == 0 {
        return None;
    }
    let mut out = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i % 2 == 0 {
            out.push_str(part);
        } else {
            out.push_str(&var(part).filter(|v| !v.is_empty())?);
        }
    }
    Some(out)
}

/// `~/…` → Benutzerordner.
fn expand_home(s: &str) -> Option<PathBuf> {
    match s.strip_prefix("~/") {
        Some(rest) => Some(PathBuf::from(std::env::var_os("HOME")?).join(rest)),
        None if s == "~" => std::env::var_os("HOME").map(PathBuf::from),
        None => Some(PathBuf::from(s)),
    }
}

/// Befehl im PATH suchen (nur ausführbare Dateien).
fn which(cmd: &str) -> Option<PathBuf> {
    if cmd.is_empty() || cmd.contains('/') {
        return None;
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(cmd)).find(|p| executable(p))
}

#[cfg(unix)]
fn executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable(p: &Path) -> bool {
    p.is_file()
}

/// Dateiname am Ende einer Adresse, nur aus harmlosen Zeichen.
fn file_name_of(url: &str, fallback: &str) -> String {
    let last = url.split(['?', '#']).next().unwrap_or("").rsplit('/').next().unwrap_or("");
    let clean: String = last.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')).collect();
    if clean.is_empty() || clean.starts_with('.') {
        fallback.to_string()
    } else {
        clean
    }
}

/// Eigener Ordner eines Fremdprogramms im Datenordner (Linux: dort liegt das AppImage).
fn fremd_dir(f: &Fremd) -> Option<PathBuf> {
    let ok = !f.id.is_empty() && f.id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
    ok.then(|| data_dir().join("fremd").join(&f.id))
}

/// Das von der Werkbank eingerichtete AppImage (Linux), falls vorhanden.
fn fremd_appimage(f: &Fremd) -> Option<PathBuf> {
    let dir = fremd_dir(f)?;
    let mut found: Vec<PathBuf> = fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().ends_with(".appimage")))
        .collect();
    found.sort();
    found.pop()
}

/// Wo das Fremdprogramm installiert ist: Programmdatei (Windows), .app-Bündel (macOS),
/// eigenes AppImage oder Befehl im PATH (Linux und andere). Nur ein paar Dateiabfragen.
pub fn fremd_find(f: &Fremd) -> Option<PathBuf> {
    match std::env::consts::OS {
        "windows" => f
            .pfade_windows
            .iter()
            .filter_map(|p| expand_vars(p, &|k: &str| std::env::var(k).ok()))
            .map(PathBuf::from)
            .find(|p| p.is_file()),
        "macos" => f.pfade_macos.iter().filter_map(|p| expand_home(p)).find(|p| p.is_dir()),
        _ => fremd_appimage(f).or_else(|| f.befehle_linux.iter().find_map(|c| which(c))),
    }
}

/// Installierte Version, wo sie sich billig lesen lässt: Name des eigenen AppImage (Linux) oder
/// CFBundleShortVersionString des .app-Bündels (macOS). Nur bei Programmen mit GitHub-Quelle.
pub fn fremd_version(f: &Fremd) -> Option<Version> {
    f.github.as_ref()?;
    match std::env::consts::OS {
        "macos" => plist_version(&fremd_find(f)?),
        "windows" => None,
        _ => {
            let file = fremd_appimage(f)?;
            let name = file.file_name()?.to_string_lossy().into_owned();
            version_from_name(f.datei()?, &name)
        }
    }
}

/// Version aus `Contents/Info.plist` eines .app-Bündels (XML-Fassung).
fn plist_version(app: &Path) -> Option<Version> {
    let text = fs::read_to_string(app.join("Contents").join("Info.plist")).ok()?;
    let after = text.split("<key>CFBundleShortVersionString</key>").nth(1)?;
    let v = after.split("<string>").nth(1)?.split("</string>").next()?;
    parse_version(v)
}

/// Version aus einem Dateinamen nach Muster («KeePassXC-{v}-x86_64.AppImage»).
pub fn version_from_name(pattern: &str, name: &str) -> Option<Version> {
    let (pre, post) = pattern.split_once("{v}")?;
    parse_version(name.strip_prefix(pre)?.strip_suffix(post)?)
}

/// Muster mit Release-Tag füllen: `{v}` = Tag ohne führendes «v», `{tag}` = Tag.
pub fn fill_pattern(pattern: &str, tag: &str) -> String {
    pattern.replace("{tag}", tag).replace("{v}", tag.strip_prefix('v').unwrap_or(tag))
}

/// Erste Prüfsumme (64 Hex-Zeichen) in einer .DIGEST-/SHA256-Datei.
pub fn parse_digest(text: &str) -> Option<String> {
    text.split_whitespace().find(|t| t.len() == 64 && t.chars().all(|c| c.is_ascii_hexdigit())).map(str::to_lowercase)
}

fn safe_tag(t: &str) -> Option<String> {
    let t = t.trim();
    (!t.is_empty() && t.len() < 100 && t.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))).then(|| t.to_string())
}

/// Tag aus der Weiterleitung von …/releases/latest (…/releases/tag/2.8.0).
pub fn tag_from_location(loc: &str) -> Option<String> {
    safe_tag(loc.split("/releases/tag/").nth(1)?.split(['?', '#', '/']).next()?)
}

/// Neuester stabiler Release-Tag eines GitHub-Repositorys: zuerst über die Weiterleitung von
/// github.com/<repo>/releases/latest (ohne Abfragegrenze), sonst über die GitHub-API.
pub fn latest_tag(repo: &str) -> Result<String> {
    let repo = repo.trim().trim_matches('/');
    let valid = repo.split('/').count() == 2
        && !repo.contains("..")
        && repo.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'));
    if !valid {
        bail!("ungültiges GitHub-Repository: {repo}");
    }
    if let Ok(resp) = agent().get(&format!("https://github.com/{repo}/releases/latest")).call() {
        if resp.status().is_redirection() {
            if let Some(tag) = resp.headers().get("location").and_then(|h| h.to_str().ok()).and_then(tag_from_location) {
                return Ok(tag);
            }
        }
    }
    let api = format!("https://api.github.com/repos/{repo}/releases/latest");
    let mut resp = match fetch_with(&api, Some("application/vnd.github+json"), false) {
        Ok(r) => r,
        Err(e) => {
            let text = format!("{e:#}");
            if text.contains("403") || text.contains("429") {
                bail!("GitHub lässt gerade keine weiteren Abfragen zu (Abfragegrenze); bitte später nochmals versuchen");
            }
            return Err(e).context("neueste Version bei GitHub abfragen");
        }
    };
    let mut buf = Vec::new();
    resp.body_mut().as_reader().take(1 << 20).read_to_end(&mut buf)?;
    let json: serde_json::Value = serde_json::from_slice(&buf).context("Antwort der GitHub-API")?;
    json["tag_name"].as_str().and_then(safe_tag).ok_or_else(|| anyhow!("kein Release bei {repo} gefunden"))
}

/// Neueste Version eines Fremdprogramms mit GitHub-Quelle.
pub fn fremd_latest(f: &Fremd) -> Result<Version> {
    let repo = f.github.as_deref().ok_or_else(|| anyhow!("keine GitHub-Quelle"))?;
    let tag = latest_tag(repo)?;
    parse_version(tag.strip_prefix('v').unwrap_or(&tag)).ok_or_else(|| anyhow!("Tag {tag} ist keine Version"))
}

/// Woher das Installationsprogramm kommt: Adresse und Dateiname.
struct Quelle {
    url: String,
    file: String,
}

fn fremd_source(f: &Fremd) -> Result<Quelle> {
    if let (Some(repo), Some(pattern)) = (f.github.as_deref(), f.datei()) {
        let tag = latest_tag(repo)?;
        let file = file_name_of(&fill_pattern(pattern, &tag), "");
        if file.is_empty() {
            bail!("ungültiges Dateimuster: {pattern}");
        }
        let url = format!("https://github.com/{}/releases/download/{tag}/{file}", repo.trim().trim_matches('/'));
        return Ok(Quelle { url, file });
    }
    let url = match std::env::consts::OS {
        "windows" => f.windows.as_deref(),
        "macos" => f.macos.as_deref(),
        _ => None,
    }
    .filter(|u| u.starts_with("https://"))
    .ok_or_else(|| anyhow!("{} wird auf diesem System über die Paketverwaltung installiert", f.name))?;
    let fallback = if cfg!(windows) { "setup.exe" } else { "setup.dmg" };
    Ok(Quelle { url: url.to_string(), file: file_name_of(url, fallback) })
}

/// Lädt das offizielle Installationsprogramm (mit Prüfsumme, wenn es eine gibt) und richtet
/// das Programm ein: .exe/.msi ausführen (Windows), .app aus dem .dmg kopieren (macOS),
/// AppImage im Datenordner ablegen (Linux). Auch für Aktualisierungen: installiert neu.
pub fn fremd_install(f: &Fremd, progress: Progress) -> Result<()> {
    progress(crate::sprache::t("Suche neueste Version …"), 0.0);
    let q = fremd_source(f)?;
    let expected = match f.digest.as_deref().filter(|d| !d.is_empty()) {
        Some(suffix) => fetch_with(&format!("{}{suffix}", q.url), None, false).ok().and_then(|mut r| {
            let mut buf = Vec::new();
            r.body_mut().as_reader().take(64 << 10).read_to_end(&mut buf).ok()?;
            parse_digest(&String::from_utf8_lossy(&buf))
        }),
        None => None,
    };
    let appimage = q.file.to_lowercase().ends_with(".appimage");
    let dir = if appimage { fremd_dir(f).ok_or_else(|| anyhow!("ungültige Kennung {}", f.id))? } else { data_dir().join("downloads") };
    fs::create_dir_all(&dir).with_context(|| format!("{} anlegen", dir.display()))?;
    let file = if appimage { dir.join(format!("{}.teil", q.file)) } else { dir.join(format!("{}-{}", f.id, q.file)) };
    progress(crate::sprache::t("Lade herunter …"), 0.0);
    // Ohne GitHub-Schlüssel: der gehört nur zur Werkbank, nicht zu fremden Downloads.
    let loaded = fetch_with(&q.url, None, false).and_then(|resp| download(resp, &file, progress));
    let checked = loaded.and_then(|actual| match expected {
        Some(e) if e != actual => Err(anyhow!("Prüfsumme von {} stimmt nicht; Download verworfen", q.file)),
        _ => Ok(()),
    });
    if let Err(e) = checked {
        let _ = fs::remove_file(&file);
        return Err(e);
    }
    if appimage {
        return place_appimage(&dir, &file, &q.file);
    }
    progress(crate::sprache::t("Installationsprogramm läuft …"), 1.0);
    let lower = q.file.to_lowercase();
    let result = if lower.ends_with(".msi") {
        run_msi(&file)
    } else if lower.ends_with(".dmg") {
        install_dmg(&file)
    } else if lower.ends_with(".exe") {
        run_installer(&file)
    } else {
        Err(anyhow!("unbekannte Art von Installationsprogramm: {}", q.file))
    };
    let _ = fs::remove_file(&file);
    result?;
    if fremd_find(f).is_none() {
        bail!("{} wurde nicht gefunden – Installation abgebrochen?", f.name);
    }
    Ok(())
}

/// Linux: geladenes AppImage ausführbar machen und an die Stelle des alten setzen.
fn place_appimage(dir: &Path, part: &Path, name: &str) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(part, fs::Permissions::from_mode(0o755))?;
    }
    let final_path = dir.join(name);
    fs::rename(part, &final_path).with_context(|| format!("{} ablegen", final_path.display()))?;
    // Ältere AppImages dieses Programms wegräumen (sie gehören der Werkbank).
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p != final_path && p.is_file() {
            let _ = fs::remove_file(&p);
        }
    }
    Ok(())
}

/// Windows: .msi mit dem Windows-Installer ausführen (eigene Oberfläche und UAC) und warten.
fn run_msi(msi: &Path) -> Result<()> {
    let windir = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let msiexec = PathBuf::from(windir).join("System32").join("msiexec.exe");
    let exe = if msiexec.is_file() { msiexec } else { PathBuf::from("msiexec") };
    std::process::Command::new(exe).arg("/i").arg(msi).status().context("msiexec starten")?;
    Ok(())
}

/// Windows: Installationsprogramm starten und warten. Verlangt es Administratorrechte
/// (Fehler 740), über PowerShell mit UAC-Abfrage.
fn run_installer(exe: &Path) -> Result<()> {
    match std::process::Command::new(exe).status() {
        Ok(_) => Ok(()),
        Err(e) if e.raw_os_error() == Some(740) => {
            let script = format!(
                "$p = Start-Process -FilePath '{}' -Verb RunAs -Wait -PassThru; exit $p.ExitCode",
                exe.display().to_string().replace('\'', "''")
            );
            std::process::Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                .status()
                .context("Installationsprogramm mit Administratorrechten starten")?;
            Ok(())
        }
        Err(e) => Err(e).with_context(|| format!("{} starten", exe.display())),
    }
}

/// macOS: Disk-Image einhängen, das .app-Bündel mit ditto kopieren, wieder aushängen.
fn install_dmg(dmg: &Path) -> Result<()> {
    let mount = std::env::temp_dir().join(format!("werkbank-dmg-{}", std::process::id()));
    let _ = fs::create_dir_all(&mount);
    let status = std::process::Command::new("/usr/bin/hdiutil")
        .args(["attach", "-nobrowse", "-readonly", "-noautoopen", "-mountpoint"])
        .arg(&mount)
        .arg(dmg)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .status()
        .context("hdiutil attach")?;
    if !status.success() {
        let _ = fs::remove_dir(&mount);
        bail!("Disk-Image liess sich nicht öffnen");
    }
    let result = copy_app(&mount);
    let detached = std::process::Command::new("/usr/bin/hdiutil").arg("detach").arg(&mount).arg("-quiet").status().is_ok_and(|s| s.success());
    if !detached {
        let _ = std::process::Command::new("/usr/bin/hdiutil").arg("detach").arg(&mount).arg("-force").arg("-quiet").status();
    }
    let _ = fs::remove_dir(&mount);
    result
}

fn copy_app(volume: &Path) -> Result<()> {
    let app = fs::read_dir(volume)
        .context("Disk-Image lesen")?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "app") && p.is_dir())
        .ok_or_else(|| anyhow!("kein Programm (.app) im Disk-Image"))?;
    let name = app.file_name().ok_or_else(|| anyhow!("Programmname fehlt"))?;
    let system = PathBuf::from("/Applications");
    let dest_dir = if writable(&system) {
        system
    } else {
        let home = expand_home("~/Applications").ok_or_else(|| anyhow!("Benutzerordner unbekannt"))?;
        fs::create_dir_all(&home).with_context(|| format!("{} anlegen", home.display()))?;
        home
    };
    let dest = dest_dir.join(name);
    // Erst daneben kopieren; eine ältere Fassung kommt danach in den Papierkorb.
    let fresh = dest_dir.join(format!(".{}.neu", name.to_string_lossy()));
    let _ = fs::remove_dir_all(&fresh);
    let ok = std::process::Command::new("/usr/bin/ditto").arg(&app).arg(&fresh).status().context("ditto")?.success();
    if !ok {
        let _ = fs::remove_dir_all(&fresh);
        bail!("{} liess sich nicht nach {} kopieren", name.to_string_lossy(), dest_dir.display());
    }
    if dest.exists() {
        let trash = expand_home("~/.Trash").ok_or_else(|| anyhow!("Benutzerordner unbekannt"))?;
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let old = trash.join(format!("{} {stamp}.app", name.to_string_lossy().trim_end_matches(".app")));
        if let Err(e) = fs::rename(&dest, &old) {
            let _ = fs::remove_dir_all(&fresh);
            return Err(e).with_context(|| format!("alte Fassung {} in den Papierkorb legen", dest.display()));
        }
    }
    fs::rename(&fresh, &dest).with_context(|| format!("{} einsetzen", dest.display()))?;
    Ok(())
}

/// Startet das Fremdprogramm, losgelöst wie die anderen Programme.
pub fn fremd_run(f: &Fremd, file: Option<&Path>) -> Result<()> {
    let found = fremd_find(f).ok_or_else(|| anyhow!("{} ist nicht installiert", f.name))?;
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("/usr/bin/open");
        c.arg("-a").arg(&found);
        c
    } else {
        let mut c = std::process::Command::new(&found);
        if let Some(dir) = found.parent().filter(|_| cfg!(windows)) {
            c.current_dir(dir);
        }
        c
    };
    cmd.args(file).spawn().with_context(|| format!("Start von {}", found.display()))?;
    Ok(())
}

/// «Deinstallieren»: zeigt den Ort, an dem das System das Programm entfernt
/// (Windows: Einstellungen «Apps», macOS: Bündel im Finder). Löscht selbst nichts – ausser
/// einem AppImage, das die Werkbank selbst im Datenordner abgelegt hat (dann `true`).
pub fn fremd_uninstall_help(f: &Fremd) -> Result<bool> {
    if let (false, false, Some(_)) = (cfg!(windows), cfg!(target_os = "macos"), fremd_appimage(f)) {
        let dir = fremd_dir(f).ok_or_else(|| anyhow!("ungültige Kennung {}", f.id))?;
        fs::remove_dir_all(&dir).with_context(|| format!("{} entfernen", dir.display()))?;
        return Ok(true);
    }
    match std::env::consts::OS {
        "windows" => open::that("ms-settings:appsfeatures").context("Einstellungen öffnen")?,
        "macos" => {
            let found = fremd_find(f).ok_or_else(|| anyhow!("{} ist nicht installiert", f.name))?;
            std::process::Command::new("/usr/bin/open").arg("-R").arg(&found).spawn().context("Finder öffnen")?;
        }
        _ => {}
    }
    Ok(false)
}

// ---------------------------------------------------------------------------------------------
// Lokaler Webserver für die Browser-Versionen

fn mime(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript",
        "wasm" => "application/wasm",
        "css" => "text/css",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn portal(manifest: &Manifest) -> String {
    let mut items = String::new();
    for app in &manifest.app {
        if let Some(v) = installed_version(app, Kind::Web) {
            items += &format!(
                "<a href=\"/{}/\"><b>{}</b><span>{}</span><small>Version {}</small></a>",
                app.id,
                app.name,
                app.about,
                fmt_version(v)
            );
        }
    }
    if items.is_empty() {
        items = "<p>Noch keine Browser-Version installiert. Im Launcher bei einem Programm «Im Browser» wählen.</p>".into();
    }
    format!(
        "<!doctype html><html lang=de><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'>\
<title>{t}</title><style>body{{font:16px system-ui,sans-serif;margin:0;padding:32px 16px;background:#16171b;color:#eceef2}}\
h1{{max-width:900px;margin:0 auto 24px}}main{{max-width:900px;margin:auto;display:grid;gap:12px;grid-template-columns:repeat(auto-fill,minmax(250px,1fr))}}\
a{{display:flex;flex-direction:column;gap:6px;padding:18px;border-radius:12px;background:#23252c;color:inherit;text-decoration:none;border:1px solid #33363f}}\
a:hover{{border-color:#7bb51c}}b{{font-size:19px}}span{{color:#b4b8c2}}small{{color:#7d8290}}</style><h1>{t}</h1><main>{items}</main>",
        t = manifest.title
    )
}

/// Startet den Webserver auf 127.0.0.1 und gibt die Adresse zurück. Läuft in eigenen Threads.
pub fn serve_web(manifest: Manifest) -> Result<String> {
    let server = (4780..4800)
        .find_map(|port| tiny_http::Server::http(("127.0.0.1", port)).ok())
        .ok_or_else(|| anyhow!("kein freier Port für den lokalen Webserver"))?;
    let addr = format!("http://{}", server.server_addr());
    std::thread::spawn(move || {
        for req in server.incoming_requests() {
            let path = req.url().split(['?', '#']).next().unwrap_or("/").to_string();
            let mut parts = path.trim_start_matches('/').splitn(2, '/');
            let id = parts.next().unwrap_or("");
            let rest = parts.next();
            let header = |k: &str, v: &str| tiny_http::Header::from_bytes(k, v).unwrap();
            if id.is_empty() {
                let page = portal(&manifest);
                let _ = req.respond(
                    tiny_http::Response::from_string(page).with_header(header("Content-Type", mime(Path::new("x.html")))),
                );
                continue;
            }
            let root = manifest.app.iter().find(|a| a.id == id).and_then(web_root);
            let (Some(root), Some(rest)) = (root, rest) else {
                // "/pdfcraft" ohne Schrägstrich: weiterleiten, damit relative Pfade stimmen.
                let resp = if rest.is_none() && manifest.app.iter().any(|a| a.id == id) {
                    tiny_http::Response::empty(302).with_header(header("Location", &format!("/{id}/")))
                } else {
                    tiny_http::Response::empty(404)
                };
                let _ = req.respond(resp);
                continue;
            };
            let rel = if rest.is_empty() { "index.html" } else { rest };
            // Nur einfache Pfadteile zulassen, damit nichts ausserhalb des App-Ordners lesbar ist.
            let safe = Path::new(rel).components().all(|c| matches!(c, std::path::Component::Normal(_)));
            let file = root.join(rel);
            match (safe, fs::File::open(&file)) {
                (true, Ok(f)) if file.is_file() => {
                    let _ = req.respond(
                        tiny_http::Response::from_file(f)
                            .with_header(header("Content-Type", mime(&file)))
                            .with_header(header("X-Content-Type-Options", "nosniff"))
                            .with_header(header("Cache-Control", "no-cache")),
                    );
                }
                _ => {
                    let _ = req.respond(tiny_http::Response::empty(404));
                }
            }
        }
    });
    Ok(addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variablen_einsetzen() {
        let var = |k: &str| match k {
            "LOCALAPPDATA" => Some(r"C:\Users\oli\AppData\Local".to_string()),
            "ProgramFiles(x86)" => Some(r"C:\Program Files (x86)".to_string()),
            _ => None,
        };
        assert_eq!(expand_vars(r"%LOCALAPPDATA%\Brave\brave.exe", &var).as_deref(), Some(r"C:\Users\oli\AppData\Local\Brave\brave.exe"));
        assert_eq!(expand_vars(r"%ProgramFiles(x86)%\b.exe", &var).as_deref(), Some(r"C:\Program Files (x86)\b.exe"));
        assert_eq!(expand_vars(r"%FEHLT%\b.exe", &var), None);
        assert_eq!(expand_vars(r"%offen\b.exe", &var), None);
        assert_eq!(expand_vars(r"C:\b.exe", &var).as_deref(), Some(r"C:\b.exe"));
    }

    #[test]
    fn dateiname_aus_adresse() {
        assert_eq!(file_name_of("https://x/y/BraveBrowserStandaloneSetup.exe?a=1", "s.exe"), "BraveBrowserStandaloneSetup.exe");
        assert_eq!(file_name_of("https://x/y/", "s.exe"), "s.exe");
        assert_eq!(file_name_of("https://x/..", "s.exe"), "s.exe");
    }

    #[test]
    fn fremdprogramme_in_der_liste() {
        let mut m: Manifest = toml::from_str(DEFAULT_MANIFEST).expect("eingebaute Liste");
        let brave = m.fremd.iter().find(|f| f.id == "brave").cloned().expect("Brave");
        assert!(brave.homepage.is_some() && !brave.pfade_windows.is_empty());
        let before = m.app.len();
        add_fremd(&mut m);
        add_fremd(&mut m);
        let fremd: Vec<&App> = m.app.iter().filter(|a| a.fremd.is_some()).collect();
        assert_eq!(m.app.len(), before + fremd.len());
        assert!(fremd.len() <= m.fremd.len());
        for a in fremd {
            assert!(installed_version(a, Kind::Native).is_none());
            assert!(latest_version(a).is_err());
            assert!(uninstall(a).is_err());
            assert!(a.types.is_empty() && a.no_web);
        }
    }

    #[test]
    fn release_dateien() {
        assert_eq!(fill_pattern("KeePassXC-{v}-x64.msi", "2.8.0"), "KeePassXC-2.8.0-x64.msi");
        assert_eq!(fill_pattern("app-{v}-{tag}.zip", "v1.2.3"), "app-1.2.3-v1.2.3.zip");
        assert_eq!(version_from_name("KeePassXC-{v}-x86_64.AppImage", "KeePassXC-2.8.0-x86_64.AppImage"), Some((2, 8, 0)));
        assert_eq!(version_from_name("KeePassXC-{v}-x86_64.AppImage", "KeePassXC-2.8.0-aarch64.AppImage"), None);
        assert_eq!(version_from_name("ohne-platzhalter", "ohne-platzhalter"), None);
        assert_eq!(tag_from_location("https://github.com/keepassxreboot/keepassxc/releases/tag/2.8.0").as_deref(), Some("2.8.0"));
        assert_eq!(tag_from_location("https://github.com/a/b/releases/tag/v1.0.0?x=1").as_deref(), Some("v1.0.0"));
        assert_eq!(tag_from_location("https://github.com/a/b/releases"), None);
        assert_eq!(tag_from_location("https://github.com/a/b/releases/tag/..%2F"), None);
        assert!(latest_tag("nicht gültig").is_err());
        assert!(latest_tag("a/../b").is_err());
    }

    #[test]
    fn pruefsumme_lesen() {
        let d = "4f105af5ba780428718c8be5614cf95deceed5cb3b474a9bd624c5df81cec6e5  KeePassXC-2.8.0-x64.msi\n";
        assert_eq!(parse_digest(d).as_deref(), Some("4f105af5ba780428718c8be5614cf95deceed5cb3b474a9bd624c5df81cec6e5"));
        let upper = "SHA256 (x) = 4F105AF5BA780428718C8BE5614CF95DECEED5CB3B474A9BD624C5DF81CEC6E5";
        assert_eq!(parse_digest(upper).as_deref(), Some("4f105af5ba780428718c8be5614cf95deceed5cb3b474a9bd624c5df81cec6e5"));
        assert_eq!(parse_digest("<html>Not Found</html>"), None);
    }

    #[test]
    fn keepassxc_eintrag() {
        let m: Manifest = toml::from_str(DEFAULT_MANIFEST).expect("eingebaute Liste");
        let k = m.fremd.iter().find(|f| f.id == "keepassxc").expect("KeePassXC");
        assert_eq!(k.github.as_deref(), Some("keepassxreboot/keepassxc"));
        assert_eq!(k.digest.as_deref(), Some(".DIGEST"));
        assert!(k.datei().is_some_and(|p| p.contains("{v}")));
        assert!(k.can_install() && k.available());
        let brave = m.fremd.iter().find(|f| f.id == "brave").expect("Brave");
        assert!(brave.datei().is_none());
    }
}
