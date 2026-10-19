//! Reine Logik von Diawerk: Einstellungen, Dateierkennung, Dateien ausmessen
//! (JPEG-Kopf bzw. `ffmpeg -i`), Zeitplan der Diashow und der Aufbau der
//! ffmpeg-Filter. Nur Standardbibliothek – eigenständig mit `rustc` testbar.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

// ---------------------------------------------------------------- Typen

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Image,
    Video,
    /// Vorspann bzw. Abspann (wird nicht in der Liste geführt, sondern beim Rendern ergänzt).
    Intro,
    Outro,
}

/// Eine Datei der Liste (Bild oder Drohnenvideo), bereits ausgemessen.
#[derive(Clone, Debug)]
pub struct Item {
    pub path: PathBuf,
    pub name: String,
    pub kind: Kind,
    /// Breite/Höhe so, wie das Bild später aussieht (EXIF-Drehung eingerechnet).
    pub w: u32,
    pub h: u32,
    /// EXIF-Ausrichtung 1..=8 (1 = unverändert). Nur bei Bildern.
    pub orient: u8,
    /// Länge in Sekunden (nur Video).
    pub secs: f32,
    pub modified: Option<SystemTime>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Transition {
    /// Ruhige Abfolge aus Überblendung (meist), Wischen, Schieben und Zoom.
    Mix,
    Crossfade,
    Push,
    Wipe,
    Zoom,
}

impl Transition {
    pub const ALL: [Transition; 5] = [
        Transition::Mix,
        Transition::Crossfade,
        Transition::Push,
        Transition::Wipe,
        Transition::Zoom,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Transition::Mix => "Gemischt (ruhig)",
            Transition::Crossfade => "Überblenden",
            Transition::Push => "Schieben",
            Transition::Wipe => "Wischen",
            Transition::Zoom => "Zoom-Überblendung",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Quality {
    Draft,
    Good,
    Best,
}

impl Quality {
    pub const ALL: [Quality; 3] = [Quality::Draft, Quality::Good, Quality::Best];
    pub fn label(self) -> &'static str {
        match self {
            Quality::Draft => "Entwurf (schnell)",
            Quality::Good => "Gut",
            Quality::Best => "Sehr gut (langsam)",
        }
    }
    /// (x264-Preset, CRF)
    pub fn x264(self) -> (&'static str, u32) {
        match self {
            Quality::Draft => ("veryfast", 27),
            Quality::Good => ("medium", 20),
            Quality::Best => ("slow", 17),
        }
    }
}

/// Ausgabeformate (wie in Wandelwerk): MP4, MKV, MOV mit H.264, WebM mit VP9.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutFmt {
    Mp4,
    Mkv,
    Mov,
    Webm,
}

impl OutFmt {
    pub const ALL: [OutFmt; 4] = [OutFmt::Mp4, OutFmt::Mkv, OutFmt::Mov, OutFmt::Webm];
    pub fn ext(self) -> &'static str {
        match self {
            OutFmt::Mp4 => "mp4",
            OutFmt::Mkv => "mkv",
            OutFmt::Mov => "mov",
            OutFmt::Webm => "webm",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            OutFmt::Mp4 => "MP4 (H.264) – läuft überall",
            OutFmt::Mkv => "MKV (H.264)",
            OutFmt::Mov => "MOV (H.264)",
            OutFmt::Webm => "WebM (VP9) – Browser",
        }
    }
    /// Video- und Tonkodierung als ffmpeg-Argumente.
    pub fn codec_args(self, q: Quality, audio: bool) -> Vec<String> {
        let mut a: Vec<String> = Vec::new();
        let mut push = |xs: &[&str]| a.extend(xs.iter().map(|x| x.to_string()));
        match self {
            OutFmt::Webm => {
                let (crf, cpu) = match q {
                    Quality::Draft => ("38", "8"),
                    Quality::Good => ("32", "4"),
                    Quality::Best => ("28", "2"),
                };
                push(&["-c:v", "libvpx-vp9", "-crf", crf, "-b:v", "0", "-row-mt", "1", "-cpu-used", cpu]);
                if audio {
                    push(&["-c:a", "libopus", "-b:a", "128k"]);
                }
            }
            _ => {
                let (preset, crf) = q.x264();
                push(&["-c:v", "libx264", "-preset", preset, "-crf", &crf.to_string()]);
                if audio {
                    push(&["-c:a", "aac", "-b:a", "192k"]);
                }
                if self != OutFmt::Mkv {
                    push(&["-movflags", "+faststart"]);
                }
            }
        }
        push(&["-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709"]);
        a
    }
}

/// Vor- und Abspann mit dem Fotografen (Porträt, Name, Titel, Text).
#[derive(Clone, Debug)]
pub struct Credits {
    pub intro: bool,
    pub outro: bool,
    pub portrait: Option<PathBuf>,
    pub intro_kicker: String,
    pub name: String,
    pub intro_text: String,
    pub outro_kicker: String,
    pub outro_text: String,
    pub card_secs: f32,
    /// Eigene Schriftdatei (.ttf/.otf); leer = passende Systemschrift suchen.
    pub font: Option<PathBuf>,
}

impl Default for Credits {
    fn default() -> Self {
        Credits {
            intro: true,
            outro: true,
            portrait: None,
            intro_kicker: "Fotograf und Gestalter HF, Kommunikationsdesign".into(),
            name: "Samuel Büttler".into(),
            intro_text: "Der Mann hinter der Kamera: unkompliziert, lösungsorientiert und mit dem Auge für Formen und Farben".into(),
            outro_kicker: "Fotografien von".into(),
            outro_text: "Vielen Dank für diese Bilder.".into(),
            card_secs: 8.0,
            font: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Standzeit eines normalen Bildes (inkl. Übergänge).
    pub still_secs: f32,
    pub trans_secs: f32,
    pub transition: Transition,
    /// Sanfter Zoom bei normalen Bildern in Prozent (0 = aus).
    pub zoom_pct: f32,
    /// true: Bilder nahe 16:9 füllen den Schirm (Rand wird beschnitten);
    /// false: Bilder werden nie beschnitten (unscharfer Hintergrund).
    pub crop_to_fill: bool,
    /// Ab diesem Seitenverhältnis (Breite : Höhe) gilt ein Bild als Panorama.
    pub pano_ratio: f32,
    /// Schwenktempo in Pixel pro Sekunde bei 1080 Zeilen Höhe.
    pub pano_speed: f32,
    pub pano_min_secs: f32,
    pub pano_max_secs: f32,
    /// Ruhe am Anfang und Ende des Schwenks (zusätzlich zum Übergang).
    pub pano_pause: f32,
    /// Längste Spielzeit eines Videos in Sekunden (0 = ganze Länge).
    pub video_max_secs: f32,
    pub quality: Quality,
    pub format: OutFmt,
    /// Musik in dieser Reihenfolge (MP3 u. a.); wird bei Bedarf wiederholt.
    pub music: Vec<PathBuf>,
    pub credits: Credits,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            width: 1920,
            height: 1080,
            fps: 30,
            still_secs: 6.0,
            trans_secs: 1.0,
            transition: Transition::Mix,
            zoom_pct: 5.0,
            crop_to_fill: true,
            pano_ratio: 2.2,
            pano_speed: 240.0,
            pano_min_secs: 8.0,
            pano_max_secs: 25.0,
            pano_pause: 0.5,
            video_max_secs: 0.0,
            quality: Quality::Good,
            format: OutFmt::Mp4,
            music: Vec::new(),
            credits: Credits::default(),
        }
    }
}

// ---------------------------------------------------------------- Dateitypen, Sortieren

pub const IMAGE_EXT: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "tif", "tiff", "gif"];
pub const VIDEO_EXT: &[&str] = &["mp4", "mov", "m4v", "mkv", "avi", "webm", "mts", "m2ts"];

pub fn detect_kind(p: &Path) -> Option<Kind> {
    let ext = p.extension()?.to_string_lossy().to_lowercase();
    if IMAGE_EXT.contains(&ext.as_str()) {
        Some(Kind::Image)
    } else if VIDEO_EXT.contains(&ext.as_str()) {
        Some(Kind::Video)
    } else {
        None
    }
}

/// Sortierung „natürlich": img2 vor img10, Gross-/Kleinschreibung egal.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(&c) = x.peek() {
                    if c.is_ascii_digit() {
                        na.push(c);
                        x.next();
                    } else {
                        break;
                    }
                }
                let mut nb = String::new();
                while let Some(&c) = y.peek() {
                    if c.is_ascii_digit() {
                        nb.push(c);
                        y.next();
                    } else {
                        break;
                    }
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(c), Some(d)) => {
                if c != d {
                    return c.cmp(&d);
                }
                x.next();
                y.next();
            }
        }
    }
}

/// Alle unterstützten Dateien eines Ordners, natürlich nach Namen sortiert.
pub fn collect_files(dir: &Path, recursive: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(dir, recursive, &mut out);
    out.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
    out
}

fn walk(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if recursive {
                walk(&p, recursive, out);
            }
        } else if detect_kind(&p).is_some() {
            out.push(p);
        }
    }
}

// ---------------------------------------------------------------- Ausmessen

/// Was `ffmpeg -i` über eine Datei verrät.
#[derive(Debug, PartialEq)]
pub struct Info {
    pub w: u32,
    pub h: u32,
    pub secs: Option<f32>,
}

/// Liest Grösse, Drehung und Dauer aus der Textausgabe von `ffmpeg -i datei`.
pub fn parse_ffmpeg_info(text: &str) -> Option<Info> {
    let mut secs = None;
    let mut dims = None;
    let mut rot = 0.0f32;
    for line in text.lines() {
        let l = line.trim();
        if secs.is_none() {
            if let Some(rest) = l.strip_prefix("Duration:") {
                let t = rest.trim().split(',').next().unwrap_or("").trim();
                secs = parse_hms(t);
            }
        }
        if dims.is_none() && l.contains("Video:") && l.starts_with("Stream") {
            for piece in l.split_whitespace() {
                let piece = piece.trim_end_matches(',');
                if let Some((a, b)) = piece.split_once('x') {
                    if !a.starts_with('0')
                        && !b.starts_with('0')
                        && a.chars().all(|c| c.is_ascii_digit())
                        && b.chars().all(|c| c.is_ascii_digit())
                    {
                        if let (Ok(w), Ok(h)) = (a.parse::<u32>(), b.parse::<u32>()) {
                            if w >= 16 && h >= 16 {
                                dims = Some((w, h));
                                break;
                            }
                        }
                    }
                }
            }
        }
        if let Some(i) = l.find("rotation of") {
            let num: String = l[i + 11..]
                .trim()
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '-' || *c == '.')
                .collect();
            if let Ok(v) = num.parse::<f32>() {
                rot = v;
            }
        }
    }
    let (mut w, mut h) = dims?;
    let r = (rot.abs().round() as i64) % 180;
    if r == 90 {
        std::mem::swap(&mut w, &mut h);
    }
    Some(Info { w, h, secs })
}

fn parse_hms(t: &str) -> Option<f32> {
    let mut parts = t.split(':');
    let h: f32 = parts.next()?.trim().parse().ok()?;
    let m: f32 = parts.next()?.trim().parse().ok()?;
    let s: f32 = parts.next()?.trim().parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

/// Dauer einer Ton- oder Videodatei aus der Textausgabe von `ffmpeg -i`.
pub fn parse_duration(text: &str) -> Option<f32> {
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("Duration:") {
            return parse_hms(rest.trim().split(',').next().unwrap_or("").trim());
        }
    }
    None
}

/// Länge einer Musikdatei in Sekunden.
pub fn audio_secs(ffmpeg: &str, path: &Path) -> Option<f32> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-i"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    hide_window(&mut cmd);
    let out = cmd.output().ok()?;
    parse_duration(&String::from_utf8_lossy(&out.stderr))
}

fn ffmpeg_info(ffmpeg: &str, path: &Path) -> Result<Info, String> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-i"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    hide_window(&mut cmd);
    let out = cmd
        .output()
        .map_err(|e| format!("ffmpeg konnte nicht gestartet werden: {e}"))?;
    let text = String::from_utf8_lossy(&out.stderr);
    parse_ffmpeg_info(&text).ok_or_else(|| "keine Bild-/Videodaten gefunden".to_string())
}

/// Windows: kein aufblitzendes Konsolenfenster beim Start von ffmpeg.
pub fn hide_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// JPEG-Kopf lesen: (Breite, Höhe, EXIF-Ausrichtung). Schnell, ohne ffmpeg.
pub fn jpeg_info(path: &Path) -> Option<(u32, u32, u8)> {
    let mut f = BufReader::new(File::open(path).ok()?);
    let mut b2 = [0u8; 2];
    f.read_exact(&mut b2).ok()?;
    if b2 != [0xFF, 0xD8] {
        return None;
    }
    let mut orient = 1u8;
    loop {
        // Marker suchen
        let mut b = [0u8; 1];
        f.read_exact(&mut b).ok()?;
        if b[0] != 0xFF {
            continue;
        }
        let mut m = [0xFFu8; 1];
        while m[0] == 0xFF {
            f.read_exact(&mut m).ok()?;
        }
        let marker = m[0];
        if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            continue; // Marker ohne Länge
        }
        if marker == 0xD9 {
            return None;
        }
        f.read_exact(&mut b2).ok()?;
        let len = u16::from_be_bytes(b2) as usize;
        if len < 2 {
            return None;
        }
        let body = len - 2;
        match marker {
            0xE1 => {
                let mut data = vec![0u8; body];
                f.read_exact(&mut data).ok()?;
                if let Some(o) = exif_orientation(&data) {
                    orient = o;
                }
            }
            0xC0..=0xCF if marker != 0xC4 && marker != 0xC8 && marker != 0xCC => {
                let mut data = vec![0u8; body.min(16)];
                f.read_exact(&mut data).ok()?;
                if data.len() < 5 {
                    return None;
                }
                let h = u16::from_be_bytes([data[1], data[2]]) as u32;
                let w = u16::from_be_bytes([data[3], data[4]]) as u32;
                if w == 0 || h == 0 {
                    return None;
                }
                let (w, h) = if (5..=8).contains(&orient) { (h, w) } else { (w, h) };
                return Some((w, h, orient));
            }
            0xDA => return None, // Bilddaten ohne Rahmenkopf
            _ => {
                f.seek(SeekFrom::Current(body as i64)).ok()?;
            }
        }
    }
}

/// Liest die Ausrichtung (Tag 0x0112) aus einem APP1-Block „Exif".
pub fn exif_orientation(app1: &[u8]) -> Option<u8> {
    if app1.len() < 14 || &app1[0..6] != b"Exif\0\0" {
        return None;
    }
    let t = &app1[6..];
    let le = match &t[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let rd16 = |o: usize| -> Option<u16> {
        let b = t.get(o..o + 2)?;
        Some(if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) })
    };
    let rd32 = |o: usize| -> Option<u32> {
        let b = t.get(o..o + 4)?;
        Some(if le {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        })
    };
    if rd16(2)? != 42 {
        return None;
    }
    let ifd = rd32(4)? as usize;
    let n = rd16(ifd)? as usize;
    for i in 0..n {
        let e = ifd + 2 + i * 12;
        if rd16(e)? == 0x0112 {
            let v = rd16(e + 8)?;
            return if (1..=8).contains(&v) { Some(v as u8) } else { None };
        }
    }
    None
}

/// Misst eine Datei aus (Bild oder Video).
pub fn probe(ffmpeg: &str, path: &Path) -> Result<Item, String> {
    let kind = detect_kind(path).ok_or("Dateityp nicht unterstützt")?;
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let (w, h, orient, secs) = match kind {
        Kind::Intro | Kind::Outro => return Err("kein Medium".into()),
        Kind::Image => {
            let jp = path
                .extension()
                .map(|e| {
                    let e = e.to_string_lossy().to_lowercase();
                    e == "jpg" || e == "jpeg"
                })
                .unwrap_or(false);
            match if jp { jpeg_info(path) } else { None } {
                Some((w, h, o)) => (w, h, o, 0.0),
                None => {
                    let i = ffmpeg_info(ffmpeg, path)?;
                    (i.w, i.h, 1, 0.0)
                }
            }
        }
        Kind::Video => {
            let i = ffmpeg_info(ffmpeg, path)?;
            let secs = i.secs.ok_or("Videolänge unbekannt")?;
            (i.w, i.h, 1, secs)
        }
    };
    Ok(Item { path: path.to_path_buf(), name, kind, w, h, orient, secs, modified })
}

// ---------------------------------------------------------------- Zeitplan

/// Hochformat: Bild oder Video, das (nach der Drehung) höher als breit ist.
pub fn is_portrait(it: &Item) -> bool {
    matches!(it.kind, Kind::Image | Kind::Video) && it.h > it.w
}

/// Zeilen der Liste, die im Hochformat sind.
pub fn portrait_rows(items: &[Item]) -> Vec<usize> {
    items.iter().enumerate().filter(|(_, it)| is_portrait(it)).map(|(i, _)| i).collect()
}

pub fn is_pano(it: &Item, s: &Settings) -> bool {
    it.kind == Kind::Image && it.h > 0 && it.w as f32 / it.h as f32 >= s.pano_ratio
}

/// Breite eines Panoramas, wenn es auf die Bildhöhe skaliert ist.
pub fn pano_width(it: &Item, s: &Settings) -> u32 {
    ((s.height as f64) * it.w as f64 / it.h.max(1) as f64).round().max(1.0) as u32
}

/// Frames für eine Zeit in Sekunden.
pub fn frames_of(secs: f32, fps: u32) -> u32 {
    (secs * fps as f32).round().max(1.0) as u32
}

/// Länge eines Clips in Sekunden (einschliesslich der Übergänge an beiden Enden).
pub fn clip_secs(it: &Item, s: &Settings) -> f32 {
    let min_len = 2.0 * s.trans_secs + 0.6;
    let d = match it.kind {
        Kind::Intro | Kind::Outro => s.credits.card_secs,
        Kind::Video => {
            if s.video_max_secs > 0.0 {
                it.secs.min(s.video_max_secs)
            } else {
                it.secs
            }
        }
        Kind::Image => {
            if is_pano(it, s) {
                let travel = (pano_width(it, s) as f32 - s.width as f32).max(0.0);
                let speed = (s.pano_speed * s.height as f32 / 1080.0).max(1.0);
                let edge = s.trans_secs + s.pano_pause;
                let d = (travel / speed + 2.0 * edge).clamp(s.pano_min_secs, s.pano_max_secs.max(s.pano_min_secs));
                d.max(s.still_secs)
            } else {
                s.still_secs
            }
        }
    };
    d.max(min_len)
}

#[derive(Clone, Copy, Debug)]
pub struct Slot {
    pub item: usize,
    /// Erster Frame des Clips in der Gesamtzeit.
    pub start: u32,
    pub frames: u32,
    pub pano: bool,
}

#[derive(Clone, Debug)]
pub struct Timeline {
    pub slots: Vec<Slot>,
    pub total: u32,
    /// Frames je Übergang (0 = harter Schnitt).
    pub trans: u32,
}

impl Timeline {
    pub fn total_secs(&self, fps: u32) -> f32 {
        self.total as f32 / fps.max(1) as f32
    }
}

pub fn build_timeline(items: &[Item], s: &Settings) -> Timeline {
    let t = if s.trans_secs > 0.0 { frames_of(s.trans_secs, s.fps) } else { 0 };
    let mut slots = Vec::with_capacity(items.len());
    let mut start = 0u32;
    for (i, it) in items.iter().enumerate() {
        let n = frames_of(clip_secs(it, s), s.fps).max(2 * t + 2);
        slots.push(Slot { item: i, start, frames: n, pano: is_pano(it, s) });
        start += n - t;
    }
    let total = slots.last().map(|l| l.start + l.frames).unwrap_or(0);
    Timeline { slots, total, trans: t }
}

/// Frames am Rand eines Panoramas, in denen das Bild still steht
/// (Übergang + Pause); dazwischen läuft der Schwenk.
pub fn pano_edge_frames(s: &Settings) -> u32 {
    let t = if s.trans_secs > 0.0 { s.trans_secs } else { 0.0 };
    frames_of(t + s.pano_pause, s.fps)
}

const MIX: [Transition; 6] = [
    Transition::Crossfade,
    Transition::Wipe,
    Transition::Crossfade,
    Transition::Push,
    Transition::Crossfade,
    Transition::Zoom,
];

/// Übergangsart zwischen Clip `pair` und `pair + 1`.
/// Bei Panoramen immer sanft überblenden.
pub fn transition_for(pair: usize, s: &Settings, pano_a: bool, pano_b: bool) -> Transition {
    if pano_a || pano_b {
        return Transition::Crossfade;
    }
    match s.transition {
        Transition::Mix => MIX[pair % MIX.len()],
        t => t,
    }
}

/// Ergänzt Vor- und Abspann (falls eingeschaltet) um die Liste.
pub fn with_credits(items: &[Item], s: &Settings) -> Vec<Item> {
    with_cards(items, s, s.credits.intro, s.credits.outro)
}

/// Wie `with_credits`, aber mit frei gewählten Karten (für Vorschau-Ausschnitte).
pub fn with_cards(items: &[Item], s: &Settings, intro: bool, outro: bool) -> Vec<Item> {
    let card = |kind: Kind, name: &str| Item {
        path: PathBuf::new(),
        name: name.into(),
        kind,
        w: s.width,
        h: s.height,
        orient: 1,
        secs: 0.0,
        modified: None,
    };
    let mut v = Vec::with_capacity(items.len() + 2);
    if intro && !items.is_empty() {
        v.push(card(Kind::Intro, "Vorspann"));
    }
    v.extend_from_slice(items);
    if outro && !items.is_empty() {
        v.push(card(Kind::Outro, "Abspann"));
    }
    v
}

/// Kleine Einstellungen für die Vorschau im Programmfenster (schnell zu rechnen).
pub fn preview_settings(s: &Settings) -> Settings {
    let mut p = s.clone();
    // Die längere Seite wird 640 Pixel (Hochformat: 640 hoch).
    let even = |v: f32| ((v / 2.0).round() as u32 * 2).max(2);
    if s.height > s.width {
        p.height = 640;
        p.width = even(640.0 * s.width as f32 / s.height.max(1) as f32);
    } else {
        p.width = 640;
        p.height = even(640.0 * s.height as f32 / s.width.max(1) as f32);
    }
    p.fps = s.fps.clamp(10, 25);
    p.music.clear();
    p
}

// ---------------------------------------------------------------- Liste von Hand ordnen

/// Verschiebt die gewählten Einträge um eine Stelle nach oben/unten (als Block, Reihenfolge bleibt).
/// Gibt die neue Auswahl zurück.
pub fn move_selected<T>(v: &mut Vec<T>, sel: &BTreeSet<usize>, up: bool) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    let n = v.len();
    if up {
        for &i in sel.iter().filter(|&&i| i < n) {
            if i == 0 || out.contains(&(i - 1)) {
                out.insert(i);
            } else {
                v.swap(i - 1, i);
                out.insert(i - 1);
            }
        }
    } else {
        for &i in sel.iter().rev().filter(|&&i| i < n) {
            if i + 1 >= n || out.contains(&(i + 1)) {
                out.insert(i);
            } else {
                v.swap(i, i + 1);
                out.insert(i + 1);
            }
        }
    }
    out
}

/// Nimmt die gewählten Einträge heraus und setzt sie (in ihrer Reihenfolge) so wieder ein,
/// dass der erste an Stelle `at` steht (0 = ganz vorn, grosse Zahl = ans Ende).
pub fn place_at<T>(v: &mut Vec<T>, sel: &BTreeSet<usize>, at: usize) -> BTreeSet<usize> {
    let mut taken: Vec<T> = Vec::new();
    let len = v.len();
    for &i in sel.iter().rev().filter(|&&i| i < len) {
        taken.push(v.remove(i));
    }
    taken.reverse();
    let at = at.min(v.len());
    let k = taken.len();
    for (j, t) in taken.into_iter().enumerate() {
        v.insert(at + j, t);
    }
    (at..at + k).collect()
}

/// Wie `place_at`, aber `before` ist die Einfügemarke in der **alten** Liste
/// (0 = vor dem ersten Eintrag, `len` = hinter dem letzten) – so, wie man beim Ziehen zielt.
pub fn move_before<T>(v: &mut Vec<T>, sel: &BTreeSet<usize>, before: usize) -> BTreeSet<usize> {
    let removed_above = sel.iter().filter(|&&i| i < before).count();
    place_at(v, sel, before.saturating_sub(removed_above))
}

/// Verteilt `groups` gleichmässig zwischen die Einträge von `base` (deren Reihenfolge bleibt).
pub fn spread_groups<T>(base: Vec<T>, groups: Vec<Vec<T>>) -> Vec<T> {
    let n = base.len();
    if n == 0 {
        return groups.into_iter().flatten().collect();
    }
    // (nach welchem Basis-Eintrag, Gruppe, Nummer in der Gruppe, Eintrag)
    let mut events: Vec<(usize, usize, usize, T)> = Vec::new();
    for (g, grp) in groups.into_iter().enumerate() {
        let m = grp.len();
        for (k, it) in grp.into_iter().enumerate() {
            let after = ((k + 1) as f32 * n as f32 / (m + 1) as f32).round() as usize;
            events.push((after.saturating_sub(1).min(n - 1), g, k, it));
        }
    }
    events.sort_by_key(|e| (e.0, e.1, e.2));
    let mut out = Vec::with_capacity(n + events.len());
    let mut ev = events.into_iter().peekable();
    for (i, b) in base.into_iter().enumerate() {
        out.push(b);
        while ev.peek().map(|e| e.0 == i).unwrap_or(false) {
            out.push(ev.next().unwrap().3);
        }
    }
    out
}

/// Streut Videos und/oder Panoramen gleichmässig zwischen die übrigen Bilder.
pub fn spread_items(items: Vec<Item>, s: &Settings, videos: bool, panos: bool) -> Vec<Item> {
    let mut base = Vec::new();
    let (mut vids, mut pans) = (Vec::new(), Vec::new());
    for it in items {
        if videos && it.kind == Kind::Video {
            vids.push(it);
        } else if panos && is_pano(&it, s) {
            pans.push(it);
        } else {
            base.push(it);
        }
    }
    spread_groups(base, vec![vids, pans])
}

// ---------------------------------------------------------------- Vor-/Abspann: Gestaltung

/// Ein Textblock der Karte (mehrere Zeilen mit \n).
#[derive(Clone, Debug)]
pub struct TextBlock {
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub size: i32,
    pub color: &'static str,
    pub spacing: i32,
}

#[derive(Clone, Debug)]
pub struct CardLayout {
    pub blocks: Vec<TextBlock>,
    /// Goldene Linie: (x, y, Breite, Höhe)
    pub rule: Option<(i32, i32, i32, i32)>,
    /// Porträt: (grösste Breite, Höhe, x, y) – x/y als ffmpeg-Ausdrücke für overlay
    /// (W/H = Karte, w/h = Porträt).
    pub portrait: Option<(i32, i32, String, String)>,
}

pub const CARD_BG: &str = "0x0d0d0d";
pub const CARD_GOLD: &str = "0xa8962a";

/// Bricht Text an Wortgrenzen auf höchstens `max` Zeichen je Zeile um.
pub fn wrap_text(text: &str, max: usize) -> Vec<String> {
    let max = max.max(4);
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let wl = word.chars().count();
        let cl = cur.chars().count();
        if cl == 0 {
            cur = word.to_string();
        } else if cl + 1 + wl <= max {
            cur.push(' ');
            cur.push_str(word);
        } else {
            lines.push(std::mem::take(&mut cur));
            cur = word.to_string();
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Aufteilung der Karte wie auf der Webseite des Fotografen: links kleine
/// Titelzeile, grosser Name, goldene Linie, Text; rechts das Porträt.
pub fn card_layout(w: u32, h: u32, has_portrait: bool, kicker: &str, name: &str, text: &str) -> CardLayout {
    if h as f32 > w as f32 * 1.1 {
        return card_layout_tall(w, h, has_portrait, kicker, name, text);
    }
    let (wf, hf) = (w as f32, h as f32);
    let x0 = if has_portrait { 0.05 * wf } else { 0.10 * wf };
    let colw = if has_portrait { 0.42 * wf } else { 0.80 * wf };
    let fs = |f: f32| ((f * hf) as i32).max(10);
    let (fk, fnm, ft) = (fs(0.028), fs(0.095), fs(0.032));
    let kicker_l = wrap_text(&kicker.to_uppercase(), (colw / (fk as f32 * 0.70)) as usize);
    let name_up = name.to_uppercase();
    let name_l: Vec<String> = if has_portrait {
        name_up.split_whitespace().map(|x| x.to_string()).collect()
    } else {
        wrap_text(&name_up, (colw / (fnm as f32 * 0.70)) as usize)
    };
    let text_l = wrap_text(text, (colw / (ft as f32 * 0.52)) as usize);
    let lh = |size: i32, n: usize| (size as f32 * 1.45) * n as f32;
    let (g1, g2, g3) = (0.06 * hf, 0.045 * hf, 0.05 * hf);
    let rule_h = (0.004 * hf).max(3.0);
    let mut total = 0.0;
    let mut parts = 0;
    if !kicker_l.is_empty() {
        total += lh(fk, kicker_l.len());
        parts += 1;
    }
    if !name_l.is_empty() {
        total += if parts > 0 { g1 } else { 0.0 } + lh(fnm, name_l.len());
        parts += 1;
    }
    total += g2 + rule_h;
    if !text_l.is_empty() {
        total += g3 + lh(ft, text_l.len());
    }
    let mut y = ((hf - total) / 2.0).max(0.04 * hf);
    let mut blocks = Vec::new();
    let mut first = true;
    if !kicker_l.is_empty() {
        blocks.push(TextBlock { text: kicker_l.join("\n"), x: x0 as i32, y: y as i32, size: fk, color: "0xe6e6e6", spacing: (fk as f32 * 0.2) as i32 });
        y += lh(fk, kicker_l.len());
        first = false;
    }
    if !name_l.is_empty() {
        if !first {
            y += g1;
        }
        blocks.push(TextBlock { text: name_l.join("\n"), x: x0 as i32, y: y as i32, size: fnm, color: "0xffffff", spacing: (fnm as f32 * 0.2) as i32 });
        y += lh(fnm, name_l.len());
    }
    y += g2;
    let rule = Some((x0 as i32, y as i32, (0.06 * wf) as i32, rule_h as i32));
    y += rule_h;
    if !text_l.is_empty() {
        y += g3;
        blocks.push(TextBlock { text: text_l.join("\n"), x: x0 as i32, y: y as i32, size: ft, color: "0xb4b4b4", spacing: (ft as f32 * 0.2) as i32 });
    }
    let portrait = if has_portrait {
        Some(((0.46 * wf) as i32, (0.84 * hf) as i32, format!("W-w-{}", (0.05 * wf) as i32), "(H-h)/2".to_string()))
    } else {
        None
    };
    CardLayout { blocks, rule, portrait }
}

/// Hochformat (z. B. Instagram 1080×1920 oder 4:5): Porträt oben in der Mitte, Text darunter,
/// alle Grössen nach der Bildbreite.
pub fn card_layout_tall(w: u32, h: u32, has_portrait: bool, kicker: &str, name: &str, text: &str) -> CardLayout {
    let (wf, hf) = (w as f32, h as f32);
    let x0 = 0.08 * wf;
    let colw = 0.84 * wf;
    let fs = |f: f32| ((f * wf) as i32).max(10);
    let (fk, fnm, ft) = (fs(0.034), fs(0.10), fs(0.044));
    let kicker_l = wrap_text(&kicker.to_uppercase(), (colw / (fk as f32 * 0.70)) as usize);
    let name_l = wrap_text(&name.to_uppercase(), (colw / (fnm as f32 * 0.70)) as usize);
    let text_l = wrap_text(text, (colw / (ft as f32 * 0.52)) as usize);
    let lh = |size: i32, n: usize| (size as f32 * 1.45) * n as f32;
    let (g1, g2, g3) = (0.035 * wf, 0.03 * wf, 0.035 * wf);
    let rule_h = (0.004 * wf).max(3.0);
    // Porträt im oberen Teil; der Text bekommt den Rest.
    let (top, portrait) = if has_portrait {
        let ph = 0.42 * hf;
        (0.07 * hf + ph + 0.04 * hf, Some(((0.76 * wf) as i32, ph as i32, "(W-w)/2".to_string(), format!("{}", (0.07 * hf) as i32))))
    } else {
        (0.0, None)
    };
    let mut total = 0.0;
    let mut parts = 0;
    if !kicker_l.is_empty() {
        total += lh(fk, kicker_l.len());
        parts += 1;
    }
    if !name_l.is_empty() {
        total += if parts > 0 { g1 } else { 0.0 } + lh(fnm, name_l.len());
    }
    total += g2 + rule_h;
    if !text_l.is_empty() {
        total += g3 + lh(ft, text_l.len());
    }
    let bottom = 0.95 * hf;
    let mut y = (top + ((bottom - top - total) / 2.0).max(0.0)).max(0.04 * hf);
    let mut blocks = Vec::new();
    let mut first = true;
    if !kicker_l.is_empty() {
        blocks.push(TextBlock { text: kicker_l.join("\n"), x: x0 as i32, y: y as i32, size: fk, color: "0xe6e6e6", spacing: (fk as f32 * 0.2) as i32 });
        y += lh(fk, kicker_l.len());
        first = false;
    }
    if !name_l.is_empty() {
        if !first {
            y += g1;
        }
        blocks.push(TextBlock { text: name_l.join("\n"), x: x0 as i32, y: y as i32, size: fnm, color: "0xffffff", spacing: (fnm as f32 * 0.2) as i32 });
        y += lh(fnm, name_l.len());
    }
    y += g2;
    let rule = Some((x0 as i32, y as i32, (0.12 * wf) as i32, rule_h as i32));
    y += rule_h;
    if !text_l.is_empty() {
        y += g3;
        blocks.push(TextBlock { text: text_l.join("\n"), x: x0 as i32, y: y as i32, size: ft, color: "0xb4b4b4", spacing: (ft as f32 * 0.2) as i32 });
    }
    CardLayout { blocks, rule, portrait }
}

/// Pfad für ffmpeg-Filteroptionen (drawtext): Schrägstriche, Doppelpunkt maskiert.
pub fn esc_filter_path(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/").replace(':', "\\:")
}

/// Sucht eine passende Systemschrift (leicht bevorzugt).
pub fn find_font(custom: Option<&Path>) -> Option<PathBuf> {
    if let Some(c) = custom {
        if c.is_file() {
            return Some(c.to_path_buf());
        }
    }
    let cands = [
        "C:/Windows/Fonts/segoeuil.ttf",
        "C:/Windows/Fonts/segoeui.ttf",
        "C:/Windows/Fonts/calibri.ttf",
        "C:/Windows/Fonts/arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/Library/Fonts/Arial.ttf",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
    ];
    cands.iter().map(PathBuf::from).find(|p| p.is_file())
}

/// Filtergraph für Vor-/Abspann: Hintergrund (Eingang 0), optional Porträt (Eingang 1).
/// `files` sind die Textdateien in Reihenfolge von `layout.blocks`.
pub fn card_graph(layout: &CardLayout, font: &Path, files: &[PathBuf], portrait_orient: u8) -> String {
    let mut chain = String::from("[0:v]");
    for (b, f) in layout.blocks.iter().zip(files) {
        chain.push_str(&format!(
            "drawtext=fontfile='{}':textfile='{}':fontsize={}:fontcolor={}:x={}:y={}:line_spacing={},",
            esc_filter_path(font),
            esc_filter_path(f),
            b.size,
            b.color,
            b.x,
            b.y,
            b.spacing
        ));
    }
    if let Some((x, y, w, h)) = layout.rule {
        chain.push_str(&format!("drawbox=x={x}:y={y}:w={w}:h={h}:color={CARD_GOLD}:t=fill,"));
    }
    match &layout.portrait {
        Some((maxw, ph, px, py)) => {
            let o = orient_filter(portrait_orient);
            let pre = if o.is_empty() { String::new() } else { format!("{o},") };
            format!(
                "[1:v]{pre}scale={maxw}:{ph}:force_original_aspect_ratio=decrease,format=rgb24[p];\
                 {chain}format=rgb24[bg];[bg][p]overlay=x={px}:y={py}:format=rgb,format=rgb24"
            )
        }
        None => format!("{chain}format=rgb24"),
    }
}

// ---------------------------------------------------------------- ffmpeg-Filter

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Look {
    Pano,
    /// Bild füllt den Schirm (Ränder werden beschnitten).
    Cover,
    /// Ganzes Bild vor unscharfem, abgedunkeltem Hintergrund.
    Blur,
}

/// Bis zu dieser Abweichung vom Schirmformat wird beschnitten statt Balken zu zeigen.
const COVER_TOLERANCE: f32 = 0.20;

pub fn look_for(it: &Item, s: &Settings) -> Look {
    if is_pano(it, s) {
        return Look::Pano;
    }
    let a = it.w as f32 / it.h.max(1) as f32;
    let t = s.width as f32 / s.height.max(1) as f32;
    if s.crop_to_fill && (a / t - 1.0).abs() <= COVER_TOLERANCE {
        Look::Cover
    } else {
        Look::Blur
    }
}

/// Grösse der Leinwand bei normalen Bildern (etwas grösser als das Bild, damit der Zoom Platz hat).
pub fn canvas_size(s: &Settings) -> (u32, u32) {
    let z = 1.0 + s.zoom_pct.max(0.0) / 100.0;
    (
        ((s.width as f32) * z).round() as u32,
        ((s.height as f32) * z).round() as u32,
    )
}

/// ffmpeg-Filter, der die EXIF-Ausrichtung herstellt (leer = nichts zu tun).
pub fn orient_filter(o: u8) -> &'static str {
    match o {
        2 => "hflip",
        3 => "hflip,vflip",
        4 => "vflip",
        5 => "transpose=0",
        6 => "transpose=1",
        7 => "transpose=3",
        8 => "transpose=2",
        _ => "",
    }
}

/// Filterkette + Ausgabegrösse (Breite, Höhe) zum Laden eines Bildes als rohes RGB.
pub fn image_filter(it: &Item, s: &Settings) -> (String, u32, u32) {
    let o = orient_filter(it.orient);
    let pre = if o.is_empty() { String::new() } else { format!("{o},") };
    match look_for(it, s) {
        Look::Pano => {
            let pw = pano_width(it, s);
            (
                format!("{pre}scale={pw}:{}:flags=lanczos,format=rgb24", s.height),
                pw,
                s.height,
            )
        }
        Look::Cover => {
            let (cw, ch) = canvas_size(s);
            (
                format!(
                    "{pre}scale={cw}:{ch}:force_original_aspect_ratio=increase:flags=lanczos,crop={cw}:{ch},format=rgb24"
                ),
                cw,
                ch,
            )
        }
        Look::Blur => {
            let (cw, ch) = canvas_size(s);
            let (sw, sh) = ((cw / 8).max(2), (ch / 8).max(2));
            (
                format!(
                    "{pre}format=rgb24,split=2[a][b];\
                     [a]scale={sw}:{sh}:force_original_aspect_ratio=increase,crop={sw}:{sh},gblur=sigma=5,\
                     scale={cw}:{ch},lutrgb=r=val*0.55:g=val*0.55:b=val*0.55[bg];\
                     [b]scale={cw}:{ch}:force_original_aspect_ratio=decrease:flags=lanczos[fg];\
                     [bg][fg]overlay=(W-w)/2:(H-h)/2:format=rgb,format=rgb24"
                ),
                cw,
                ch,
            )
        }
    }
}

/// Filter, der ein Video auf Ausgabegrösse und Bildrate bringt. Quer: schwarze Balken bei
/// anderem Format. Hochformat (Instagram): das Video vor einer unscharfen, abgedunkelten
/// Vergrösserung von sich selbst, wie bei den Fotos.
pub fn video_filter(s: &Settings) -> String {
    if s.height > s.width {
        let (w, h) = (s.width, s.height);
        let (sw, sh) = ((w / 8).max(2), (h / 8).max(2));
        return format!(
            "fps={fps},format=rgb24,split=2[a][b];\
             [a]scale={sw}:{sh}:force_original_aspect_ratio=increase,crop={sw}:{sh},gblur=sigma=5,\
             scale={w}:{h},lutrgb=r=val*0.55:g=val*0.55:b=val*0.55[bg];\
             [b]scale={w}:{h}:force_original_aspect_ratio=decrease:flags=bicubic[fg];\
             [bg][fg]overlay=(W-w)/2:(H-h)/2:format=rgb,format=rgb24",
            fps = s.fps
        );
    }
    format!(
        "scale={w}:{h}:force_original_aspect_ratio=decrease:flags=bicubic,\
         pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=black,fps={fps},format=rgb24",
        w = s.width,
        h = s.height,
        fps = s.fps
    )
}

/// Verschiebt Videos gleichmässig zwischen die Bilder (Reihenfolge der Bilder bleibt).
pub fn spread_videos(items: Vec<Item>) -> Vec<Item> {
    let (vids, base): (Vec<Item>, Vec<Item>) = items.into_iter().partition(|i| i.kind == Kind::Video);
    spread_groups(base, vec![vids])
}

pub fn fmt_hms(secs: f32) -> String {
    let t = secs.max(0.0).round() as u64;
    if t >= 3600 {
        format!("{}:{:02}:{:02}", t / 3600, (t / 60) % 60, t % 60)
    } else {
        format!("{}:{:02}", t / 60, t % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(name: &str, w: u32, h: u32) -> Item {
        Item {
            path: PathBuf::from(name),
            name: name.into(),
            kind: Kind::Image,
            w,
            h,
            orient: 1,
            secs: 0.0,
            modified: None,
        }
    }
    fn vid(name: &str, secs: f32) -> Item {
        Item { kind: Kind::Video, w: 3840, h: 2160, secs, ..img(name, 3840, 2160) }
    }

    #[test]
    fn natuerliche_reihenfolge() {
        let mut v = vec!["IMG_10.jpg", "img_2.jpg", "IMG_1.jpg", "img_02a.jpg"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["IMG_1.jpg", "img_2.jpg", "img_02a.jpg", "IMG_10.jpg"]);
    }

    #[test]
    fn ffmpeg_ausgabe_video() {
        let t = "Input #0, mov,mp4, from 'a.mp4':\n  Duration: 00:01:02.50, start: 0.000000, bitrate: 54197 kb/s\n  \
                 Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(tv, bt709, progressive), 3840x2160 [SAR 1:1 DAR 16:9], 54120 kb/s, 24 fps\n";
        assert_eq!(parse_ffmpeg_info(t), Some(Info { w: 3840, h: 2160, secs: Some(62.5) }));
    }

    #[test]
    fn ffmpeg_ausgabe_gedreht_und_bild() {
        let t = "  Duration: 00:00:10.00, start: 0.0\n  Stream #0:0: Video: h264, yuv420p, 1920x1080, 30 fps\n      \
                 Side data:\n        displaymatrix: rotation of -90.00 degrees\n";
        let i = parse_ffmpeg_info(t).unwrap();
        assert_eq!((i.w, i.h), (1080, 1920));
        let png = "  Duration: N/A, bitrate: N/A\n  Stream #0:0: Video: png, rgba(pc, gbr/unknown/unknown), 4000x1000, 25 fps\n";
        assert_eq!(parse_ffmpeg_info(png), Some(Info { w: 4000, h: 1000, secs: None }));
        assert_eq!(parse_ffmpeg_info("nichts"), None);
    }

    #[test]
    fn exif_ausrichtung() {
        // little endian, ein Eintrag 0x0112 = 6
        let mut v = b"Exif\0\0II".to_vec();
        v.extend_from_slice(&[42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0]);
        assert_eq!(exif_orientation(&v), Some(6));
        // big endian, Wert 3
        let mut b = b"Exif\0\0MM".to_vec();
        b.extend_from_slice(&[0, 42, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 3, 0, 0]);
        assert_eq!(exif_orientation(&b), Some(3));
        assert_eq!(exif_orientation(b"kein exif"), None);
    }

    #[test]
    fn hochformat_erkennen() {
        let mut v = vid("drohne.mp4", 10.0);
        assert!(!is_portrait(&v));
        v.w = 1080;
        v.h = 1920;
        assert!(is_portrait(&v));
        let items = vec![img("a.jpg", 6000, 4000), img("b.jpg", 4000, 6000), img("c.jpg", 3000, 3000), v];
        assert_eq!(portrait_rows(&items), vec![1, 3]);
    }

    #[test]
    fn panorama_und_aussehen() {
        let s = Settings::default();
        let pano = img("p.jpg", 12000, 2000);
        let quer = img("q.jpg", 6000, 4000); // 3:2
        let hoch = img("h.jpg", 3000, 4000);
        let vier3 = img("v.jpg", 4000, 3000);
        assert!(is_pano(&pano, &s) && !is_pano(&quer, &s));
        assert_eq!(look_for(&pano, &s), Look::Pano);
        assert_eq!(look_for(&quer, &s), Look::Cover);
        assert_eq!(look_for(&hoch, &s), Look::Blur);
        assert_eq!(look_for(&vier3, &s), Look::Blur);
        let mut s2 = s.clone();
        s2.crop_to_fill = false;
        assert_eq!(look_for(&quer, &s2), Look::Blur);
    }

    #[test]
    fn panorama_dauert_laenger() {
        let s = Settings::default();
        let normal = clip_secs(&img("a.jpg", 6000, 4000), &s);
        let pano = clip_secs(&img("p.jpg", 12000, 2000), &s);
        let pano_klein = clip_secs(&img("p.jpg", 5000, 2000), &s);
        assert_eq!(normal, 6.0);
        assert!(pano > normal && pano <= s.pano_max_secs);
        assert!(pano_klein >= s.pano_min_secs);
        // 6:1 bei 1080p: 6480 px breit, 4560 px Weg bei 240 px/s = 19 s + 3 s Rand
        assert!((pano - 22.0).abs() < 0.1, "{pano}");
    }

    #[test]
    fn zeitplan_ueberlappt() {
        let mut s = Settings::default();
        s.fps = 30;
        s.still_secs = 5.0;
        s.video_max_secs = 30.0;
        let items = vec![img("a.jpg", 6000, 4000), img("b.jpg", 6000, 4000), vid("v.mp4", 100.0)];
        let t = build_timeline(&items, &s);
        assert_eq!(t.trans, 30);
        assert_eq!(t.slots[0].start, 0);
        assert_eq!(t.slots[0].frames, 150);
        assert_eq!(t.slots[1].start, 120); // 150 - 30 Überlappung
        assert_eq!(t.slots[2].start, 240);
        assert_eq!(t.slots[2].frames, 900); // Video auf 30 s gekürzt
        assert_eq!(t.total, 240 + 900);
        s.trans_secs = 0.0;
        let t0 = build_timeline(&items, &s);
        assert_eq!(t0.slots[1].start, 150);
    }

    #[test]
    fn videos_gleichmaessig() {
        let mut items: Vec<Item> = (0..9).map(|i| img(&format!("i{i}.jpg"), 6000, 4000)).collect();
        items.push(vid("v1.mp4", 10.0));
        items.push(vid("v2.mp4", 10.0));
        let out = spread_videos(items);
        assert_eq!(out.len(), 11);
        let pos: Vec<usize> = out.iter().enumerate().filter(|(_, i)| i.kind == Kind::Video).map(|(p, _)| p).collect();
        assert_eq!(pos.len(), 2);
        assert!(pos[0] > 1 && pos[0] < 5 && pos[1] > 6 && pos[1] < 10, "{pos:?}");
        // Bilder behalten ihre Reihenfolge
        let namen: Vec<&str> = out.iter().filter(|i| i.kind == Kind::Image).map(|i| i.name.as_str()).collect();
        assert_eq!(namen, (0..9).map(|i| format!("i{i}.jpg")).collect::<Vec<_>>());
    }

    #[test]
    fn filter_groessen() {
        let s = Settings::default();
        let (f, w, h) = image_filter(&img("p.jpg", 12000, 2000), &s);
        assert_eq!((w, h), (6480, 1080));
        assert!(f.contains("scale=6480:1080"));
        let (_, cw, ch) = image_filter(&img("q.jpg", 6000, 4000), &s);
        assert_eq!((cw, ch), (2016, 1134)); // 5 % grösser als 1920×1080
        let mut o = img("h.jpg", 3000, 4000);
        o.orient = 6;
        assert!(image_filter(&o, &s).0.starts_with("transpose=1,"));
    }

    #[test]
    fn umbruch() {
        assert_eq!(wrap_text("aa bb cc dd", 5), vec!["aa bb", "cc dd"]);
        assert_eq!(wrap_text("langeswort", 4), vec!["langeswort"]);
        assert!(wrap_text("   ", 10).is_empty());
    }

    #[test]
    fn karte_mit_und_ohne_portraet() {
        let l = card_layout(1920, 1080, true, "Fotograf und Gestalter HF, Kommunikationsdesign", "Samuel Büttler", "Der Mann hinter der Kamera");
        assert_eq!(l.blocks.len(), 3);
        assert_eq!(l.blocks[1].text, "SAMUEL\nBÜTTLER");
        assert!(l.blocks[0].text.contains('\n')); // Titelzeile bricht um
        let (maxw, ph, px, _) = l.portrait.unwrap();
        assert!(maxw > 800 && ph == 907 && px == "W-w-96");
        // Reihenfolge von oben nach unten, alles im Bild
        let ys: Vec<i32> = l.blocks.iter().map(|b| b.y).collect();
        assert!(ys[0] < ys[1] && ys[1] < l.rule.unwrap().1 && l.rule.unwrap().1 < ys[2]);
        assert!(ys.iter().all(|y| *y > 0 && *y < 1080));
        let o = card_layout(1920, 1080, false, "", "Samuel Büttler", "Vielen Dank");
        assert_eq!(o.blocks.len(), 2);
        assert!(o.portrait.is_none());
    }

    #[test]
    fn karte_im_hochformat() {
        let l = card_layout(1080, 1920, true, "Fotograf und Gestalter HF, Kommunikationsdesign", "Samuel Büttler", "Der Mann hinter der Kamera");
        let (maxw, ph, px, py) = l.portrait.clone().unwrap();
        assert_eq!((px.as_str(), py.as_str()), ("(W-w)/2", "134"));
        assert!(maxw <= 1080 && ph < 1920 / 2);
        // Text unterhalb des Porträts, alles im Bild und innerhalb der Breite
        let ys: Vec<i32> = l.blocks.iter().map(|b| b.y).collect();
        assert!(ys[0] > 134 + ph && ys.iter().all(|y| *y < 1920));
        assert!(ys.windows(2).all(|w| w[0] < w[1]));
        for b in &l.blocks {
            for line in b.text.lines() {
                assert!((line.chars().count() as f32) * b.size as f32 * 0.55 < 1080.0 * 0.92, "zu breit: {line}");
            }
        }
        let o = card_layout(1080, 1350, false, "", "Samuel Büttler", "Vielen Dank");
        assert!(o.portrait.is_none() && o.blocks.len() == 2);
    }

    #[test]
    fn video_im_hochformat_ohne_balken() {
        let quer = video_filter(&Settings::default());
        assert!(quer.contains("pad=1920:1080"));
        let hoch = video_filter(&Settings { width: 1080, height: 1920, ..Settings::default() });
        assert!(hoch.contains("gblur") && hoch.contains("overlay") && !hoch.contains("pad="));
    }

    #[test]
    fn pfade_fuer_filter() {
        assert_eq!(esc_filter_path(Path::new("C:\\Windows\\Fonts\\a.ttf")), "C\\:/Windows/Fonts/a.ttf");
    }

    #[test]
    fn vor_und_abspann_in_der_zeit() {
        let s = Settings::default();
        let items = vec![img("a.jpg", 6000, 4000)];
        let v = with_credits(&items, &s);
        assert_eq!(v.len(), 3);
        assert_eq!((v[0].kind, v[2].kind), (Kind::Intro, Kind::Outro));
        assert_eq!(clip_secs(&v[0], &s), 8.0);
        assert!(with_credits(&[], &s).is_empty());
    }

    #[test]
    fn dauer_aus_ausgabe() {
        assert_eq!(parse_duration("  Duration: 00:03:21.50, start: 0.0, bitrate: 320 kb/s"), Some(201.5));
        assert_eq!(parse_duration("kein Text"), None);
    }

    #[test]
    fn formate() {
        let a = OutFmt::Webm.codec_args(Quality::Good, true);
        assert!(a.contains(&"libvpx-vp9".to_string()) && a.contains(&"libopus".to_string()));
        let m = OutFmt::Mkv.codec_args(Quality::Good, false);
        assert!(!m.contains(&"-movflags".to_string()) && !m.contains(&"-c:a".to_string()));
        assert!(OutFmt::Mp4.codec_args(Quality::Draft, true).contains(&"-movflags".to_string()));
    }

    fn set(v: &[usize]) -> BTreeSet<usize> {
        v.iter().copied().collect()
    }

    #[test]
    fn block_verschieben() {
        let mut v = vec!['a', 'b', 'c', 'd', 'e'];
        let sel = move_selected(&mut v, &set(&[1, 2]), true);
        assert_eq!(v, vec!['b', 'c', 'a', 'd', 'e']);
        assert_eq!(sel, set(&[0, 1]));
        let sel = move_selected(&mut v, &sel, true); // schon oben: bleibt
        assert_eq!((v.clone(), sel), (vec!['b', 'c', 'a', 'd', 'e'], set(&[0, 1])));
        let sel = move_selected(&mut v, &set(&[3, 4]), false); // schon unten
        assert_eq!((v.clone(), sel), (vec!['b', 'c', 'a', 'd', 'e'], set(&[3, 4])));
        let sel = move_selected(&mut v, &set(&[0, 2]), false);
        assert_eq!(v, vec!['c', 'b', 'd', 'a', 'e']);
        assert_eq!(sel, set(&[1, 3]));
    }

    #[test]
    fn an_stelle_setzen() {
        let mut v: Vec<usize> = (0..8).collect();
        let sel = place_at(&mut v, &set(&[1, 5]), 0);
        assert_eq!(v, vec![1, 5, 0, 2, 3, 4, 6, 7]);
        assert_eq!(sel, set(&[0, 1]));
        let sel = place_at(&mut v, &set(&[0, 1]), 100); // ans Ende
        assert_eq!(v, vec![0, 2, 3, 4, 6, 7, 1, 5]);
        assert_eq!(sel, set(&[6, 7]));
    }

    #[test]
    fn ziehen_vor_marke() {
        let mut v: Vec<usize> = (0..6).collect();
        // Eintrag 1 vor Marke 4 (= zwischen 3 und 4) ziehen
        let sel = move_before(&mut v, &set(&[1]), 4);
        assert_eq!(v, vec![0, 2, 3, 1, 4, 5]);
        assert_eq!(sel, set(&[3]));
        // Eintrag 4 vor Marke 1
        let mut w: Vec<usize> = (0..6).collect();
        move_before(&mut w, &set(&[4]), 1);
        assert_eq!(w, vec![0, 4, 1, 2, 3, 5]);
        // hinter den letzten
        let mut x: Vec<usize> = (0..4).collect();
        move_before(&mut x, &set(&[0]), 4);
        assert_eq!(x, vec![1, 2, 3, 0]);
    }

    #[test]
    fn panoramen_und_videos_streuen() {
        let s = Settings::default();
        let mut items: Vec<Item> = (0..12).map(|i| img(&format!("i{i:02}.jpg"), 6000, 4000)).collect();
        items.extend((0..3).map(|i| img(&format!("p{i}.jpg"), 12000, 2000)));
        items.push(vid("v0.mp4", 10.0));
        items.push(vid("v1.mp4", 10.0));
        let out = spread_items(items.clone(), &s, true, true);
        assert_eq!(out.len(), 17);
        let pos = |pred: &dyn Fn(&Item) -> bool| -> Vec<usize> {
            out.iter().enumerate().filter(|(_, i)| pred(i)).map(|(p, _)| p).collect()
        };
        let pp = pos(&|i| is_pano(i, &s));
        let vp = pos(&|i| i.kind == Kind::Video);
        assert_eq!(pp.len(), 3);
        assert_eq!(vp.len(), 2);
        // nie zwei Panoramen/Videos direkt hintereinander, nicht ganz vorn/hinten
        assert!(pp.windows(2).all(|w| w[1] - w[0] >= 3), "{pp:?}");
        assert!(vp[1] - vp[0] >= 4, "{vp:?}");
        assert!(pp[0] > 0 && *pp.last().unwrap() < 16);
        // normale Bilder behalten Reihenfolge
        let norm: Vec<&str> = out.iter().filter(|i| !is_pano(i, &s) && i.kind == Kind::Image).map(|i| i.name.as_str()).collect();
        let expect: Vec<String> = (0..12).map(|i| format!("i{i:02}.jpg")).collect();
        assert_eq!(norm, expect.iter().map(|x| x.as_str()).collect::<Vec<_>>());
        // nur Panoramen streuen: Videos bleiben wo sie waren (am Ende)
        let o2 = spread_items(items, &s, false, true);
        assert_eq!(o2[o2.len() - 1].kind, Kind::Video);
        assert_eq!(o2.iter().filter(|i| is_pano(i, &s)).count(), 3);
    }

    #[test]
    fn vorschau_einstellungen() {
        let s = Settings::default();
        let p = preview_settings(&s);
        assert_eq!((p.width, p.height), (640, 360));
        let hoch = Settings { width: 1080, height: 1920, ..Settings::default() };
        let ph = preview_settings(&hoch);
        assert_eq!((ph.width, ph.height), (360, 640));
        assert!(p.fps <= 25 && p.music.is_empty());
        let items = vec![img("a.jpg", 6000, 4000)];
        assert_eq!(with_cards(&items, &p, false, false).len(), 1);
        assert_eq!(with_cards(&items, &p, true, true).len(), 3);
    }
}
