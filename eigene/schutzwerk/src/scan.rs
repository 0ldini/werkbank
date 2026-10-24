//! Virenscan über die Engine des Betriebssystems (defensiv): unter Windows Microsoft
//! Defender (`MpCmdRun.exe`, PowerShell-Status und -Verlauf), unter macOS/Linux ClamAV
//! (`clamscan`), falls installiert. Schutzwerk hat keine eigenen Signaturen.
//! Scans laufen als eigener Prozess im Hintergrund; die Ausgabe wird Zeile für Zeile gelesen.

use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::scanparse::{self, Bedrohung};
use crate::sys::{cmd, ps, sperre};

/// Höchstzahl gemerkter Ausgabezeilen eines Laufs.
const MAX_ZEILEN: usize = 20_000;

/// Zustand eines laufenden oder beendeten Scans.
pub struct Lauf {
    /// Deutscher Titel (wird in der Oberfläche übersetzt).
    pub titel: &'static str,
    pub zeilen: Vec<String>,
    pub fertig: bool,
    pub code: Option<i32>,
    pub fehler: Option<String>,
    pub start: Instant,
    pub dauer_s: u64,
    pub abgebrochen: bool,
    /// ClamAV statt Defender (andere Auswertung).
    pub clam: bool,
}

#[derive(Clone)]
pub struct Laeufer {
    pub lauf: Arc<Mutex<Option<Lauf>>>,
    kind: Arc<Mutex<Option<Child>>>,
}

fn zeilen_lesen(r: impl Read, lauf: Arc<Mutex<Option<Lauf>>>) {
    let mut br = BufReader::new(r);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match br.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let line = String::from_utf8_lossy(&buf).trim_end_matches(['\r', '\n']).replace('\0', "");
                if line.trim().is_empty() {
                    continue;
                }
                if let Some(l) = sperre(&lauf).as_mut() {
                    if l.zeilen.len() < MAX_ZEILEN {
                        l.zeilen.push(line);
                    }
                }
            }
        }
    }
}

impl Laeufer {
    pub fn new() -> Self {
        Laeufer { lauf: Arc::new(Mutex::new(None)), kind: Arc::new(Mutex::new(None)) }
    }

    pub fn laeuft(&self) -> bool {
        sperre(&self.lauf).as_ref().is_some_and(|l| !l.fertig)
    }

    /// Startet `prog args` im Hintergrund (nur, wenn nichts anderes läuft).
    pub fn starte(&self, titel: &'static str, prog: PathBuf, args: Vec<String>, clam: bool) {
        if self.laeuft() {
            return;
        }
        *sperre(&self.lauf) = Some(Lauf {
            titel,
            zeilen: Vec::new(),
            fertig: false,
            code: None,
            fehler: None,
            start: Instant::now(),
            dauer_s: 0,
            abgebrochen: false,
            clam,
        });
        let child = cmd(&prog).args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                if let Some(l) = sperre(&self.lauf).as_mut() {
                    l.fehler = Some(e.to_string());
                    l.fertig = true;
                }
                return;
            }
        };
        let out = child.stdout.take();
        let err = child.stderr.take();
        *sperre(&self.kind) = Some(child);
        let lauf = self.lauf.clone();
        let kind = self.kind.clone();
        std::thread::spawn(move || {
            let e_thread = err.map(|e| {
                let l = lauf.clone();
                std::thread::spawn(move || zeilen_lesen(e, l))
            });
            if let Some(o) = out {
                zeilen_lesen(o, lauf.clone());
            }
            if let Some(h) = e_thread {
                let _ = h.join();
            }
            let code = sperre(&kind).take().and_then(|mut c| c.wait().ok()).and_then(|s| s.code());
            if let Some(l) = sperre(&lauf).as_mut() {
                l.code = code;
                l.fertig = true;
                l.dauer_s = l.start.elapsed().as_secs();
            }
        });
    }

    /// Bricht den laufenden Scan ab (beendet den Scan-Prozess; Defender bekommt
    /// zusätzlich «-Scan -Cancel», weil der eigentliche Scan im Defender-Dienst läuft).
    pub fn abbrechen(&self) {
        let clam = sperre(&self.lauf).as_ref().is_none_or(|l| l.clam);
        if !clam {
            if let Some(exe) = mpcmdrun() {
                let _ = cmd(exe).args(["-Scan", "-Cancel"]).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
            }
        }
        if let Some(c) = sperre(&self.kind).as_mut() {
            let _ = c.kill();
        }
        if let Some(l) = sperre(&self.lauf).as_mut() {
            l.abgebrochen = true;
        }
    }
}

// ---------------- Windows: Microsoft Defender ----------------

/// Pfad von `MpCmdRun.exe` (Programme-Ordner, sonst neueste Plattform-Version).
pub fn mpcmdrun() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let pf = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Program Files"));
    let p = pf.join("Windows Defender").join("MpCmdRun.exe");
    if p.exists() {
        return Some(p);
    }
    let pd = std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\ProgramData"));
    let plat = pd.join("Microsoft").join("Windows Defender").join("Platform");
    let mut best: Option<(Vec<u32>, PathBuf)> = None;
    for e in std::fs::read_dir(plat).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let ver: Vec<u32> = name.split(['.', '-']).filter_map(|x| x.parse().ok()).collect();
        let exe = e.path().join("MpCmdRun.exe");
        if exe.exists() && best.as_ref().is_none_or(|(b, _)| ver > *b) {
            best = Some((ver, exe));
        }
    }
    best.map(|(_, p)| p)
}

/// Art des Defender-Scans.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScanArt {
    Schnell,
    Voll,
    Pfad,
}

/// Startet einen Defender-Scan. Liefert false, wenn MpCmdRun fehlt.
pub fn defender_scan(l: &Laeufer, art: ScanArt, pfad: &str) -> bool {
    let Some(exe) = mpcmdrun() else { return false };
    let (titel, args): (&'static str, Vec<String>) = match art {
        ScanArt::Schnell => ("Schnellscan", vec!["-Scan".into(), "-ScanType".into(), "1".into()]),
        ScanArt::Voll => ("Vollständiger Scan", vec!["-Scan".into(), "-ScanType".into(), "2".into()]),
        ScanArt::Pfad => ("Ordner/Datei scannen", vec!["-Scan".into(), "-ScanType".into(), "3".into(), "-File".into(), pfad.trim().trim_matches('"').to_string()]),
    };
    l.starte(titel, exe, args, false);
    true
}

/// Lädt neue Defender-Signaturen.
pub fn defender_update(l: &Laeufer) -> bool {
    let Some(exe) = mpcmdrun() else { return false };
    l.starte("Signaturen aktualisieren", exe, vec!["-SignatureUpdate".into()], false);
    true
}

/// Defender-Status (nur Lesen).
#[derive(Clone, Default)]
pub struct DefStatus {
    pub verfuegbar: bool,
    pub echtzeit: Option<bool>,
    pub dienst: Option<bool>,
    pub antivirus: Option<bool>,
    pub sig_alter_tage: Option<u32>,
    pub sig_stand: String,
    pub sig_version: String,
    pub schnell_alter_tage: Option<u32>,
    pub voll_alter_tage: Option<u32>,
    pub modus: String,
    pub manipulationsschutz: Option<bool>,
    /// Im Windows-Sicherheitscenter gemeldete Virenschutz-Programme.
    pub produkte: String,
}

fn tage(v: Option<&str>) -> Option<u32> {
    v?.parse::<u32>().ok().filter(|n| *n < 10_000)
}

pub fn defender_status() -> DefStatus {
    if !cfg!(windows) {
        return DefStatus::default();
    }
    let script = "$ErrorActionPreference='SilentlyContinue';$s=Get-MpComputerStatus;\
if($s){foreach($k in 'AMServiceEnabled','AntivirusEnabled','RealTimeProtectionEnabled','AntivirusSignatureAge','AntivirusSignatureVersion','QuickScanAge','FullScanAge','AMRunningMode','IsTamperProtected'){\"$k`t$($s.$k)\"};\
if($s.AntivirusSignatureLastUpdated){\"SigStand`t$($s.AntivirusSignatureLastUpdated.ToString('dd.MM.yyyy HH:mm'))\"}};\
$av=(Get-CimInstance -Namespace root/SecurityCenter2 -ClassName AntiVirusProduct | ForEach-Object {$_.displayName}) -join ', ';\"Produkte`t$av\"";
    let out = ps(script).unwrap_or_default();
    let k = scanparse::parse_kv(&out);
    let g = |n: &str| scanparse::kv(&k, n);
    DefStatus {
        verfuegbar: g("AMServiceEnabled").is_some(),
        echtzeit: scanparse::ja(g("RealTimeProtectionEnabled")),
        dienst: scanparse::ja(g("AMServiceEnabled")),
        antivirus: scanparse::ja(g("AntivirusEnabled")),
        sig_alter_tage: tage(g("AntivirusSignatureAge")),
        sig_stand: g("SigStand").unwrap_or("").to_string(),
        sig_version: g("AntivirusSignatureVersion").unwrap_or("").to_string(),
        schnell_alter_tage: tage(g("QuickScanAge")),
        voll_alter_tage: tage(g("FullScanAge")),
        modus: g("AMRunningMode").unwrap_or("").to_string(),
        manipulationsschutz: scanparse::ja(g("IsTamperProtected")),
        produkte: g("Produkte").unwrap_or("").to_string(),
    }
}

/// Von Defender erkannte Bedrohungen (Verlauf), nur Lesen.
pub fn defender_bedrohungen() -> Vec<Bedrohung> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let script = "$ErrorActionPreference='SilentlyContinue';$n=@{};$akt=@{};$sev=@{};\
foreach($t in Get-MpThreat){$i=[string]$t.ThreatID;$n[$i]=$t.ThreatName;$akt[$i]=$t.IsActive;$sev[$i]=$t.SeverityID};\
foreach($d in Get-MpThreatDetection){$i=[string]$d.ThreatID;$name=$n[$i];if(-not $name){$name=\"ID $i\"};\
$res=((@($d.Resources)) -join ' | ') -replace \"[`t`r`n]\",' ';$z='';if($d.InitialDetectionTime){$z=$d.InitialDetectionTime.ToString('yyyy-MM-dd HH:mm')};\
\"$z`t$name`t$($sev[$i])`t$($akt[$i])`t$($d.ThreatStatusID)`t$res\"}";
    scanparse::parse_bedrohungen(&ps(script).unwrap_or_default())
}

// ---------------- macOS/Linux: ClamAV ----------------

#[derive(Clone)]
pub struct Clam {
    pub clamscan: PathBuf,
    pub freshclam: Option<PathBuf>,
    pub version: String,
}

fn version_von(p: &PathBuf) -> Option<String> {
    let o = cmd(p).arg("--version").output().ok()?;
    if !o.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// Sucht ClamAV (auch dort, wo Programme aus dem Finder ihren PATH nicht kennen).
pub fn clam_finden() -> Option<Clam> {
    if cfg!(windows) {
        return None;
    }
    let orte = ["", "/opt/homebrew/bin/", "/usr/local/bin/", "/usr/bin/", "/opt/local/bin/"];
    for o in orte {
        let p = PathBuf::from(format!("{o}clamscan"));
        if let Some(v) = version_von(&p) {
            let fresh = PathBuf::from(format!("{o}freshclam"));
            let freshclam = if o.is_empty() || fresh.exists() { Some(fresh) } else { None };
            return Some(Clam { clamscan: p, freshclam, version: v });
        }
    }
    None
}

pub fn clam_scan(l: &Laeufer, c: &Clam, pfad: &str) {
    l.starte("Ordner scannen", c.clamscan.clone(), vec!["-r".into(), "-i".into(), pfad.trim().to_string()], true);
}

pub fn clam_update(l: &Laeufer, c: &Clam) -> bool {
    let Some(f) = c.freshclam.clone() else { return false };
    l.starte("Signaturen aktualisieren", f, Vec::new(), true);
    true
}

/// Standardordner für den ClamAV-Scan: der Benutzerordner.
pub fn standard_pfad() -> String {
    if cfg!(windows) {
        std::env::var("USERPROFILE").unwrap_or_default()
    } else {
        std::env::var("HOME").unwrap_or_default()
    }
}
