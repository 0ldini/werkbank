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

/// libvlc_audio_output_device_t
#[repr(C)]
pub struct AudioDevice {
    p_next: *mut AudioDevice,
    psz_device: *mut c_char,
    psz_description: *mut c_char,
}

/// libvlc_media_stats_t (libVLC 3.0)
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Statistik {
    pub gelesen_bytes: c_int,
    pub eingang_bitrate: f32,
    pub demux_bytes: c_int,
    pub demux_bitrate: f32,
    pub demux_kaputt: c_int,
    pub demux_unterbrochen: c_int,
    pub dekodiert_video: c_int,
    pub dekodiert_audio: c_int,
    pub angezeigt: c_int,
    pub verloren: c_int,
    pub audio_gespielt: c_int,
    pub audio_verloren: c_int,
    pub gesendet_pakete: c_int,
    pub gesendet_bytes: c_int,
    pub sende_bitrate: f32,
}

/// Anfang von libvlc_audio_track_t
#[repr(C)]
struct AudioTrack {
    i_channels: c_uint,
    i_rate: c_uint,
}

/// Anfang von libvlc_video_track_t (nur die ersten Felder werden gelesen)
#[repr(C)]
struct VideoTrack {
    i_height: c_uint,
    i_width: c_uint,
    i_sar_num: c_uint,
    i_sar_den: c_uint,
    i_frame_rate_num: c_uint,
    i_frame_rate_den: c_uint,
}

/// libvlc_media_track_t (libVLC 3.0)
#[repr(C)]
#[allow(dead_code)]
pub struct MediaTrack {
    i_codec: u32,
    i_original_fourcc: u32,
    i_id: c_int,
    i_type: c_int,
    i_profile: c_int,
    i_level: c_int,
    u: *mut c_void,
    i_bitrate: c_uint,
    psz_language: *mut c_char,
    psz_description: *mut c_char,
}

/// Angaben zu einer Spur für «Medieninformationen».
#[derive(Clone, Debug, Default)]
pub struct SpurInfo {
    /// 0 Audio, 1 Video, 2 Untertitel, sonst unbekannt
    pub art: i32,
    pub codec: String,
    pub fourcc: String,
    pub sprache: String,
    pub beschreibung: String,
    pub bitrate: u32,
    pub breite: u32,
    pub hoehe: u32,
    pub sar: (u32, u32),
    pub fps: f64,
    pub kanaele: u32,
    pub rate: u32,
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

/// Funktionen, die nicht jedes libVLC 3.x hat oder die nur Zusätze betreffen: fehlt eine,
/// wird die Funktion in der Oberfläche ausgegraut, statt dass das Laden scheitert.
macro_rules! vlc_opt_fns {
    ($($name:ident : fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        struct Opt {
            $($name: Option<unsafe extern "C" fn($($arg),*) $(-> $ret)?>,)*
        }
        impl Opt {
            /// SAFETY: wie `Fns::laden`; fehlende Namen ergeben `None`.
            unsafe fn laden(lib: &Library) -> Opt {
                Opt {
                    $($name: lib
                        .get::<unsafe extern "C" fn($($arg),*) $(-> $ret)?>(concat!(stringify!($name), "\0").as_bytes())
                        .ok()
                        .map(|s| *s),)*
                }
            }
        }
    };
}

vlc_opt_fns! {
    libvlc_media_new_location: fn(*mut c_void, *const c_char) -> *mut c_void;
    libvlc_media_get_duration: fn(*mut c_void) -> i64;
    libvlc_media_get_parsed_status: fn(*mut c_void) -> c_int;
    libvlc_media_parse_stop: fn(*mut c_void);
    libvlc_media_get_stats: fn(*mut c_void, *mut Statistik) -> c_int;
    libvlc_media_tracks_get: fn(*mut c_void, *mut *mut *mut MediaTrack) -> c_uint;
    libvlc_media_tracks_release: fn(*mut *mut MediaTrack, c_uint);
    libvlc_media_get_codec_description: fn(c_int, u32) -> *const c_char;
    libvlc_media_player_next_frame: fn(*mut c_void);
    libvlc_media_player_get_chapter_count: fn(*mut c_void) -> c_int;
    libvlc_media_player_get_chapter: fn(*mut c_void) -> c_int;
    libvlc_media_player_set_chapter: fn(*mut c_void, c_int);
    libvlc_media_player_get_title_count: fn(*mut c_void) -> c_int;
    libvlc_media_player_get_title: fn(*mut c_void) -> c_int;
    libvlc_media_player_set_title: fn(*mut c_void, c_int);
    libvlc_media_player_add_slave: fn(*mut c_void, c_int, *const c_char, bool) -> c_int;
    libvlc_video_set_spu_delay: fn(*mut c_void, i64) -> c_int;
    libvlc_video_set_deinterlace: fn(*mut c_void, *const c_char);
    libvlc_video_set_adjust_int: fn(*mut c_void, c_uint, c_int);
    libvlc_video_set_adjust_float: fn(*mut c_void, c_uint, f32);
    libvlc_audio_equalizer_get_preset_count: fn() -> c_uint;
    libvlc_audio_equalizer_get_preset_name: fn(c_uint) -> *const c_char;
    libvlc_audio_equalizer_get_band_count: fn() -> c_uint;
    libvlc_audio_equalizer_get_band_frequency: fn(c_uint) -> f32;
    libvlc_audio_equalizer_new: fn() -> *mut c_void;
    libvlc_audio_equalizer_new_from_preset: fn(c_uint) -> *mut c_void;
    libvlc_audio_equalizer_release: fn(*mut c_void);
    libvlc_audio_equalizer_set_preamp: fn(*mut c_void, f32) -> c_int;
    libvlc_audio_equalizer_get_preamp: fn(*mut c_void) -> f32;
    libvlc_audio_equalizer_set_amp_at_index: fn(*mut c_void, f32, c_uint) -> c_int;
    libvlc_audio_equalizer_get_amp_at_index: fn(*mut c_void, c_uint) -> f32;
    libvlc_media_player_set_equalizer: fn(*mut c_void, *mut c_void) -> c_int;
    libvlc_audio_output_device_enum: fn(*mut c_void) -> *mut AudioDevice;
    libvlc_audio_output_device_list_release: fn(*mut AudioDevice);
    libvlc_audio_output_device_set: fn(*mut c_void, *const c_char, *const c_char);
    libvlc_audio_output_device_get: fn(*mut c_void) -> *mut c_char;
    libvlc_audio_get_channel: fn(*mut c_void) -> c_int;
    libvlc_audio_set_channel: fn(*mut c_void, c_int) -> c_int;
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
    o: Opt,
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
        let mut voll = vec!["--no-video-title-show", "--no-snapshot-preview", "--no-osd", "--quiet"];
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
                // SAFETY: siehe `Opt::laden`.
                let o = unsafe { Opt::laden(&lib) };
                return Ok(Core { f, o, inst, version, datei: k.datei.clone(), _lib: ManuallyDrop::new(lib) });
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

/// Angaben eines Mediums (libvlc_meta_t).
pub mod meta {
    pub const TITEL: i32 = 0;
    pub const KUENSTLER: i32 = 1;
    pub const GENRE: i32 = 2;
    pub const COPYRIGHT: i32 = 3;
    pub const ALBUM: i32 = 4;
    pub const NUMMER: i32 = 5;
    pub const BESCHREIBUNG: i32 = 6;
    pub const DATUM: i32 = 8;
    pub const URL: i32 = 10;
    pub const SPRACHE: i32 = 11;
    pub const LAEUFT: i32 = 12;
    pub const HERAUSGEBER: i32 = 13;
    pub const KODIERT_VON: i32 = 14;
    pub const BILD: i32 = 15;
    pub const REGIE: i32 = 18;
    pub const ALBUM_KUENSTLER: i32 = 23;
}

/// Ein Videobild als RGB (für Bildschirmfotos).
pub struct Foto {
    pub breite: usize,
    pub hoehe: usize,
    pub rgb: Vec<u8>,
}

fn c_text(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: libVLC liefert nullterminierte Zeichenketten (oder null, oben geprüft).
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

impl Core {
    /// Angabe eines Mediums; das Ergebnis von libvlc_media_get_meta wird freigegeben.
    fn meta_von(&self, media: *mut c_void, m: i32) -> Option<String> {
        if media.is_null() {
            return None;
        }
        // SAFETY: `media` ist gültig; das Ergebnis gehört uns und wird mit libvlc_free freigegeben.
        unsafe {
            let p = (self.f.libvlc_media_get_meta)(media, m as c_int);
            if p.is_null() {
                return None;
            }
            let s = CStr::from_ptr(p).to_string_lossy().trim().to_string();
            (self.f.libvlc_free)(p as *mut c_void);
            (!s.is_empty()).then_some(s)
        }
    }
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
        self.starten(m, true)
    }

    /// Adresse (http://, rtsp://, file:// …) laden und abspielen.
    pub fn oeffnen_adresse(&mut self, url: &str) -> Result<(), String> {
        let Some(neu) = self.core.o.libvlc_media_new_location else { return Err("Netzwerkstreams gehen mit diesem libVLC nicht".to_string()) };
        let c = CString::new(url.trim()).map_err(|_| "Adresse nicht lesbar".to_string())?;
        // SAFETY: gültige Instanz und nullterminierte Adresse.
        let m = unsafe { neu(self.core.inst, c.as_ptr()) };
        self.starten(m, false)
    }

    fn starten(&mut self, m: *mut c_void, lokal: bool) -> Result<(), String> {
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
            if lokal {
                // Angaben (Titel, Künstler, Albumbild) im Hintergrund lesen; 0x02 = lokal holen.
                (self.core.f.libvlc_media_parse_with_options)(m, 0x02, 5000);
            }
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

    /// Lautstärke 0–125 und stumm anwenden, falls libVLC schon eine Tonausgabe hat.
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
    pub fn meta(&self, m: i32) -> Option<String> {
        self.core.meta_von(self.media, m)
    }

    // -------------------------------------------------------------- Zusätze (optional)

    pub fn kann_adresse(&self) -> bool {
        self.core.o.libvlc_media_new_location.is_some()
    }

    pub fn kann_bild_fuer_bild(&self) -> bool {
        self.core.o.libvlc_media_player_next_frame.is_some()
    }

    pub fn bild_fuer_bild(&self) {
        if let Some(f) = self.core.o.libvlc_media_player_next_frame {
            // SAFETY: `mp` ist gültig.
            unsafe { f(self.mp) }
        }
    }

    /// Titel (DVD/Blu-ray): Anzahl und aktueller (−1 = unbekannt).
    pub fn titel_info(&self) -> (i32, i32) {
        match (self.core.o.libvlc_media_player_get_title_count, self.core.o.libvlc_media_player_get_title) {
            // SAFETY: `mp` ist gültig.
            (Some(n), Some(t)) => unsafe { (n(self.mp), t(self.mp)) },
            _ => (0, -1),
        }
    }

    pub fn setze_titel(&self, i: i32) {
        if let Some(f) = self.core.o.libvlc_media_player_set_title {
            // SAFETY: `mp` ist gültig; ungültige Nummern weist libVLC ab.
            unsafe { f(self.mp, i) }
        }
    }

    /// Kapitel: Anzahl und aktuelles (−1 = unbekannt).
    pub fn kapitel_info(&self) -> (i32, i32) {
        match (self.core.o.libvlc_media_player_get_chapter_count, self.core.o.libvlc_media_player_get_chapter) {
            // SAFETY: `mp` ist gültig.
            (Some(n), Some(k)) => unsafe { (n(self.mp), k(self.mp)) },
            _ => (0, -1),
        }
    }

    pub fn setze_kapitel(&self, i: i32) {
        if let Some(f) = self.core.o.libvlc_media_player_set_chapter {
            // SAFETY: `mp` ist gültig; ungültige Nummern weist libVLC ab.
            unsafe { f(self.mp, i) }
        }
    }

    pub fn kann_untertitel_datei(&self) -> bool {
        self.core.o.libvlc_media_player_add_slave.is_some()
    }

    /// Untertiteldatei («file:///…») dazunehmen und wählen.
    pub fn untertitel_datei(&self, uri: &str) -> bool {
        let (Some(f), Ok(c)) = (self.core.o.libvlc_media_player_add_slave, CString::new(uri)) else { return false };
        // SAFETY: `mp` ist gültig, `c` nullterminiert; 0 = libvlc_media_slave_type_subtitle.
        unsafe { f(self.mp, 0, c.as_ptr(), true) == 0 }
    }

    pub fn kann_untertitel_verzoegerung(&self) -> bool {
        self.core.o.libvlc_video_set_spu_delay.is_some()
    }

    /// Untertitel-Verzögerung in Millisekunden.
    pub fn untertitel_verzoegerung(&self, ms: i64) {
        if let Some(f) = self.core.o.libvlc_video_set_spu_delay {
            // SAFETY: `mp` ist gültig; libVLC rechnet in Mikrosekunden.
            unsafe {
                f(self.mp, ms.clamp(-36_000_000, 36_000_000).saturating_mul(1000));
            }
        }
    }

    pub fn kann_deinterlace(&self) -> bool {
        self.core.o.libvlc_video_set_deinterlace.is_some()
    }

    /// Deinterlace-Verfahren («blend», «yadif» …) oder aus (`None`).
    pub fn deinterlace(&self, modus: Option<&str>) {
        let Some(f) = self.core.o.libvlc_video_set_deinterlace else { return };
        let c = modus.and_then(|m| CString::new(m).ok());
        // SAFETY: `mp` ist gültig; null schaltet aus, sonst nullterminierter Name.
        unsafe { f(self.mp, c.as_ref().map(|c| c.as_ptr()).unwrap_or(std::ptr::null())) }
    }

    pub fn kann_anpassen(&self) -> bool {
        self.core.o.libvlc_video_set_adjust_int.is_some() && self.core.o.libvlc_video_set_adjust_float.is_some()
    }

    /// Bild anpassen (Filter «adjust»): Helligkeit, Kontrast, Sättigung, Farbton, Gamma –
    /// oder aus (`None`).
    pub fn anpassen(&self, werte: Option<[f32; 5]>) {
        let (Some(fi), Some(ff)) = (self.core.o.libvlc_video_set_adjust_int, self.core.o.libvlc_video_set_adjust_float) else { return };
        // libvlc_video_adjust_option_t: Enable 0, Contrast 1, Brightness 2, Hue 3, Saturation 4, Gamma 5
        // SAFETY: `mp` ist gültig; die Werte sind endlich und im Bereich von libVLC.
        unsafe {
            match werte {
                Some([hell, kontrast, saett, farbton, gamma]) => {
                    let ok = |v: f32, lo: f32, hi: f32, std: f32| if v.is_finite() { v.clamp(lo, hi) } else { std };
                    fi(self.mp, 0, 1);
                    ff(self.mp, 1, ok(kontrast, 0.0, 2.0, 1.0));
                    ff(self.mp, 2, ok(hell, 0.0, 2.0, 1.0));
                    ff(self.mp, 3, ok(farbton, -180.0, 180.0, 0.0));
                    ff(self.mp, 4, ok(saett, 0.0, 3.0, 1.0));
                    ff(self.mp, 5, ok(gamma, 0.01, 10.0, 1.0));
                }
                None => fi(self.mp, 0, 0),
            }
        }
    }

    pub fn kann_equalizer(&self) -> bool {
        let o = &self.core.o;
        o.libvlc_audio_equalizer_new.is_some()
            && o.libvlc_audio_equalizer_release.is_some()
            && o.libvlc_audio_equalizer_set_preamp.is_some()
            && o.libvlc_audio_equalizer_set_amp_at_index.is_some()
            && o.libvlc_media_player_set_equalizer.is_some()
            && o.libvlc_audio_equalizer_get_band_count.is_some()
    }

    /// Frequenzen der Equalizer-Bänder in Hz.
    pub fn equalizer_baender(&self) -> Vec<f32> {
        let (Some(n), Some(fq)) = (self.core.o.libvlc_audio_equalizer_get_band_count, self.core.o.libvlc_audio_equalizer_get_band_frequency) else { return Vec::new() };
        // SAFETY: Abfragen ohne Zustand; Nummern unter der gemeldeten Anzahl.
        unsafe { (0..n().min(32)).map(|i| fq(i)).filter(|f| f.is_finite()).collect() }
    }

    /// Namen der Equalizer-Vorlagen («Flat», «Rock» …).
    pub fn equalizer_vorlagen(&self) -> Vec<String> {
        let (Some(n), Some(name)) = (self.core.o.libvlc_audio_equalizer_get_preset_count, self.core.o.libvlc_audio_equalizer_get_preset_name) else { return Vec::new() };
        // SAFETY: Abfragen ohne Zustand; die Namen sind statische Zeichenketten (oder null).
        unsafe { (0..n().min(64)).map(|i| c_text(name(i))).collect() }
    }

    /// Vorverstärkung und Bänder einer Vorlage.
    pub fn equalizer_vorlage(&self, i: usize) -> Option<(f32, Vec<f32>)> {
        let o = &self.core.o;
        let (Some(neu), Some(frei), Some(pre), Some(amp), Some(n)) = (
            o.libvlc_audio_equalizer_new_from_preset,
            o.libvlc_audio_equalizer_release,
            o.libvlc_audio_equalizer_get_preamp,
            o.libvlc_audio_equalizer_get_amp_at_index,
            o.libvlc_audio_equalizer_get_band_count,
        ) else {
            return None;
        };
        let i = c_uint::try_from(i).ok()?;
        // SAFETY: `eq` wird geprüft und nach dem Lesen freigegeben.
        unsafe {
            let eq = neu(i);
            if eq.is_null() {
                return None;
            }
            let p = pre(eq);
            let v: Vec<f32> = (0..n().min(32)).map(|k| amp(eq, k)).map(|a| if a.is_finite() { a } else { 0.0 }).collect();
            frei(eq);
            Some((if p.is_finite() { p } else { 0.0 }, v))
        }
    }

    /// Equalizer setzen (Vorverstärkung, Bänder in dB) oder ausschalten (`None`). libVLC
    /// übernimmt die Werte auch für alle folgenden Medien.
    pub fn equalizer(&self, werte: Option<(f32, &[f32])>) -> bool {
        let o = &self.core.o;
        let Some(setzen) = o.libvlc_media_player_set_equalizer else { return false };
        let Some((pre, baender)) = werte else {
            // SAFETY: `mp` ist gültig; null schaltet den Equalizer aus.
            return unsafe { setzen(self.mp, std::ptr::null_mut()) == 0 };
        };
        let (Some(neu), Some(frei), Some(sp), Some(sa)) = (o.libvlc_audio_equalizer_new, o.libvlc_audio_equalizer_release, o.libvlc_audio_equalizer_set_preamp, o.libvlc_audio_equalizer_set_amp_at_index) else {
            return false;
        };
        let db = |v: f32| if v.is_finite() { v.clamp(-20.0, 20.0) } else { 0.0 };
        // SAFETY: `eq` wird geprüft; libVLC kopiert die Werte in set_equalizer, danach geben
        // wir `eq` frei.
        unsafe {
            let eq = neu();
            if eq.is_null() {
                return false;
            }
            sp(eq, db(pre));
            for (k, v) in baender.iter().enumerate().take(32) {
                sa(eq, db(*v), k as c_uint);
            }
            let ok = setzen(self.mp, eq) == 0;
            frei(eq);
            ok
        }
    }

    pub fn kann_audiogeraet(&self) -> bool {
        let o = &self.core.o;
        o.libvlc_audio_output_device_enum.is_some() && o.libvlc_audio_output_device_list_release.is_some() && o.libvlc_audio_output_device_set.is_some()
    }

    /// Audiogeräte (Kennung, Beschreibung).
    pub fn audiogeraete(&self) -> Vec<(String, String)> {
        let (Some(e), Some(frei)) = (self.core.o.libvlc_audio_output_device_enum, self.core.o.libvlc_audio_output_device_list_release) else { return Vec::new() };
        let mut v = Vec::new();
        // SAFETY: mit null abgeschlossene Liste von libVLC; einmal gelesen und freigegeben.
        unsafe {
            let liste = e(self.mp);
            let mut cur = liste;
            while !cur.is_null() && v.len() < 100 {
                let d = &*cur;
                let id = c_text(d.psz_device);
                let name = c_text(d.psz_description);
                if !id.is_empty() {
                    v.push((id, name));
                }
                cur = d.p_next;
            }
            if !liste.is_null() {
                frei(liste);
            }
        }
        v
    }

    pub fn audiogeraet(&self) -> Option<String> {
        let f = self.core.o.libvlc_audio_output_device_get?;
        // SAFETY: Ergebnis gehört uns und wird mit libvlc_free freigegeben.
        unsafe {
            let p = f(self.mp);
            if p.is_null() {
                return None;
            }
            let s = c_text(p);
            (self.core.f.libvlc_free)(p as *mut c_void);
            Some(s)
        }
    }

    pub fn setze_audiogeraet(&self, id: &str) {
        let (Some(f), Ok(c)) = (self.core.o.libvlc_audio_output_device_set, CString::new(id)) else { return };
        // SAFETY: `mp` gültig; Modul null = aktuelles Ausgabemodul.
        unsafe { f(self.mp, std::ptr::null(), c.as_ptr()) }
    }

    pub fn kann_stereo(&self) -> bool {
        self.core.o.libvlc_audio_get_channel.is_some() && self.core.o.libvlc_audio_set_channel.is_some()
    }

    /// Stereo-Modus: 1 Stereo, 2 umgekehrt, 3 links, 4 rechts, 5 Dolby Surround (−1 = unbekannt).
    pub fn stereo(&self) -> i32 {
        match self.core.o.libvlc_audio_get_channel {
            // SAFETY: `mp` ist gültig.
            Some(f) => unsafe { f(self.mp) },
            None => -1,
        }
    }

    pub fn setze_stereo(&self, m: i32) {
        if let Some(f) = self.core.o.libvlc_audio_set_channel {
            // SAFETY: `mp` ist gültig; ungültige Werte weist libVLC ab.
            unsafe {
                f(self.mp, m.clamp(1, 5));
            }
        }
    }

    /// Statistik der Wiedergabe (gelesene Bytes, Bilder …).
    pub fn statistik(&self) -> Option<Statistik> {
        let f = self.core.o.libvlc_media_get_stats?;
        if self.media.is_null() {
            return None;
        }
        let mut st = Statistik::default();
        // SAFETY: `media` gültig, `st` hat das Layout von libvlc_media_stats_t.
        let ok = unsafe { f(self.media, &mut st) };
        (ok != 0).then_some(st)
    }

    /// Spuren des aktuellen Mediums (Codec, Auflösung, Sprache …).
    pub fn spuren_info(&self) -> Vec<SpurInfo> {
        let (Some(get), Some(rel)) = (self.core.o.libvlc_media_tracks_get, self.core.o.libvlc_media_tracks_release) else { return Vec::new() };
        if self.media.is_null() {
            return Vec::new();
        }
        let desc = self.core.o.libvlc_media_get_codec_description;
        let mut v = Vec::new();
        // SAFETY: libVLC legt ein Feld mit `n` Zeigern an; wir lesen nur innerhalb davon,
        // prüfen jeden Zeiger und geben das Feld mit libvlc_media_tracks_release frei.
        unsafe {
            let mut feld: *mut *mut MediaTrack = std::ptr::null_mut();
            let n = get(self.media, &mut feld);
            if feld.is_null() {
                return v;
            }
            for k in 0..n.min(64) as usize {
                let tp = *feld.add(k);
                if tp.is_null() {
                    continue;
                }
                let t = &*tp;
                let fourcc = |c: u32| c.to_le_bytes().iter().map(|b| if b.is_ascii_graphic() || *b == b' ' { *b as char } else { '.' }).collect::<String>();
                let mut s = SpurInfo {
                    art: t.i_type,
                    codec: desc.map(|d| c_text(d(t.i_type, t.i_codec))).unwrap_or_default(),
                    fourcc: fourcc(t.i_codec),
                    sprache: c_text(t.psz_language),
                    beschreibung: c_text(t.psz_description),
                    bitrate: t.i_bitrate,
                    ..SpurInfo::default()
                };
                if !t.u.is_null() {
                    match t.i_type {
                        0 => {
                            let a = &*(t.u as *const AudioTrack);
                            s.kanaele = a.i_channels;
                            s.rate = a.i_rate;
                        }
                        1 => {
                            let vt = &*(t.u as *const VideoTrack);
                            s.breite = vt.i_width;
                            s.hoehe = vt.i_height;
                            s.sar = (vt.i_sar_num, vt.i_sar_den);
                            if vt.i_frame_rate_den > 0 {
                                s.fps = vt.i_frame_rate_num as f64 / vt.i_frame_rate_den as f64;
                            }
                        }
                        _ => {}
                    }
                }
                v.push(s);
            }
            rel(feld, n);
        }
        v
    }

    /// Pixel-Seitenverhältnis der ersten Videospur (Zähler, Nenner), falls bekannt.
    pub fn sar(&self) -> Option<(u32, u32)> {
        self.spuren_info().into_iter().find(|s| s.art == 1 && s.sar.0 > 0 && s.sar.1 > 0).map(|s| s.sar)
    }

    /// Das zuletzt gezeigte Videobild als RGB (für ein Bildschirmfoto).
    pub fn foto(&self) -> Option<Foto> {
        let p = sperren(&self.vmem.puffer);
        let (w, h, pitch) = (p.w, p.h, p.pitch);
        let noetig = pitch.checked_mul(h)?;
        if w == 0 || h == 0 || p.daten.len() < noetig || pitch < w.checked_mul(4)? {
            return None;
        }
        let mut rgb = Vec::with_capacity(w.checked_mul(h)?.checked_mul(3)?);
        for zeile in 0..h {
            let start = zeile * pitch;
            let z = p.daten.get(start..start + w * 4)?;
            for c in z.chunks_exact(4) {
                // RV32: B, G, R, X
                rgb.extend_from_slice(&[c[2], c[1], c[0]]);
            }
        }
        Some(Foto { breite: w, hoehe: h, rgb })
    }

    /// Medium im Hintergrund prüfen (Dauer, Titel, Album) für die Wiedergabeliste.
    pub fn probe(&self, pfad: &Path) -> Option<Probe> {
        self.core.o.libvlc_media_get_parsed_status?;
        self.core.o.libvlc_media_get_duration?;
        let c = CString::new(pfad.to_str()?).ok()?;
        // SAFETY: gültige Instanz und nullterminierter Pfad.
        let m = unsafe { (self.core.f.libvlc_media_new_path)(self.core.inst, c.as_ptr()) };
        if m.is_null() {
            return None;
        }
        // SAFETY: `m` ist gültig; 0 = nur lokal lesen, Zeitlimit 4 s. Läuft im Hintergrund.
        if unsafe { (self.core.f.libvlc_media_parse_with_options)(m, 0, 4000) } != 0 {
            // SAFETY: unser Verweis auf `m`.
            unsafe { (self.core.f.libvlc_media_release)(m) };
            return None;
        }
        Some(Probe { core: self.core.clone(), m, pfad: pfad.to_path_buf(), seit: std::time::Instant::now() })
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

/// Medium, das libVLC im Hintergrund liest (Dauer und Angaben für die Wiedergabeliste).
pub struct Probe {
    core: Rc<Core>,
    m: *mut c_void,
    pub pfad: PathBuf,
    seit: std::time::Instant,
}

impl Probe {
    /// `None` = noch nicht fertig, `Some(true)` = gelesen, `Some(false)` = ging nicht.
    pub fn fertig(&self) -> Option<bool> {
        let f = self.core.o.libvlc_media_get_parsed_status?;
        // SAFETY: `m` ist gültig, bis `Probe` freigegeben wird.
        match unsafe { f(self.m) } {
            4 => Some(true),
            1..=3 => Some(false),
            _ if self.seit.elapsed() > std::time::Duration::from_secs(8) => Some(false),
            _ => None,
        }
    }

    /// Dauer in Millisekunden (falls bekannt).
    pub fn dauer(&self) -> Option<i64> {
        let f = self.core.o.libvlc_media_get_duration?;
        // SAFETY: `m` ist gültig.
        let d = unsafe { f(self.m) };
        (d > 0).then_some(d)
    }

    pub fn meta(&self, m: i32) -> Option<String> {
        self.core.meta_von(self.m, m)
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        // SAFETY: `m` ist gültig; ein laufendes Lesen wird zuerst beendet, dann unser Verweis
        // freigegeben.
        unsafe {
            let laeuft = self.core.o.libvlc_media_get_parsed_status.is_some_and(|f| f(self.m) == 0);
            if let (true, Some(stop)) = (laeuft, self.core.o.libvlc_media_parse_stop) {
                stop(self.m);
            }
            (self.core.f.libvlc_media_release)(self.m);
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ende-zu-Ende mit echtem libVLC (CI: .github/workflows/abspielwerk-test.yml). Ohne
    //! libVLC wird der Test übersprungen. Die Testdateien (Y4M-Video, WAV, SRT) erzeugt der
    //! Test selbst. `ABSPIELWERK_LIBVLC` kann auf eine bestimmte libvlc-Datei zeigen.
    use super::*;
    use std::time::{Duration, Instant};

    fn warte_bild(p: &Player, s: u64) -> Option<egui::ColorImage> {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(s) {
            if let Some(b) = p.neues_bild() {
                return Some(b);
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        None
    }

    fn testdateien(dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
        std::fs::create_dir_all(dir).expect("Testordner");
        // 4 s Video 64×48, 25 Bilder/s, Graustufen-Verlauf, der sich bewegt
        let (w, h, n) = (64usize, 48usize, 100usize);
        let mut y4m = format!("YUV4MPEG2 W{w} H{h} F25:1 Ip A1:1 C420jpeg\n").into_bytes();
        for f in 0..n {
            y4m.extend_from_slice(b"FRAME\n");
            for y in 0..h {
                for x in 0..w {
                    y4m.push(((x * 4 + y + f * 3) % 256) as u8);
                }
            }
            y4m.extend(std::iter::repeat(128u8).take(w * h / 2));
        }
        let video = dir.join("test video ä.y4m");
        std::fs::write(&video, y4m).expect("y4m");
        // 2 s WAV, 440 Hz, Mono 16 bit
        let rate = 22050u32;
        let samples: Vec<i16> = (0..rate * 2).map(|i| ((i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 8000.0) as i16).collect();
        let daten = samples.len() as u32 * 2;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + daten).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&daten.to_le_bytes());
        for s in samples {
            wav.extend_from_slice(&s.to_le_bytes());
        }
        let musik = dir.join("ton.wav");
        std::fs::write(&musik, wav).expect("wav");
        let srt = dir.join("test video ä.srt");
        std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:04,000\nHallo Abspielwerk\n\n").expect("srt");
        (video, musik, srt)
    }

    #[test]
    fn libvlc_ende_zu_ende() {
        let kandidaten = match std::env::var_os("ABSPIELWERK_LIBVLC") {
            Some(p) => vec![Kandidat { datei: PathBuf::from(p) }],
            None => vorbereiten(),
        };
        let core = match laden(&kandidaten) {
            Ok(c) => c,
            Err(e) => {
                if std::env::var_os("ABSPIELWERK_LIBVLC_PFLICHT").is_some() {
                    panic!("libVLC fehlt: {e}");
                }
                eprintln!("übersprungen: libVLC fehlt ({e})");
                return;
            }
        };
        println!("libVLC {} aus {}", core.version, core.datei.display());
        let dir = std::env::temp_dir().join(format!("abspielwerk-test-{}", std::process::id()));
        let (video, musik, srt) = testdateien(&dir);
        let mut p = Player::neu(Rc::new(core), egui::Context::default()).expect("Player");

        // Video öffnen und erstes Bild abwarten
        p.oeffnen(&video).expect("Video öffnen");
        let b = warte_bild(&p, 30).expect("kein Videobild von libVLC");
        println!("Videobild {}×{}", b.size[0], b.size[1]);
        assert_eq!(b.size, [64, 48]);
        let start = Instant::now();
        while p.zustand() != Zustand::Spielt && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(p.zustand(), Zustand::Spielt);

        // Bildschirmfoto aus dem Puffer, als PNG schreiben
        let f = p.foto().expect("Foto");
        assert_eq!((f.breite, f.hoehe), (64, 48));
        assert_eq!(f.rgb.len(), 64 * 48 * 3);
        let png = dir.join("abspielwerk-test.png");
        image::save_buffer_with_format(&png, &f.rgb, 64, 48, image::ColorType::Rgb8, image::ImageFormat::Png).expect("PNG");
        assert!(std::fs::metadata(&png).map(|m| m.len() > 50).unwrap_or(false));
        println!("Bildschirmfoto: {}", png.display());

        // Spuren, Pixel-Seitenverhältnis, Statistik
        let spuren = p.spuren_info();
        println!("Spuren: {spuren:?}");
        assert!(spuren.iter().any(|s| s.art == 1 && s.breite == 64), "keine Videospur");
        println!("SAR {:?}", p.sar());
        println!("Statistik {:?}", p.statistik());

        // Equalizer
        assert!(p.kann_equalizer(), "Equalizer fehlt");
        let vorlagen = p.equalizer_vorlagen();
        println!("Equalizer-Vorlagen: {vorlagen:?}");
        assert!(vorlagen.len() >= 10);
        let baender = p.equalizer_baender();
        assert_eq!(baender.len(), 10);
        let (vor, amp) = p.equalizer_vorlage(vorlagen.len() - 1).expect("Vorlage");
        assert_eq!(amp.len(), baender.len());
        assert!(p.equalizer(Some((vor, amp.as_slice()))));
        assert!(p.equalizer(Some((f32::NAN, &[f32::INFINITY, 99.0][..]))));
        assert!(p.equalizer(None));
        assert!(p.equalizer_vorlage(9999).is_none());

        // Video-Anpassungen und Deinterlace: es kommen weiter Bilder
        assert!(p.kann_anpassen());
        p.anpassen(Some([1.3, 1.2, 1.5, 40.0, 1.1]));
        assert!(p.kann_deinterlace());
        p.deinterlace(Some("yadif"));
        assert!(warte_bild(&p, 10).is_some(), "nach Anpassen/Deinterlace keine Bilder mehr");
        p.anpassen(Some([f32::NAN, -5.0, 99.0, 1e9, 0.0]));
        p.anpassen(None);
        p.deinterlace(None);

        // Untertitel: Datei dazu, Verzögerung, Spurwahl
        assert!(p.untertitel_datei(&crate::merken::pfad_zu_url(&srt.to_string_lossy())), "Untertiteldatei");
        p.untertitel_verzoegerung(500);
        p.untertitel_verzoegerung(i64::MAX);
        p.untertitel_verzoegerung(0);
        std::thread::sleep(Duration::from_millis(500));
        println!("Untertitel: {:?} aktiv {}", p.untertitel_liste(), p.untertitel());

        // Ton, Tempo, Geräte, Stereo, Titel/Kapitel
        p.ton(125, false);
        p.tempo(2.0);
        p.tempo(f32::NAN);
        p.tempo(1.0);
        println!("Audiogeräte {:?}, aktuell {:?}, Stereo {}", p.audiogeraete(), p.audiogeraet(), p.stereo());
        p.setze_stereo(99);
        println!("Titel {:?}, Kapitel {:?}", p.titel_info(), p.kapitel_info());
        p.setze_titel(-5);
        p.setze_kapitel(1000);

        // Spulen, Pause, Bild für Bild
        p.setze_zeit(2000);
        p.pause(true);
        std::thread::sleep(Duration::from_millis(400));
        assert!(p.kann_bild_fuer_bild());
        let _ = p.neues_bild();
        p.bild_fuer_bild();
        p.bild_fuer_bild();
        std::thread::sleep(Duration::from_millis(300));
        p.pause(false);

        // Im Hintergrund lesen (Liste: Dauer)
        let probe = p.probe(&musik).expect("Probe");
        let start = Instant::now();
        let mut ok = None;
        while ok.is_none() && start.elapsed() < Duration::from_secs(15) {
            ok = probe.fertig();
            std::thread::sleep(Duration::from_millis(50));
        }
        println!("Probe WAV: {ok:?}, Dauer {:?}", probe.dauer());
        assert_eq!(ok, Some(true));
        assert!(probe.dauer().is_some_and(|d| (1500..=2500).contains(&d)), "Dauer {:?}", probe.dauer());
        drop(probe);
        // Probe, die mitten im Lesen freigegeben wird
        drop(p.probe(&video));

        // Gleiche Datei als Adresse (wie Netzwerkstream) öffnen
        assert!(p.kann_adresse());
        p.oeffnen_adresse(&crate::merken::pfad_zu_url(&video.to_string_lossy())).expect("Adresse");
        assert!(warte_bild(&p, 20).is_some(), "kein Bild über file://-Adresse");
        assert!(p.oeffnen_adresse("a\0b").is_err());
        // Fehlende Datei: darf höchstens einen Fehlerzustand geben
        let _ = p.oeffnen(Path::new("/gibt/es/nicht.mkv"));
        std::thread::sleep(Duration::from_millis(500));
        println!("fehlende Datei: {:?}", p.zustand());

        // Musik
        p.oeffnen(&musik).expect("WAV öffnen");
        std::thread::sleep(Duration::from_millis(800));
        println!("WAV: Zustand {:?}, Länge {}", p.zustand(), p.laenge());
        p.stop();
        let _ = p.foto();
        drop(p);
        let _ = std::fs::remove_dir_all(&dir);
        println!("libVLC-Test fertig");
    }
}
