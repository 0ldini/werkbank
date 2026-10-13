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
    pub repo: String,
    pub asset: Option<String>,
    pub exe: Option<String>,
    pub pin: Option<String>,
    pub color: Option<String>,
    /// Anfang der Release-Tags, z. B. "pdfcraft-v" (Standard: "v").
    pub tag: Option<String>,
    /// true: nur Browser-Version (z. B. eigene HTML5-Apps), keine Desktop-Programmdatei.
    #[serde(default)]
    pub web_only: bool,
}

impl App {
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

/// Datenordner. Liegt neben dem Launcher eine Datei `portable.txt`, bleibt alles
/// im Ordner `data` daneben (USB-Stick); sonst im Benutzerordner des Systems.
pub fn data_dir() -> PathBuf {
    if let Ok(d) = std::env::var("WERKBANK_DATA") {
        return PathBuf::from(d);
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        if dir.join("portable.txt").exists() {
            return dir.join("data");
        }
    }
    directories::ProjectDirs::from("", "", "Werkbank")
        .map(|p| p.data_local_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("werkbank-data"))
}

/// Wo die aktuelle Programmliste liegt. Neue Programme erscheinen so ohne neuen Launcher.
pub const MANIFEST_URL: &str = "https://raw.githubusercontent.com/0ldini/werkbank/main/launcher/apps.toml";

/// Lädt die Programmliste. Reihenfolge: eigene `apps.toml` im Datenordner (falls vorhanden),
/// sonst die aktuelle Liste aus dem Netz, sonst deren letzte gespeicherte Fassung, sonst die
/// in den Launcher eingebaute.
pub fn load_manifest() -> Result<Manifest> {
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
        .build()
        .into()
}

fn http_text(url: &str, limit: u64) -> Result<String> {
    let mut resp = agent().get(url).call().with_context(|| format!("Abruf von {url}"))?;
    let mut buf = Vec::new();
    resp.body_mut().as_reader().take(limit).read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
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

/// Neueste Version, zu der tatsächlich ein Release mit Dateien existiert.
pub fn latest_version(app: &App) -> Result<Version> {
    if let Some(pin) = &app.pin {
        return parse_version(pin).ok_or_else(|| anyhow!("pin \"{pin}\" ist keine Version"));
    }
    for v in remote_versions(app)?.into_iter().take(5) {
        if http_text(&release_url(app, v, "SHA256SUMS.txt"), 1 << 20).is_ok() {
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
    [root.join(&exe), root.join("bin").join(&exe)].into_iter().find(|p| p.is_file())
}

/// Fortschritt: Text und Anteil 0..1.
pub type Progress<'a> = &'a (dyn Fn(&str, f32) + Sync);

fn download(url: &str, dest: &Path, progress: Progress) -> Result<String> {
    let mut resp = agent().get(url).call().with_context(|| format!("Download {url}"))?;
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
        progress(&format!("Lade herunter … {mb:.0} MB"), if total > 0.0 { done / total } else { 0.0 });
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
    progress("Entpacke …", 1.0);
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
    progress("Suche neueste Version …", 0.0);
    let v = latest_version(app)?;
    if installed_version(app, kind) == Some(v) {
        return Ok(v);
    }
    let file = asset_name(app, kind, v)?;
    let sums = http_text(&release_url(app, v, "SHA256SUMS.txt"), 1 << 20)?;
    let expected = sums
        .lines()
        .find_map(|l| {
            let (sum, name) = l.split_once(char::is_whitespace)?;
            (name.trim().trim_start_matches('*') == file).then(|| sum.to_lowercase())
        })
        .ok_or_else(|| anyhow!("{file} fehlt im Release v{}", fmt_version(v)))?;

    fs::create_dir_all(app_dir(app))?;
    let archive = app_dir(app).join(&file);
    let actual = download(&release_url(app, v, &file), &archive, progress)?;
    if actual != expected {
        let _ = fs::remove_file(&archive);
        bail!("Prüfsumme von {file} stimmt nicht; Download verworfen");
    }
    let result = activate(app, kind, v, &archive, progress);
    let _ = fs::remove_file(&archive);
    result.map(|_| v)
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
    progress(&format!("Prüfsumme: {}", &sha256_file(archive)?[..12]), 0.5);
    fs::create_dir_all(app_dir(app))?;
    activate(app, kind, v, archive, progress)?;
    Ok((kind, v))
}

pub fn uninstall(app: &App) -> Result<()> {
    let dir = app_dir(app);
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

pub fn run(app: &App) -> Result<()> {
    let exe = exe_path(app).ok_or_else(|| anyhow!("{} ist nicht installiert", app.name))?;
    std::process::Command::new(&exe)
        .current_dir(exe.parent().unwrap_or(Path::new(".")))
        .spawn()
        .with_context(|| format!("Start von {}", exe.display()))?;
    Ok(())
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
