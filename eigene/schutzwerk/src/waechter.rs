//! Ressourcen-Wächter (defensiv, nur Lesen): misst CPU, Speicher und – unter Windows,
//! wenn verfügbar – GPU-Last je Prozess und stuft auffällige Programme ein (versteckte
//! Miner, Programme aus Temp/AppData/Downloads, nachgeahmte Systemnamen). Optional eine
//! Hintergrund-Wache, die bei dauerhaft hoher Last warnt, solange das Fenster offen ist.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::prozparse::{self, Ort};
#[cfg(any(windows, target_os = "macos"))]
use crate::sys;
use crate::sys::sperre;

#[derive(Clone)]
pub struct Proz {
    pub pid: u32,
    pub name: String,
    pub exe: String,
    /// CPU-Anteil am ganzen Rechner (0–100).
    pub cpu: f32,
    pub mem: u64,
    pub gpu: Option<f32>,
    pub ort: Ort,
    pub imitation: Option<&'static str>,
    pub miner: bool,
    pub fern: Option<&'static str>,
}

impl Proz {
    /// Hohe Last (CPU oder GPU) – Schwelle in Prozent des ganzen Rechners.
    pub fn hohe_last(&self, schwelle: f32) -> bool {
        self.cpu >= schwelle || self.gpu.is_some_and(|g| g >= schwelle.max(50.0))
    }
}

#[derive(Clone, Default)]
pub struct Messung {
    pub procs: Vec<Proz>,
    /// Sekunden ohne Eingabe (Maus/Tastatur), wenn ermittelbar.
    pub leerlauf_s: Option<u64>,
    pub kerne: usize,
    pub sekunden: u64,
    pub gpu_verfuegbar: bool,
}

pub fn kerne() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).max(1)
}

fn auffrischen(s: &mut System) {
    s.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cpu().with_memory().with_exe(UpdateKind::OnlyIfNotSet),
    );
}

fn windows_pfad() -> bool {
    cfg!(windows)
}

/// Prozessliste aus einem aufgefrischten `System`.
fn liste(s: &System, kerne: usize, gpu: &HashMap<u32, f32>) -> Vec<Proz> {
    let mut v = Vec::new();
    for (pid, p) in s.processes() {
        if p.thread_kind().is_some() {
            continue; // Linux: Threads nicht als eigene Prozesse zeigen
        }
        let pid = pid.as_u32();
        if pid == 0 {
            continue;
        }
        let name = p.name().to_string_lossy().to_string();
        let exe = p.exe().map(|e| e.display().to_string()).unwrap_or_default();
        let roh = p.cpu_usage();
        let cpu = if roh.is_finite() { (roh / kerne as f32).clamp(0.0, 100.0) } else { 0.0 };
        let ort = prozparse::ort_von(&exe, windows_pfad());
        let imitation = if windows_pfad() { prozparse::imitation(&name, &exe) } else { None };
        v.push(Proz {
            pid,
            miner: prozparse::ist_miner(&name),
            fern: prozparse::fernzugriff(&name),
            gpu: gpu.get(&pid).copied(),
            name,
            exe,
            cpu,
            mem: p.memory(),
            ort,
            imitation,
        });
    }
    v.sort_by(|a, b| b.cpu.partial_cmp(&a.cpu).unwrap_or(std::cmp::Ordering::Equal).then(b.mem.cmp(&a.mem)));
    v
}

/// GPU-Last je Prozess (nur Windows, über die Leistungsindikatoren; sonst leer).
fn gpu_lesen() -> Option<HashMap<u32, f32>> {
    #[cfg(windows)]
    {
        // Zweimal abfragen: die erste Abfrage der «formatierten» Werte liefert oft nur Nullen.
        let script = "$ErrorActionPreference='SilentlyContinue';$c='Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine';\
$null=Get-CimInstance $c;Start-Sleep -Milliseconds 1200;\
$r=Get-CimInstance $c;if($r){'ok'};foreach($e in $r){\"$($e.Name)`t$($e.UtilizationPercentage)\"}";
        let out = sys::ps(script)?;
        if !out.lines().any(|l| l.trim() == "ok") {
            return None;
        }
        Some(prozparse::parse_gpu(&out))
    }
    #[cfg(not(windows))]
    None
}

/// Misst alle Prozesse über `sekunden` Sekunden (im Hintergrund-Thread aufrufen).
pub fn messen(sekunden: u64) -> Messung {
    let gpu_thread = std::thread::spawn(gpu_lesen);
    let mut s = System::new();
    auffrischen(&mut s);
    std::thread::sleep(Duration::from_secs(sekunden.max(1)).max(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL));
    auffrischen(&mut s);
    let gpu = gpu_thread.join().ok().flatten();
    let k = kerne();
    Messung {
        procs: liste(&s, k, gpu.as_ref().unwrap_or(&HashMap::new())),
        leerlauf_s: leerlauf_s(),
        kerne: k,
        sekunden,
        gpu_verfuegbar: gpu.is_some(),
    }
}

/// Sekunden seit der letzten Eingabe (Windows: GetLastInputInfo, macOS: ioreg).
pub fn leerlauf_s() -> Option<u64> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::GetTickCount;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
        let mut li = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        // SAFETY: `li` ist eine gültige, lokal angelegte Struktur mit gesetzter Grösse.
        let ok = unsafe { GetLastInputInfo(&mut li) };
        if ok == 0 {
            return None;
        }
        // SAFETY: einfache Abfrage ohne Zeiger.
        let jetzt = unsafe { GetTickCount() };
        Some((jetzt.wrapping_sub(li.dwTime) / 1000) as u64)
    }
    #[cfg(target_os = "macos")]
    {
        let out = sys::run("ioreg", &["-c", "IOHIDSystem", "-d", "4"])?;
        prozparse::parse_ioreg_idle(&out)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    None
}

/// Name und Programmdatei zu PIDs (für Verbindungen, Fernzugriff).
pub fn prozesse_kurz() -> HashMap<u32, (String, String)> {
    let mut s = System::new();
    s.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet));
    s.processes()
        .iter()
        .filter(|(_, p)| p.thread_kind().is_none())
        .map(|(pid, p)| (pid.as_u32(), (p.name().to_string_lossy().to_string(), p.exe().map(|e| e.display().to_string()).unwrap_or_default())))
        .collect()
}

// ---------------- Hintergrund-Wache ----------------

#[derive(Clone)]
pub struct Alarm {
    pub pid: u32,
    pub name: String,
    pub exe: String,
    pub sekunden: u64,
    pub last: f32,
}

/// Warnt, wenn ein Prozess länger als `dauer` über `schwelle` % CPU liegt.
/// Läuft nur, solange Schutzwerk offen ist.
pub struct Wache {
    an: Arc<AtomicBool>,
    /// Zähler je Start: ein alter Thread beendet sich, sobald ein neuer läuft.
    gen: Arc<AtomicUsize>,
    pub alarme: Arc<Mutex<Vec<Alarm>>>,
    /// (Schwelle in %, Dauer in Sekunden)
    pub grenzen: Arc<Mutex<(f32, u64)>>,
}

impl Wache {
    pub fn new() -> Self {
        Wache { an: Arc::new(AtomicBool::new(false)), gen: Arc::new(AtomicUsize::new(0)), alarme: Arc::new(Mutex::new(Vec::new())), grenzen: Arc::new(Mutex::new((80.0, 120))) }
    }

    pub fn laeuft(&self) -> bool {
        self.an.load(Ordering::SeqCst)
    }

    pub fn starten(&self, ctx: egui::Context) {
        if self.an.swap(true, Ordering::SeqCst) {
            return;
        }
        let g = self.gen.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
        let gen = self.gen.clone();
        let an0 = self.an.clone();
        let an = move || an0.load(Ordering::SeqCst) && gen.load(Ordering::SeqCst) == g;
        let alarme = self.alarme.clone();
        let grenzen = self.grenzen.clone();
        std::thread::spawn(move || {
            let mut s = System::new();
            auffrischen(&mut s);
            let mut spur = prozparse::Dauerlast::default();
            let start = std::time::Instant::now();
            let k = kerne();
            while an() {
                // In kleinen Schritten schlafen, damit «Aus» rasch wirkt.
                for _ in 0..20 {
                    if !an() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
                if !an() {
                    break;
                }
                auffrischen(&mut s);
                let procs = liste(&s, k, &HashMap::new());
                let proben: Vec<(u32, String, f32)> = procs.iter().map(|p| (p.pid, p.name.clone(), p.cpu)).collect();
                let (schwelle, dauer) = *sperre(&grenzen);
                let treffer = spur.messen(start.elapsed().as_secs(), &proben, schwelle, dauer);
                let neu: Vec<Alarm> = treffer
                    .into_iter()
                    .map(|(pid, name, sekunden, last)| Alarm {
                        pid,
                        exe: procs.iter().find(|p| p.pid == pid).map(|p| p.exe.clone()).unwrap_or_default(),
                        name,
                        sekunden,
                        last,
                    })
                    .collect();
                *sperre(&alarme) = neu;
                ctx.request_repaint();
            }
            if !an() {
                sperre(&alarme).clear();
            }
            ctx.request_repaint();
        });
    }

    pub fn stoppen(&self) {
        self.an.store(false, Ordering::SeqCst);
    }
}
