//! Reine Datenlogik ohne Oberfläche und ohne Plattform-APIs: Einstellungen,
//! Bitratenwahl, Zeitstempel-Dateiname, Timecode und das Zurechtrücken eines
//! frei gewählten Bereichs. Nur Standardbibliothek – darum gut testbar.

/// Was aufgenommen wird.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Ganzer Monitor.
    Fullscreen,
    /// Ein einzelnes Fenster.
    Window,
    /// Frei gewählter Ausschnitt eines Monitors.
    Region,
}

/// Qualitätsstufe – bestimmt die Ziel-Bitrate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Quality {
    Hoch,
    Mittel,
    Sparsam,
}

impl Quality {
    /// Bits pro Pixel und Bild (grobe Richtwerte für H.264).
    fn bpp(self) -> f64 {
        match self {
            Quality::Hoch => 0.12,
            Quality::Mittel => 0.07,
            Quality::Sparsam => 0.035,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Quality::Hoch => "Hoch",
            Quality::Mittel => "Mittel",
            Quality::Sparsam => "Datensparend",
        }
    }
}

/// Ecke für das Webcam-Overlay.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Corner {
    ObenLinks,
    ObenRechts,
    UntenLinks,
    UntenRechts,
}

/// Alle Nutzer-Einstellungen in einem Objekt (von der Oberfläche gefüllt).
#[derive(Clone)]
pub struct Settings {
    pub mode: Mode,
    pub monitor_index: usize,
    pub window_title: String,
    /// Bereich in Bildschirmkoordinaten (nur bei Mode::Region).
    pub region: (i32, i32, u32, u32),
    pub fps: u32,
    pub quality: Quality,
    pub system_audio: bool,
    pub microphone: bool,
    pub mic_device: String,
    pub webcam: bool,
    pub webcam_corner: Corner,
    /// Webcam-Breite in Prozent der Videobreite (Höhe folgt 16:9).
    pub webcam_percent: u32,
    pub highlight_cursor: bool,
    pub highlight_clicks: bool,
    pub countdown: u32,
    pub out_dir: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            mode: Mode::Fullscreen,
            monitor_index: 0,
            window_title: String::new(),
            region: (0, 0, 1280, 720),
            fps: 60,
            quality: Quality::Hoch,
            system_audio: true,
            microphone: false,
            mic_device: String::new(),
            webcam: false,
            webcam_corner: Corner::UntenRechts,
            webcam_percent: 20,
            highlight_cursor: true,
            highlight_clicks: true,
            countdown: 3,
            out_dir: String::new(),
        }
    }
}

/// Nächstkleinere gerade Zahl (H.264 braucht gerade Kantenlängen).
pub fn even(n: u32) -> u32 {
    n & !1
}

/// Ziel-Bitrate in Bit/s aus Auflösung, Bildrate und Qualität.
/// Begrenzt auf einen sinnvollen Bereich (2–50 Mbit/s).
pub fn pick_bitrate(w: u32, h: u32, fps: u32, q: Quality) -> u32 {
    let raw = (w as f64) * (h as f64) * (fps as f64) * q.bpp();
    let clamped = raw.clamp(2_000_000.0, 50_000_000.0);
    clamped as u32
}

/// Dateiname mit Zeitstempel, Windows-tauglich (keine Doppelpunkte).
/// Beispiel: "Aufnahme 2026-10-16 14-22-05.mp4".
pub fn output_filename(y: i32, mon: u32, d: u32, hh: u32, mm: u32, ss: u32) -> String {
    format!(
        "Aufnahme {:04}-{:02}-{:02} {:02}-{:02}-{:02}.mp4",
        y, mon, d, hh, mm, ss
    )
}

/// Dauer als Timecode HH:MM:SS.
pub fn fmt_timecode(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

/// Rückt einen gewählten Bereich so zurecht, dass er ganz auf dem Monitor
/// liegt, eine Mindestgrösse hat und gerade Kantenlängen besitzt.
/// Gibt (x, y, breite, höhe) zurück.
pub fn clamp_region(
    rx: i32,
    ry: i32,
    rw: u32,
    rh: u32,
    screen_w: u32,
    screen_h: u32,
) -> (i32, i32, u32, u32) {
    let sw = screen_w as i32;
    let sh = screen_h as i32;
    // Ursprung in den sichtbaren Bereich schieben.
    let x = rx.clamp(0, (sw - 2).max(0));
    let y = ry.clamp(0, (sh - 2).max(0));
    // Breite/Höhe auf das begrenzen, was ab (x,y) noch Platz hat.
    let max_w = (sw - x).max(0) as u32;
    let max_h = (sh - y).max(0) as u32;
    let w = even(rw.clamp(2, max_w).max(2));
    let h = even(rh.clamp(2, max_h).max(2));
    (x, y, w, h)
}
