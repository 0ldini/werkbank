//! libVLC 3.0 – zur Laufzeit geladen (libloading), ohne Abhängigkeit beim Linken.
//!
//! Fehlt libVLC, baut und startet das Programm trotzdem; die Oberfläche meldet es und
//! zeigt weiter Bilder. Alles `unsafe` des Programms steht in dieser Datei.
//!
//! Bild: libVLC zeichnet über die «vmem»-Rückrufe (lock/unlock/display) in einen eigenen
//! Puffer (RV32 = BGRA im Speicher); `display` wandelt ihn in ein egui-Bild um, legt es in
//! `Vmem::bild` ab und fordert ein neues Zeichnen an. Die Grösse legt libVLC über den
//! Format-Rückruf fest, damit jedes Video passt.

use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};
use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};

use libloading::Library;

#[repr(C)]
pub struct TrackDesc {
    i_id: c_int,
    psz_name: *mut c_char,
    p_next: *mut TrackDesc,
}

type SetupCb = unsafe extern "C" fn(*mut *mut c_void, *mut c_char, *mut c_uint, *mut c_uint, *mut c_uint, *mut c_uint) -> c_uint;
type CleanupCb = unsafe extern "C" fn(*mut c_void);
type LockCb = unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> *mut c_void;
type UnlockCb = unsafe extern "C" fn(*mut c_void, *mut c_void, *const *mut c_void);
type DisplayCb = unsafe extern "C" fn(*mut c_void, *mut c_void);

macro_rules! vlc_fns {
    ($($name:ident : fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        struct Fns {
            $($name: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
        }
        impl Fns {
            /// SAFETY: Die Signaturen entsprechen libvlc 3.0 (vlc/libvlc*.h). Die Zeiger
            /// gelten, solange `lib` geladen ist; die Bibliothek wird nie entladen.
            unsafe fn laden(lib: &Library) -> Result<Fns, String> {
                Ok(Fns {
                    $($name: *lib
                        .get::<unsafe extern "C" fn($($arg),*) $(-> $ret)?>(concat!(stringify!($name), "\0").as_bytes())
                        .map_err(|e| format!("{}: {e}", stringify!($name)))?,)*
                })
            }
        }
    };
}

vlc_fns! {
    libvlc_new: fn(c_int, *const *const c_char) -> *mut c_void;
    libvlc_release: fn(*mut c_void);
    libvlc_get_version: fn() -> *const c_char;
    libvlc_free: fn(*mut c_void);
    libvlc_media_new_path: fn(*mut c_void, *const c_char) -> *mut c_void;
    libvlc_media_release: fn(*mut c_void);
    libvlc_media_parse_with_options: fn(*mut c_void, c_int, c_int) -> c_int;
    libvlc_media_get_meta: fn(*mut c_void, c_int) -> *mut c_char;
    libvlc_media_player_new: fn(*mut c_void) -> *mut c_void;
    libvlc_media_player_release: fn(*mut c_void);
    libvlc_media_player_set_media: fn(*mut c_void, *mut c_void);
    libvlc_media_player_play: fn(*mut c_void) -> c_int;
    libvlc_media_player_set_pause: fn(*mut c_void, c_int);
    libvlc_media_player_stop: fn(*mut c_void);
    libvlc_media_player_get_time: fn(*mut c_void) -> i64;
    libvlc_media_player_set_time: fn(*mut c_void, i64);
    libvlc_media_player_get_length: fn(*mut c_void) -> i64;
    libvlc_media_player_get_state: fn(*mut c_void) -> c_int;
    libvlc_media_player_set_rate: fn(*mut c_void, f32) -> c_int;
    libvlc_audio_set_volume: fn(*mut c_void, c_int) -> c_int;
    libvlc_audio_get_volume: fn(*mut c_void) -> c_int;
    libvlc_audio_set_mute: fn(*mut c_void, c_int);
    libvlc_audio_get_mute: fn(*mut c_void) -> c_int;
    libvlc_video_set_callbacks: fn(*mut c_void, Option<LockCb>, Option<UnlockCb>, Option<DisplayCb>, *mut c_void);
    libvlc_video_set_format_callbacks: fn(*mut c_void, Option<SetupCb>, Option<CleanupCb>);
    libvlc_audio_get_track_description: fn(*mut c_void) -> *mut TrackDesc;
    libvlc_audio_get_track: fn(*mut c_void) -> c_int;
    libvlc_audio_set_track: fn(*mut c_void, c_int) -> c_int;
    libvlc_video_get_spu_description: fn(*mut c_void) -> *mut TrackDesc;
    libvlc_video_get_spu: fn(*mut c_void) -> c_int;
    libvlc_video_set_spu: fn(*mut c_void, c_int) -> c_int;
    libvlc_track_description_list_release: fn(*mut TrackDesc);
}

/// Möglicher Ort von libVLC (voller Pfad oder nur der Name für die Systemsuche).
#[derive(Clone, Debug)]
pub struct Kandidat {
    pub datei: PathBuf,
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf))
}

/// Sucht libVLC (zuerst neben dem Programm, dann im System) und setzt `VLC_PLUGIN_PATH`
/// auf den mitgelieferten Plugin-Ordner. Muss vor dem Start weiterer Threads laufen.
pub fn vorbereiten() -> Vec<Kandidat> {
    let mut v: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    let exe = exe_dir();
    if cfg!(windows) {
        if let Some(d) = &exe {
            v.push((d.join("libvlc.dll"), Some(d.join("plugins"))));
        }
        for base in ["ProgramFiles", "ProgramW6432"] {
            if let Some(pf) = std::env::var_os(base) {
                let d = PathBuf::from(pf).join("VideoLAN").join("VLC");
                v.push((d.join("libvlc.dll"), Some(d.join("plugins"))));
            }
        }
        v.push((PathBuf::from("libvlc.dll"), None));
    } else if cfg!(target_os = "macos") {
        if let Some(d) = &exe {
            v.push((d.join("lib").join("libvlc.dylib"), Some(d.join("plugins"))));
            v.push((d.join("libvlc.dylib"), Some(d.join("plugins"))));
        }
        let app = PathBuf::from("/Applications/VLC.app/Contents/MacOS");
        v.push((app.join("lib").join("libvlc.dylib"), Some(app.join("plugins"))));
        v.push((PathBuf::from("libvlc.dylib"), None));
    } else {
        if let Some(d) = &exe {
            v.push((d.join("libvlc.so.5"), Some(d.join("plugins"))));
        }
        v.push((PathBuf::from("libvlc.so.5"), None));
        v.push((PathBuf::from("libvlc.so"), None));
    }
    // Nur volle Pfade, die es gibt, und blosse Namen behalten.
    v.retain(|(p, _)| !p.is_absolute() || p.is_file());
    // Plugins: für den ersten vorhandenen vollen Pfad (falls nicht schon gesetzt).
    if std::env::var_os("VLC_PLUGIN_PATH").is_none() {
        if let Some(plug) = v.iter().find(|(p, _)| p.is_absolute()).and_then(|(_, pl)| pl.clone()).filter(|pl| pl.is_dir()) {
            std::env::set_var("VLC_PLUGIN_PATH", plug);
        }
    }
    v.into_iter().map(|(datei, _)| Kandidat { datei }).collect()
}

fn bibliothek(k: &Kandidat) -> Result<Library, libloading::Error> {
    #[cfg(windows)]
    {
        if k.datei.is_absolute() {
            // libvlccore.dll liegt neben libvlc.dll: beim Laden dort suchen.
            // SAFETY: libVLC hat keine Initialisierung beim Laden, die Annahmen verletzt.
            return unsafe { libloading::os::windows::Library::load_with_flags(&k.datei, libloading::os::windows::LOAD_WITH_ALTERED_SEARCH_PATH) }.map(Library::from);
        }
    }
    // SAFETY: wie oben; libVLC führt beim Laden keinen fremden Code aus.
    unsafe { Library::new(&k.datei) }
}

/// Geladenes libVLC mit einer Instanz.
pub struct Core {
    f: Fns,
    inst: *mut c_void,
    pub version: String,
    pub datei: PathBuf,
    // Nie entladen: libVLC hinterlässt Threads und Rückrufe, die ein Entladen nicht übersteht.
    _lib: ManuallyDrop<Library>,
}

// SAFETY: Eine libvlc-Instanz darf von jedem Thread benutzt werden (libVLC ist threadsicher);
// `Core` wird nur einmal vom Ladethread an die Oberfläche übergeben.
unsafe impl Send for Core {}

impl Drop for Core {
    fn drop(&mut self) {
        // SAFETY: `inst` stammt aus libvlc_new, ist nicht null und wird nur hier freigegeben.
        unsafe { (self.f.libvlc_release)(self.inst) }
    }
}

/// libVLC laden und eine Instanz anlegen (dauert, weil libVLC die Plugins einliest –
/// darum im Hilfsthread aufrufen).
pub fn laden(kandidaten: &[Kandidat]) -> Result<Core, String> {
    if kandidaten.is_empty() {
        return Err(String::new());
    }
    let mut fehler = Vec::new();
    for k in kandidaten {
        let lib = match bibliothek(k) {
            Ok(l) => l,
            Err(e) => {
                fehler.push(format!("{}: {e}", k.datei.display()));
                continue;
            }
        };
        // SAFETY: siehe `Fns::laden`.
        let f = match unsafe { Fns::laden(&lib) } {
            Ok(f) => f,
            Err(e) => {
                fehler.push(format!("{}: {e}", k.datei.display()));
                continue;
            }
        };
        let mut sets: Vec<Vec<&str>> = Vec::new();
        let mut voll = vec!["--no-video-title-show", "--no-snapshot-preview", "--no-stats", "--no-osd", "--quiet"];
        if cfg!(target_os = "linux") {
            voll.push("--no-xlib");
        }
        sets.push(voll);
        sets.push(vec!["--no-video-title-show", "--quiet"]);
        sets.push(Vec::new());
        for args in sets {
            let cs: Vec<CString> = args.iter().filter_map(|a| CString::new(*a).ok()).collect();
            let ptrs: Vec<*const c_char> = cs.iter().map(|c| c.as_ptr()).collect();
            // SAFETY: `ptrs` zeigt auf gültige, nullterminierte Zeichenketten in `cs`,
            // die bis nach dem Aufruf leben; argc passt zur Länge.
            let inst = unsafe { (f.libvlc_new)(ptrs.len() as c_int, if ptrs.is_empty() { std::ptr::null() } else { ptrs.as_ptr() }) };
            if !inst.is_null() {
                // SAFETY: libvlc_get_version liefert eine statische Zeichenkette (oder null).
                let version = unsafe {
                    let p = (f.libvlc_get_version)();
                    if p.is_null() {
                        String::new()
                    } else {
                        CStr::from_ptr(p).to_string_lossy().into_owned()
                    }
                };
                return Ok(Core { f, inst, version, datei: k.datei.clone(), _lib: ManuallyDrop::new(lib) });
            }
        }
        fehler.push(format!("{}: libvlc_new", k.datei.display()));
    }
    Err(fehler.join("\n"))
}

// ------------------------------------------------------------------------------ Bild (vmem)

fn sperren<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct Puffer {
    w: usize,
    h: usize,
    pitch: usize,
    daten: Vec<u8>,
}

/// Letztes fertiges Videobild.
pub struct Bild {
    pub size: [usize; 2],
    pub pixel: Vec<egui::Color32>,
    pub neu: bool,
}

pub struct Vmem {
    ctx: egui::Context,
    puffer: Mutex<Puffer>,
    pub bild: Mutex<Bild>,
}

/// Grösste Fläche, die das Bild bekommt (grössere Videos rechnet libVLC herunter).
const MAX_FLAECHE: usize = 3840 * 2160;

unsafe extern "C" fn cb_setup(opaque: *mut *mut c_void, chroma: *mut c_char, width: *mut c_uint, height: *mut c_uint, pitches: *mut c_uint, lines: *mut c_uint) -> c_uint {
    // SAFETY: libVLC übergibt gültige Zeiger (chroma: 4 Zeichen, pitches/lines: je ein Feld
    // pro Ebene); *opaque ist der Zeiger auf `Vmem`, der lebt, bis der Player freigegeben ist.
    unsafe {
        if opaque.is_null() || (*opaque).is_null() || chroma.is_null() || width.is_null() || height.is_null() || pitches.is_null() || lines.is_null() {
            return 0;
        }
        let v = &*((*opaque) as *const Vmem);
        let (mut w, mut h) = (*width as usize, *height as usize);
        if w == 0 || h == 0 {
            return 0;
        }
        let flaeche = w.saturating_mul(h);
        if flaeche > MAX_FLAECHE {
            let f = (MAX_FLAECHE as f64 / flaeche as f64).sqrt();
            w = (((w as f64 * f) as usize) & !1).max(2);
            h = (((h as f64 * f) as usize) & !1).max(2);
        }
        let pitch = w * 4;
        std::ptr::copy_nonoverlapping(b"RV32".as_ptr() as *const c_char, chroma, 4);
        *width = w as c_uint;
        *height = h as c_uint;
        *pitches = pitch as c_uint;
        *lines = h as c_uint;
        let mut p = sperren(&v.puffer);
        p.w = w;
        p.h = h;
        p.pitch = pitch;
        // Etwas Reserve: manche Dekoder schreiben über die letzte Zeile hinaus.
        p.daten = vec![0u8; pitch * (h + 8) + 64];
        1
    }
}

unsafe extern "C" fn cb_cleanup(opaque: *mut c_void) {
    if opaque.is_null() {
        return;
    }
    // SAFETY: `opaque` zeigt auf `Vmem` (siehe cb_setup).
    let v = unsafe { &*(opaque as *const Vmem) };
    let mut p = sperren(&v.puffer);
    p.w = 0;
    p.h = 0;
    p.daten = Vec::new();
}

unsafe extern "C" fn cb_lock(opaque: *mut c_void, planes: *mut *mut c_void) -> *mut c_void {
    if opaque.is_null() || planes.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: `opaque` zeigt auf `Vmem`; `planes` hat Platz für mindestens einen Zeiger.
    // Der Puffer wird nur in cb_setup/cb_cleanup neu angelegt – im selben Thread von libVLC,
    // nie während ein Bild gesperrt ist.
    unsafe {
        let v = &*(opaque as *const Vmem);
        let mut p = sperren(&v.puffer);
        if p.daten.is_empty() {
            p.daten = vec![0u8; 64];
        }
        *planes = p.daten.as_mut_ptr() as *mut c_void;
    }
    std::ptr::null_mut()
}

unsafe extern "C" fn cb_unlock(_opaque: *mut c_void, _picture: *mut c_void, _planes: *const *mut c_void) {}

unsafe extern "C" fn cb_display(opaque: *mut c_void, _picture: *mut c_void) {
    if opaque.is_null() {
        return;
    }
    // SAFETY: `opaque` zeigt auf `Vmem` (siehe cb_setup).
    let v = unsafe { &*(opaque as *const Vmem) };
    let p = sperren(&v.puffer);
    let (w, h, pitch) = (p.w, p.h, p.pitch);
    if w == 0 || h == 0 || p.daten.len() < pitch * h {
        return;
    }
    let mut px = Vec::with_capacity(w * h);
    for zeile in 0..h {
        let Some(z) = p.daten.get(zeile * pitch..zeile * pitch + w * 4) else { break };
        // RV32: im Speicher B, G, R, X
        px.extend(z.chunks_exact(4).map(|c| egui::Color32::from_rgb(c[2], c[1], c[0])));
    }
    drop(p);
    if px.len() != w * h {
        return;
    }
    let mut b = sperren(&v.bild);
    b.size = [w, h];
    b.pixel = px;
    b.neu = true;
    drop(b);
    v.ctx.request_repaint();
}

// ------------------------------------------------------------------------------ Player

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Zustand {
    Nichts,
    Oeffnet,
    Puffert,
    Spielt,
    Pause,
    Gestoppt,
    Ende,
    Fehler,
}

pub enum Meta {
    Titel = 0,
    Kuenstler = 1,
    Album = 4,
    Bild = 15,
}

pub struct Player {
    core: Rc<Core>,
    mp: *mut c_void,
    media: *mut c_void,
    vmem: Arc<Vmem>,
}

impl Player {
    pub fn neu(core: Rc<Core>, ctx: egui::Context) -> Result<Player, String> {
        // SAFETY: `core.inst` ist eine gültige Instanz.
        let mp = unsafe { (core.f.libvlc_media_player_new)(core.inst) };
        if mp.is_null() {
            return Err("libvlc_media_player_new".to_string());
        }
        let vmem = Arc::new(Vmem {
            ctx,
            puffer: Mutex::new(Puffer { w: 0, h: 0, pitch: 0, daten: Vec::new() }),
            bild: Mutex::new(Bild { size: [0, 0], pixel: Vec::new(), neu: false }),
        });
        let opaque = Arc::as_ptr(&vmem) as *mut c_void;
        // SAFETY: `mp` ist gültig; `opaque` zeigt auf `Vmem`, das `Player` hält, bis der
        // Player in `drop` gestoppt und freigegeben ist (danach ruft libVLC nichts mehr auf).
        unsafe {
            (core.f.libvlc_video_set_callbacks)(mp, Some(cb_lock), Some(cb_unlock), Some(cb_display), opaque);
            (core.f.libvlc_video_set_format_callbacks)(mp, Some(cb_setup), Some(cb_cleanup));
        }
        Ok(Player { core, mp, media: std::ptr::null_mut(), vmem })
    }

    pub fn version(&self) -> &str {
        &self.core.version
    }

    /// Datei laden und abspielen.
    pub fn oeffnen(&mut self, pfad: &Path) -> Result<(), String> {
        let s = pfad.to_str().ok_or_else(|| "Dateiname nicht lesbar".to_string())?;
        let c = CString::new(s).map_err(|_| "Dateiname nicht lesbar".to_string())?;
        // SAFETY: gültige Instanz und nullterminierter Pfad (UTF-8, wie libVLC es erwartet).
        let m = unsafe { (self.core.f.libvlc_media_new_path)(self.core.inst, c.as_ptr()) };
        if m.is_null() {
            return Err("Datei lässt sich nicht öffnen".to_string());
        }
        {
            let mut b = sperren(&self.vmem.bild);
            b.size = [0, 0];
            b.pixel = Vec::new();
            b.neu = false;
        }
        // SAFETY: `mp` und `m` sind gültig; set_media hält einen eigenen Verweis auf `m`.
        // Das alte Medium geben wir danach frei (unser Verweis).
        unsafe {
            (self.core.f.libvlc_media_player_set_media)(self.mp, m);
            if !self.media.is_null() {
                (self.core.f.libvlc_media_release)(self.media);
            }
            self.media = m;
            // Angaben (Titel, Künstler, Albumbild) im Hintergrund lesen; 0x02 = lokal holen.
            (self.core.f.libvlc_media_parse_with_options)(m, 0x02, 5000);
            if (self.core.f.libvlc_media_player_play)(self.mp) != 0 {
                return Err("Wiedergabe startet nicht".to_string());
            }
        }
        Ok(())
    }

    pub fn play(&self) {
        // SAFETY: `mp` ist gültig (gilt für alle folgenden Aufrufe mit `self.mp`).
        unsafe {
            (self.core.f.libvlc_media_player_play)(self.mp);
        }
    }

    pub fn pause(&self, an: bool) {
        // SAFETY: siehe `play`.
        unsafe { (self.core.f.libvlc_media_player_set_pause)(self.mp, an as c_int) }
    }

    pub fn stop(&self) {
        // SAFETY: siehe `play`. Nie aus einem Rückruf aufrufen (dort wäre es eine Verklemmung).
        unsafe { (self.core.f.libvlc_media_player_stop)(self.mp) }
        let mut b = sperren(&self.vmem.bild);
        b.size = [0, 0];
        b.pixel = Vec::new();
        b.neu = false;
    }

    pub fn hat_medium(&self) -> bool {
        !self.media.is_null()
    }

    pub fn zustand(&self) -> Zustand {
        // SAFETY: siehe `play`.
        match unsafe { (self.core.f.libvlc_media_player_get_state)(self.mp) } {
            1 => Zustand::Oeffnet,
            2 => Zustand::Puffert,
            3 => Zustand::Spielt,
            4 => Zustand::Pause,
            5 => Zustand::Gestoppt,
            6 => Zustand::Ende,
            7 => Zustand::Fehler,
            _ => Zustand::Nichts,
        }
    }

    /// Position in Millisekunden (0, wenn unbekannt).
    pub fn zeit(&self) -> i64 {
        // SAFETY: siehe `play`.
        unsafe { (self.core.f.libvlc_media_player_get_time)(self.mp) }.max(0)
    }

    pub fn setze_zeit(&self, ms: i64) {
        // SAFETY: siehe `play`.
        unsafe { (self.core.f.libvlc_media_player_set_time)(self.mp, ms.max(0)) }
    }

    /// Länge in Millisekunden (0, wenn unbekannt).
    pub fn laenge(&self) -> i64 {
        // SAFETY: siehe `play`.
        unsafe { (self.core.f.libvlc_media_player_get_length)(self.mp) }.max(0)
    }

    pub fn tempo(&self, r: f32) {
        if r.is_finite() && r > 0.0 {
            // SAFETY: siehe `play`.
            unsafe {
                (self.core.f.libvlc_media_player_set_rate)(self.mp, r);
            }
        }
    }

    /// Lautstärke 0–100 und stumm anwenden, falls libVLC schon eine Tonausgabe hat.
    pub fn ton(&self, lautstaerke: i32, stumm: bool) {
        // SAFETY: siehe `play`.
        unsafe {
            let v = (self.core.f.libvlc_audio_get_volume)(self.mp);
            if v >= 0 && v != lautstaerke {
                (self.core.f.libvlc_audio_set_volume)(self.mp, lautstaerke.clamp(0, 200));
            }
            let m = (self.core.f.libvlc_audio_get_mute)(self.mp);
            if m >= 0 && (m != 0) != stumm {
                (self.core.f.libvlc_audio_set_mute)(self.mp, stumm as c_int);
            }
        }
    }

    fn liste(&self, p: *mut TrackDesc) -> Vec<(i32, String)> {
        let mut v = Vec::new();
        let mut cur = p;
        // SAFETY: `p` ist eine von libVLC gelieferte, mit null abgeschlossene Liste; sie wird
        // hier einmal gelesen und danach mit libvlc_track_description_list_release freigegeben.
        unsafe {
            while !cur.is_null() && v.len() < 200 {
                let d = &*cur;
                let name = if d.psz_name.is_null() { String::new() } else { CStr::from_ptr(d.psz_name).to_string_lossy().into_owned() };
                v.push((d.i_id, name));
                cur = d.p_next;
            }
            if !p.is_null() {
                (self.core.f.libvlc_track_description_list_release)(p);
            }
        }
        v
    }

    /// Tonspuren (Nummer, Name); −1 = «Deaktivieren».
    pub fn tonspuren(&self) -> Vec<(i32, String)> {
        // SAFETY: siehe `play`.
        let p = unsafe { (self.core.f.libvlc_audio_get_track_description)(self.mp) };
        self.liste(p)
    }

    pub fn tonspur(&self) -> i32 {
        // SAFETY: siehe `play`.
        unsafe { (self.core.f.libvlc_audio_get_track)(self.mp) }
    }

    pub fn setze_tonspur(&self, id: i32) {
        // SAFETY: siehe `play`.
        unsafe {
            (self.core.f.libvlc_audio_set_track)(self.mp, id);
        }
    }

    pub fn untertitel_liste(&self) -> Vec<(i32, String)> {
        // SAFETY: siehe `play`.
        let p = unsafe { (self.core.f.libvlc_video_get_spu_description)(self.mp) };
        self.liste(p)
    }

    pub fn untertitel(&self) -> i32 {
        // SAFETY: siehe `play`.
        unsafe { (self.core.f.libvlc_video_get_spu)(self.mp) }
    }

    pub fn setze_untertitel(&self, id: i32) {
        // SAFETY: siehe `play`.
        unsafe {
            (self.core.f.libvlc_video_set_spu)(self.mp, id);
        }
    }

    /// Angabe des aktuellen Mediums (Titel, Künstler …), falls bekannt.
    pub fn meta(&self, m: Meta) -> Option<String> {
        if self.media.is_null() {
            return None;
        }
        // SAFETY: `media` ist gültig; das Ergebnis gehört uns und wird mit libvlc_free freigegeben.
        unsafe {
            let p = (self.core.f.libvlc_media_get_meta)(self.media, m as c_int);
            if p.is_null() {
                return None;
            }
            let s = CStr::from_ptr(p).to_string_lossy().trim().to_string();
            (self.core.f.libvlc_free)(p as *mut c_void);
            (!s.is_empty()).then_some(s)
        }
    }

    /// Neues Videobild abholen (falls seit dem letzten Mal eines kam).
    pub fn neues_bild(&self) -> Option<egui::ColorImage> {
        let mut b = sperren(&self.vmem.bild);
        if !b.neu {
            return None;
        }
        b.neu = false;
        let size = b.size;
        let pixel = std::mem::take(&mut b.pixel);
        (pixel.len() == size[0] * size[1] && !pixel.is_empty()).then_some(egui::ColorImage { size, pixels: pixel })
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // SAFETY: `mp` ist gültig; stop wartet, bis libVLC keine Rückrufe mehr aufruft.
        // Danach dürfen Player und Medium freigegeben werden; `vmem` lebt bis zum Ende dieser
        // Funktion (Feld von `self`).
        unsafe {
            (self.core.f.libvlc_media_player_stop)(self.mp);
            (self.core.f.libvlc_media_player_release)(self.mp);
            if !self.media.is_null() {
                (self.core.f.libvlc_media_release)(self.media);
            }
        }
    }
}
