//! Reine Datenlogik ohne Oberfläche und ohne Plattform-APIs: Medientyp-Erkennung,
//! Zielformate, Seitenverhältnis-Geometrie (Zuschneiden/Ränder/Strecken),
//! Qualitätsstufen, Zieldateiname und der Bau der ffmpeg-Argumente.
//! Nur Standardbibliothek – darum gut testbar.

// ---------------- Medientyp ----------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MediaKind {
    Audio,
    Image,
    Video,
}

/// Medientyp anhand der Dateiendung (ohne Punkt, Gross/Klein egal).
pub fn detect_kind(ext: &str) -> Option<MediaKind> {
    let e = ext.to_lowercase();
    const IMG: &[&str] = &["jpg", "jpeg", "png", "bmp", "gif", "tif", "tiff", "webp"];
    const AUD: &[&str] = &["mp3", "wav", "flac", "ogg", "oga", "m4a", "aac", "opus", "wma"];
    const VID: &[&str] = &["mp4", "mov", "mkv", "webm", "avi", "wmv", "m4v", "flv", "ts", "mpg", "mpeg"];
    if IMG.contains(&e.as_str()) {
        Some(MediaKind::Image)
    } else if AUD.contains(&e.as_str()) {
        Some(MediaKind::Audio)
    } else if VID.contains(&e.as_str()) {
        Some(MediaKind::Video)
    } else {
        None
    }
}

// ---------------- Zielformate ----------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AudioFmt { Mp3, M4a, Wav, Flac, Ogg, Opus }
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImageFmt { Jpg, Png, Webp, Bmp, Gif, Tiff }
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VideoFmt { Mp4, Mkv, Webm, Mov, Gif }

impl AudioFmt {
    pub fn ext(self) -> &'static str {
        match self { AudioFmt::Mp3=>"mp3", AudioFmt::M4a=>"m4a", AudioFmt::Wav=>"wav", AudioFmt::Flac=>"flac", AudioFmt::Ogg=>"ogg", AudioFmt::Opus=>"opus" }
    }
    pub fn label(self) -> &'static str {
        match self { AudioFmt::Mp3=>"MP3", AudioFmt::M4a=>"M4A", AudioFmt::Wav=>"WAV", AudioFmt::Flac=>"FLAC", AudioFmt::Ogg=>"OGG", AudioFmt::Opus=>"OPUS" }
    }
    pub fn all() -> [AudioFmt; 6] { [AudioFmt::Mp3, AudioFmt::M4a, AudioFmt::Wav, AudioFmt::Flac, AudioFmt::Ogg, AudioFmt::Opus] }
}
impl ImageFmt {
    pub fn ext(self) -> &'static str {
        match self { ImageFmt::Jpg=>"jpg", ImageFmt::Png=>"png", ImageFmt::Webp=>"webp", ImageFmt::Bmp=>"bmp", ImageFmt::Gif=>"gif", ImageFmt::Tiff=>"tiff" }
    }
    pub fn label(self) -> &'static str {
        match self { ImageFmt::Jpg=>"JPG", ImageFmt::Png=>"PNG", ImageFmt::Webp=>"WebP", ImageFmt::Bmp=>"BMP", ImageFmt::Gif=>"GIF", ImageFmt::Tiff=>"TIFF" }
    }
    pub fn all() -> [ImageFmt; 6] { [ImageFmt::Jpg, ImageFmt::Png, ImageFmt::Webp, ImageFmt::Bmp, ImageFmt::Gif, ImageFmt::Tiff] }
}
impl VideoFmt {
    pub fn ext(self) -> &'static str {
        match self { VideoFmt::Mp4=>"mp4", VideoFmt::Mkv=>"mkv", VideoFmt::Webm=>"webm", VideoFmt::Mov=>"mov", VideoFmt::Gif=>"gif" }
    }
    pub fn label(self) -> &'static str {
        match self { VideoFmt::Mp4=>"MP4", VideoFmt::Mkv=>"MKV", VideoFmt::Webm=>"WebM", VideoFmt::Mov=>"MOV", VideoFmt::Gif=>"GIF" }
    }
    pub fn all() -> [VideoFmt; 5] { [VideoFmt::Mp4, VideoFmt::Mkv, VideoFmt::Webm, VideoFmt::Mov, VideoFmt::Gif] }
}

/// Gewähltes Zielformat je nach Medientyp.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    Audio(AudioFmt),
    Image(ImageFmt),
    Video(VideoFmt),
}

impl Target {
    pub fn ext(self) -> &'static str {
        match self { Target::Audio(f)=>f.ext(), Target::Image(f)=>f.ext(), Target::Video(f)=>f.ext() }
    }
    pub fn label(self) -> &'static str {
        match self { Target::Audio(f)=>f.label(), Target::Image(f)=>f.label(), Target::Video(f)=>f.label() }
    }
    pub fn kind(self) -> MediaKind {
        match self { Target::Audio(_)=>MediaKind::Audio, Target::Image(_)=>MediaKind::Image, Target::Video(_)=>MediaKind::Video }
    }
}

/// Sinnvolles Standard-Zielformat je Quelltyp.
pub fn default_target(kind: MediaKind) -> Target {
    match kind {
        MediaKind::Audio => Target::Audio(AudioFmt::Mp3),
        MediaKind::Image => Target::Image(ImageFmt::Jpg),
        MediaKind::Video => Target::Video(VideoFmt::Mp4),
    }
}

/// Gilt ein beim Zielformat gewähltes `tgt` für eine Datei der Art `kind`? Gleichartige Dateien,
/// und ein Ton-Ziel auch für Videos («Nur Ton»). Gleiche Regel wie «Für alle übernehmen».
pub fn ziel_passt(kind: MediaKind, tgt: Target) -> bool {
    kind == tgt.kind() || (tgt.kind() == MediaKind::Audio && kind == MediaKind::Video)
}

/// Endung vereinheitlicht (Gross/Klein egal, «jpeg» = «jpg», «tif» = «tiff»).
fn norm_ext(ext: &str) -> String {
    let e = ext.to_lowercase();
    match e.as_str() {
        "jpeg" => "jpg".into(),
        "tif" => "tiff".into(),
        _ => e,
    }
}

/// true, wenn die Datei schon im Zielformat ist (gleiche Endung, Gross/Klein egal). Solche
/// Dateien werden nicht umgewandelt und nie gelöscht.
pub fn schon_im_ziel(input: &std::path::Path, target: Target) -> bool {
    match input.extension() {
        Some(e) => norm_ext(&e.to_string_lossy()) == norm_ext(target.ext()),
        None => false,
    }
}

// ---------------- Ablage und Originale ----------------

/// Wohin die umgewandelte Datei kommt.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Ablage {
    /// In den gewählten Ausgabeordner (wie bisher).
    #[default]
    Ausgabeordner,
    /// In denselben Ordner wie die Quelldatei.
    NebenOriginal,
}

/// Zielordner einer Datei je nach Ablage.
pub fn ziel_ordner(input: &std::path::Path, ablage: Ablage, ausgabe: &std::path::Path) -> std::path::PathBuf {
    match ablage {
        Ablage::Ausgabeordner => ausgabe.to_path_buf(),
        Ablage::NebenOriginal => match input.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => std::path::PathBuf::from("."),
        },
    }
}

/// Darf das Original nach dem Umwandeln in den Papierkorb? Nur, wenn der Auftrag gelungen ist,
/// die Ausgabe existiert und nicht leer ist (`out_size`) und Ein- und Ausgabe nicht dieselbe
/// Datei sind (`gleiche_datei`: nach dem Auflösen der Pfade; im Zweifel true).
pub fn darf_original_loeschen(
    ok: bool,
    input: &std::path::Path,
    output: &std::path::Path,
    out_size: Option<u64>,
    gleiche_datei: bool,
) -> bool {
    ok && out_size.is_some_and(|n| n > 0)
        && !gleiche_datei
        && input != output
        && !output.as_os_str().is_empty()
}

/// Wie viele Umwandlungen gleichzeitig laufen: halb so viele wie Prozessorkerne, 1 bis 4.
pub fn gleichzeitig(kerne: usize) -> usize {
    (kerne / 2).clamp(1, 4)
}

// ---------------- Ordner durchsuchen ----------------

/// Höchstens so tief wird in Unterordner geschaut.
pub const MAX_TIEFE: usize = 64;
/// Höchstens so viele Dateien nimmt eine Suche auf.
pub const MAX_DATEIEN: usize = 200_000;

/// Medientyp einer Datei anhand ihrer Endung.
pub fn kind_of_path(p: &std::path::Path) -> Option<MediaKind> {
    detect_kind(&p.extension()?.to_string_lossy())
}

/// Ergebnis einer Ordnersuche.
#[derive(Default, Debug)]
pub struct Fund {
    /// Unterstützte Dateien in fester Reihenfolge (Ordner für Ordner, nach Namen).
    pub dateien: Vec<(std::path::PathBuf, MediaKind)>,
    /// Ordner, die sich nicht lesen liessen (still übersprungen).
    pub ordner_uebersprungen: usize,
    /// true, wenn bei `max_dateien` abgebrochen wurde oder Ordner zu tief lagen.
    pub abgeschnitten: bool,
}

impl Fund {
    /// Anzahl Ton-, Bild- und Videodateien.
    #[allow(dead_code)]
    pub fn zaehlen(&self) -> [usize; 3] {
        let mut n = [0usize; 3];
        for (_, k) in &self.dateien {
            n[kind_index(*k)] += 1;
        }
        n
    }
}

/// 0 = Ton, 1 = Bild, 2 = Video (für kleine Felder je Art).
pub fn kind_index(k: MediaKind) -> usize {
    match k {
        MediaKind::Audio => 0,
        MediaKind::Image => 1,
        MediaKind::Video => 2,
    }
}

/// Sucht in `wurzeln` (Ordner oder einzelne Dateien) alle unterstützten Mediendateien, auch in
/// allen Unterordnern. Verknüpfte Ordner (Symlinks, Junctions) werden nicht betreten, nicht
/// lesbare Ordner still übersprungen; tiefer als `max_tiefe` wird nicht gesucht.
pub fn sammeln(wurzeln: &[std::path::PathBuf], max_tiefe: usize, max_dateien: usize) -> Fund {
    use std::path::PathBuf;
    let mut fund = Fund::default();
    let mut gesehen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut nimm = |fund: &mut Fund, p: PathBuf| -> bool {
        if fund.dateien.len() >= max_dateien {
            fund.abgeschnitten = true;
            return false;
        }
        if let Some(k) = kind_of_path(&p) {
            if gesehen.insert(p.clone()) {
                fund.dateien.push((p, k));
            }
        }
        true
    };
    for w in wurzeln {
        let meta = match std::fs::metadata(w) {
            Ok(m) => m,
            Err(_) => {
                fund.ordner_uebersprungen += 1;
                continue;
            }
        };
        if meta.is_file() {
            if !nimm(&mut fund, w.clone()) {
                return fund;
            }
            continue;
        }
        if !meta.is_dir() {
            continue;
        }
        // Tiefensuche ohne Rekursion: (Ordner, Tiefe)
        let mut stapel: Vec<(PathBuf, usize)> = vec![(w.clone(), 0)];
        while let Some((dir, tiefe)) = stapel.pop() {
            let rd = match std::fs::read_dir(&dir) {
                Ok(rd) => rd,
                Err(_) => {
                    fund.ordner_uebersprungen += 1;
                    continue;
                }
            };
            let mut dateien: Vec<PathBuf> = Vec::new();
            let mut ordner: Vec<PathBuf> = Vec::new();
            for e in rd.flatten() {
                let p = e.path();
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_symlink() {
                    // Verknüpfte Datei ja, verknüpfter Ordner nie (keine Schleifen).
                    if std::fs::metadata(&p).is_ok_and(|m| m.is_file()) {
                        dateien.push(p);
                    }
                } else if ft.is_dir() {
                    ordner.push(p);
                } else if ft.is_file() {
                    dateien.push(p);
                }
            }
            dateien.sort();
            ordner.sort();
            for p in dateien {
                if !nimm(&mut fund, p) {
                    return fund;
                }
            }
            if tiefe >= max_tiefe {
                if !ordner.is_empty() {
                    fund.abgeschnitten = true;
                }
                continue;
            }
            // Umgekehrt auf den Stapel, damit «a» vor «b» drankommt.
            for o in ordner.into_iter().rev() {
                stapel.push((o, tiefe + 1));
            }
        }
    }
    fund
}

/// Welche Dateien einer Ordnersuche aufgenommen werden.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Auswahl {
    Ton,
    Bilder,
    Videos,
    Alles,
}

impl Auswahl {
    pub fn passt(self, k: MediaKind) -> bool {
        match self {
            Auswahl::Ton => k == MediaKind::Audio,
            Auswahl::Bilder => k == MediaKind::Image,
            Auswahl::Videos => k == MediaKind::Video,
            Auswahl::Alles => true,
        }
    }
    /// Vorschlag aus den Anzahlen [Ton, Bild, Video]: die Art, die mehr als die Hälfte ausmacht,
    /// sonst alles.
    pub fn vorschlag(n: [usize; 3]) -> Auswahl {
        let total: usize = n.iter().sum();
        let arten = [Auswahl::Ton, Auswahl::Bilder, Auswahl::Videos];
        for (i, a) in arten.iter().enumerate() {
            if n[i] * 2 > total {
                return *a;
            }
        }
        Auswahl::Alles
    }
}

// ---------------- Seitenverhältnis ----------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Aspect { Original, R1_1, R16_9, R9_16, R4_3, R3_2, R21_9 }

impl Aspect {
    pub fn ratio(self) -> Option<f64> {
        match self {
            Aspect::Original => None,
            Aspect::R1_1 => Some(1.0),
            Aspect::R16_9 => Some(16.0 / 9.0),
            Aspect::R9_16 => Some(9.0 / 16.0),
            Aspect::R4_3 => Some(4.0 / 3.0),
            Aspect::R3_2 => Some(3.0 / 2.0),
            Aspect::R21_9 => Some(21.0 / 9.0),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Aspect::Original=>"Original", Aspect::R1_1=>"1:1", Aspect::R16_9=>"16:9",
            Aspect::R9_16=>"9:16", Aspect::R4_3=>"4:3", Aspect::R3_2=>"3:2", Aspect::R21_9=>"21:9",
        }
    }
    pub fn all() -> [Aspect; 7] {
        [Aspect::Original, Aspect::R1_1, Aspect::R16_9, Aspect::R9_16, Aspect::R4_3, Aspect::R3_2, Aspect::R21_9]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fit { Crop, Pad, Stretch }

impl Fit {
    pub fn label(self) -> &'static str {
        match self { Fit::Crop=>"Zuschneiden", Fit::Pad=>"Ränder", Fit::Stretch=>"Strecken" }
    }
}

/// Geometrie-Anweisung für die Umwandlung.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Plan {
    Keep,
    Crop { w: u32, h: u32, x: u32, y: u32 },
    Pad { cw: u32, ch: u32, x: u32, y: u32 },
    Stretch { w: u32, h: u32 },
}

fn even(n: i64) -> u32 {
    (n.max(2) & !1) as u32
}
fn round_u(v: f64) -> i64 {
    v.round() as i64
}

/// Berechnet aus Quellgrösse, Ziel-Seitenverhältnis und Anpassung die Geometrie.
/// `even_dims` erzwingt gerade Kantenlängen (für Video nötig).
pub fn plan_box(sw: u32, sh: u32, aspect: Aspect, fit: Fit, even_dims: bool) -> Plan {
    let r = match aspect.ratio() {
        Some(r) => r,
        None => return Plan::Keep,
    };
    let (sw, sh) = (sw as f64, sh as f64);
    let src_r = sw / sh;
    let fix = |n: i64| if even_dims { even(n) } else { n.max(1) as u32 };

    match fit {
        Fit::Crop => {
            let (cw, ch) = if src_r > r {
                (round_u(sh * r), round_u(sh)) // zu breit -> Breite beschneiden
            } else {
                (round_u(sw), round_u(sw / r)) // zu hoch -> Höhe beschneiden
            };
            let cw = fix(cw).min(sw as u32);
            let ch = fix(ch).min(sh as u32);
            let x = ((sw as u32).saturating_sub(cw)) / 2;
            let y = ((sh as u32).saturating_sub(ch)) / 2;
            Plan::Crop { w: cw, h: ch, x, y }
        }
        Fit::Pad => {
            let (cw, ch) = if src_r < r {
                (round_u(sh * r), round_u(sh)) // zu schmal -> Breite auffüllen
            } else {
                (round_u(sw), round_u(sw / r)) // zu breit -> Höhe auffüllen
            };
            let cw = fix(cw).max(sw as u32);
            let ch = fix(ch).max(sh as u32);
            let x = (cw.saturating_sub(sw as u32)) / 2;
            let y = (ch.saturating_sub(sh as u32)) / 2;
            Plan::Pad { cw, ch, x, y }
        }
        Fit::Stretch => {
            // Leinwand wie bei Ränder, aber Quelle wird darauf verzerrt.
            let (cw, ch) = if src_r < r {
                (round_u(sh * r), round_u(sh))
            } else {
                (round_u(sw), round_u(sw / r))
            };
            Plan::Stretch { w: fix(cw), h: fix(ch) }
        }
    }
}

/// Video-Filter direkt als ffmpeg-Ausdruck (ohne Quellauflösung zu kennen),
/// zentriert und mit geraden Kanten (trunc(.../2)*2). None = Original.
pub fn video_vf(aspect: Aspect, fit: Fit, pad_color: &str) -> Option<String> {
    let r = aspect.ratio()?;
    let rs = format!("{:.6}", r);
    Some(match fit {
        Fit::Crop => format!(
            "crop=w='trunc(min(iw\\,ih*{rs})/2)*2':h='trunc(min(ih\\,iw/{rs})/2)*2'"
        ),
        Fit::Pad => format!(
            "pad=w='trunc(max(iw\\,ih*{rs})/2)*2':h='trunc(max(ih\\,iw/{rs})/2)*2':x='(ow-iw)/2':y='(oh-ih)/2':color={pad_color}"
        ),
        Fit::Stretch => format!(
            "scale=w='trunc(max(iw\\,ih*{rs})/2)*2':h='trunc(max(ih\\,iw/{rs})/2)*2'"
        ),
    })
}

/// ffmpeg-Videofilter aus fertiger Pixel-Geometrie (für die Vorschau/Bilder).
pub fn plan_vf(p: &Plan, pad_color: &str) -> Option<String> {
    match *p {
        Plan::Keep => None,
        Plan::Crop { w, h, x, y } => Some(format!("crop={w}:{h}:{x}:{y}")),
        Plan::Pad { cw, ch, x, y } => Some(format!("pad={cw}:{ch}:{x}:{y}:{pad_color}")),
        Plan::Stretch { w, h } => Some(format!("scale={w}:{h}")),
    }
}

// ---------------- Qualität ----------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Quality { Hoch, Mittel, Klein }

impl Quality {
    pub fn label(self) -> &'static str {
        match self { Quality::Hoch=>"Hoch", Quality::Mittel=>"Mittel", Quality::Klein=>"Klein" }
    }
    pub fn audio_kbps(self) -> u32 {
        match self { Quality::Hoch=>256, Quality::Mittel=>192, Quality::Klein=>128 }
    }
    pub fn video_crf(self) -> u32 {
        match self { Quality::Hoch=>18, Quality::Mittel=>23, Quality::Klein=>28 }
    }
    /// Vorbis-Qualitätsstufe (VBR) für OGG: etwa 256 / 192 / 128 kbit/s bei Stereo. Eine feste
    /// Bitrate lehnt libvorbis bei Mono oder tiefer Abtastrate ab («encoder setup failed»).
    pub fn vorbis_q(self) -> u32 {
        match self { Quality::Hoch=>8, Quality::Mittel=>6, Quality::Klein=>4 }
    }
    /// Qualität 1..=100 für JPEG/WebP.
    pub fn image_q(self) -> u8 {
        match self { Quality::Hoch=>92, Quality::Mittel=>82, Quality::Klein=>70 }
    }
}

// ---------------- Dateiname ----------------

/// Baut den Zieldateinamen (Stamm + neue Endung).
pub fn out_name(stem: &str, ext: &str) -> String {
    format!("{stem}.{ext}")
}

/// Fügt " (n)" vor der Endung ein, um Kollisionen zu umgehen.
pub fn name_with_index(stem: &str, ext: &str, n: u32) -> String {
    if n <= 1 {
        format!("{stem}.{ext}")
    } else {
        format!("{stem} ({n}).{ext}")
    }
}

// ---------------- ffmpeg-Argumente ----------------

/// Lautstärke für Ton aus Filmen: +6 dB (doppelte Amplitude) mit Begrenzer knapp unter 0 dBFS.
pub const FILM_LAUTER: &str = "volume=6dB,alimiter=limit=0.97:level=0";

fn is_video_path(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .and_then(detect_kind)
        == Some(MediaKind::Video)
}

// ---------------- Sprache verbessern ----------------

/// Stufe der Rauschunterdrückung (RNNoise über ffmpeg «arnndn»).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Rauschen {
    #[default]
    Aus,
    Leicht,
    Mittel,
    Stark,
}

impl Rauschen {
    pub fn all() -> [Rauschen; 4] {
        [Rauschen::Aus, Rauschen::Leicht, Rauschen::Mittel, Rauschen::Stark]
    }
    pub fn label(self) -> &'static str {
        match self { Rauschen::Aus=>"Aus", Rauschen::Leicht=>"Leicht", Rauschen::Mittel=>"Mittel", Rauschen::Stark=>"Stark" }
    }
    /// Anteil des entrauschten Signals (1 = ganz). Im Test: Rauschen −6 / −13 / −34 dB.
    fn mix(self) -> Option<&'static str> {
        match self { Rauschen::Aus=>None, Rauschen::Leicht=>Some("0.5"), Rauschen::Mittel=>Some("0.8"), Rauschen::Stark=>Some("1") }
    }
}

/// «Sprache verbessern» für Aufnahmen mit Stimmen (Gespräche, Interviews, Sprachnachrichten).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Sprache {
    /// Leise und laute Stimmen auf dieselbe Lautstärke bringen (ffmpeg «speechnorm»).
    pub angleichen: bool,
    /// Brummen und Rumpeln unter 80 Hz entfernen (Netzbrummen, Lüftung, Trittschall).
    pub brummen: bool,
    pub rauschen: Rauschen,
    /// «Zwei Sprecher angleichen (Klang)»: die dumpfere von zwei Stimmen im Klang angleichen
    /// (nur bei Ton als Ziel, siehe sprecher.rs). Schliesst «angleichen» mit ein.
    pub sprecher: bool,
}

impl Sprache {
    pub fn aktiv(&self) -> bool {
        self.angleichen || self.sprecher || self.brummen || self.rauschen != Rauschen::Aus
    }
    /// Lautstärke angleichen – auch, wenn «Zwei Sprecher angleichen» an ist.
    pub fn angleichen_an(&self) -> bool {
        self.angleichen || self.sprecher
    }
}

/// Dateiname des Stimm-Modells (RNNoise «beguiling-drafter», frei von Urheberrecht laut
/// github.com/GregorR/rnnoise-models). ffmpeg läuft dafür im Ordner des Modells, so braucht der
/// Filter keinen Pfad mit Laufwerksbuchstaben (Doppelpunkte müssten sonst maskiert werden).
pub const STIMM_MODELL: &str = "werk-stimme.rnnn";

/// Tonfilter für ffmpeg «-af»: Sprache verbessern, sonst bei Ton aus Filmen +6 dB.
pub fn ton_filter(input: &str, s: Sprache) -> Option<String> {
    if !s.aktiv() {
        return is_video_path(input).then(|| FILM_LAUTER.to_string());
    }
    let mut f: Vec<String> = Vec::new();
    if s.brummen {
        // Dreimal 12 dB/Oktave: 50-Hz-Brummen fällt deutlich, tiefe Stimmen bleiben.
        f.push("highpass=f=80,highpass=f=80,highpass=f=80".into());
    }
    if let Some(mix) = s.rauschen.mix() {
        f.push(format!("arnndn=m={STIMM_MODELL}:mix={mix}"));
    }
    if s.angleichen_an() {
        // Hebt leise Passagen an, dämpft laute; im Test von 12 dB auf 2 dB Unterschied.
        f.push("speechnorm=e=12.5:r=0.0001:l=1".into());
    } else if is_video_path(input) {
        f.push("volume=6dB".into());
    }
    f.push("alimiter=limit=0.97:level=0".into());
    Some(f.join(","))
}

/// Filterkette für «Zwei Sprecher angleichen»: die Kette von «Sprache verbessern» (mit
/// speechnorm) ohne Begrenzer, dahinter der Klangausgleich für Stimme B (alle Werte 0 = Stimme A).
/// Der Begrenzer folgt erst nach dem Mischen beider Fassungen.
pub fn sprecher_filter(s: Sprache, tief_db: f32, praesenz_db: f32, hoehen_db: f32) -> String {
    let mut f: Vec<String> = Vec::new();
    if s.brummen {
        f.push("highpass=f=80,highpass=f=80,highpass=f=80".into());
    }
    if let Some(mix) = s.rauschen.mix() {
        f.push(format!("arnndn=m={STIMM_MODELL}:mix={mix}"));
    }
    f.push("speechnorm=e=12.5:r=0.0001:l=1".into());
    if tief_db < -0.05 {
        f.push(format!("equalizer=f=220:t=q:w=1:g={tief_db:.1}"));
    }
    if praesenz_db > 0.05 {
        f.push(format!("equalizer=f=2500:t=q:w=1:g={praesenz_db:.1}"));
    }
    if hoehen_db > 0.05 {
        f.push(format!("highshelf=f=4500:g={hoehen_db:.1}"));
    }
    f.join(",")
}

/// Begrenzer nach dem Mischen der beiden Fassungen (etwas Luft für den Klangausgleich).
pub const SPRECHER_BEGRENZER: &str = "alimiter=limit=0.89:level=0";

/// Encoder-Argumente für ein Ton-Zielformat.
pub fn audio_codec_args(f: AudioFmt, q: Quality) -> Vec<String> {
    let mut a = Vec::new();
    match f {
        AudioFmt::Mp3 => push(&mut a, &["-c:a", "libmp3lame", "-b:a", &kbps(q)]),
        AudioFmt::M4a => push(&mut a, &["-c:a", "aac", "-b:a", &kbps(q)]),
        AudioFmt::Ogg => push(&mut a, &["-c:a", "libvorbis", "-q:a", &q.vorbis_q().to_string()]),
        AudioFmt::Opus => push(&mut a, &["-c:a", "libopus", "-b:a", &kbps(q)]),
        AudioFmt::Flac => push(&mut a, &["-c:a", "flac"]),
        AudioFmt::Wav => push(&mut a, &["-c:a", "pcm_s16le"]),
    }
    a
}

/// Baut die vollständige ffmpeg-Argumentliste für Ton oder Video.
/// (Bilder laufen über die image-Crate, nicht über ffmpeg.)
/// `audio_track`: bei Ton als Ziel die gewünschte Tonspur (0 = erste Tonspur der Datei).
pub fn ffmpeg_args(input: &str, output: &str, target: Target, aspect: Aspect, fit: Fit, q: Quality, audio_track: Option<u32>, sprache: Sprache) -> Vec<String> {
    let mut a: Vec<String> = vec!["-hide_banner".into(), "-y".into(), "-i".into(), input.into()];
    match target {
        Target::Audio(f) => {
            if let Some(n) = audio_track {
                push(&mut a, &["-map", &format!("0:a:{n}")]);
            }
            // Nur Ton: Bild, Untertitel und Datenspuren weglassen (z. B. aus MKV-Filmen).
            push(&mut a, &["-vn", "-sn", "-dn"]);
            // Ton aus Filmen ist leise (Kinomischung, 5.1 auf Stereo heruntergemischt): doppelt so
            // laut (+6 dB); der Begrenzer fängt Spitzen ab, damit nichts verzerrt.
            // Mit «Sprache verbessern» übernimmt dessen Filterkette.
            if let Some(af) = ton_filter(input, sprache) {
                push(&mut a, &["-af", &af]);
            }
            a.extend(audio_codec_args(f, q));
        }
        Target::Video(f) => {
            // Für yuv420p/H.264 müssen die Kanten gerade sein – sonst absichern.
            let vf = video_vf(aspect, fit, "black")
                .unwrap_or_else(|| "scale=trunc(iw/2)*2:trunc(ih/2)*2".into());
            let crf = q.video_crf().to_string();
            match f {
                VideoFmt::Mp4 | VideoFmt::Mov | VideoFmt::Mkv => {
                    push(&mut a, &["-vf", &vf]);
                    push(&mut a, &["-c:v", "libx264", "-preset", "medium", "-crf", &crf, "-pix_fmt", "yuv420p"]);
                    push(&mut a, &["-c:a", "aac", "-b:a", "160k"]);
                    if sprache.aktiv() {
                        if let Some(af) = ton_filter(input, sprache) {
                            push(&mut a, &["-af", &af]);
                        }
                    }
                }
                VideoFmt::Webm => {
                    push(&mut a, &["-vf", &vf]);
                    push(&mut a, &["-c:v", "libvpx-vp9", "-crf", &crf, "-b:v", "0"]);
                    push(&mut a, &["-c:a", "libopus", "-b:a", "128k"]);
                    if sprache.aktiv() {
                        if let Some(af) = ton_filter(input, sprache) {
                            push(&mut a, &["-af", &af]);
                        }
                    }
                }
                VideoFmt::Gif => {
                    let gvf = format!("{vf},fps=15,split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse");
                    push(&mut a, &["-vf", &gvf, "-an"]);
                }
            }
        }
        Target::Image(_) => {} // nicht über ffmpeg
    }
    a.push(output.into());
    a
}

fn kbps(q: Quality) -> String {
    format!("{}k", q.audio_kbps())
}
fn push(a: &mut Vec<String>, items: &[&str]) {
    for s in items {
        a.push((*s).to_string());
    }
}

// ---------------- Tonspuren eines Videos ----------------

/// Eine Tonspur, wie ffmpeg sie beim Öffnen einer Datei auflistet.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AudioTrack {
    /// Nummer unter den Tonspuren (für `-map 0:a:N`).
    pub index: u32,
    pub lang: Option<String>,
    pub codec: String,
    pub channels: String,
    pub title: Option<String>,
    pub default: bool,
}

impl AudioTrack {
    /// z. B. «Deutsch · AC3 · 5.1 · Kommentar».
    pub fn label(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        parts.push(self.lang.as_deref().map(language_name).unwrap_or_else(|| format!("Spur {}", self.index + 1)));
        if !self.codec.is_empty() {
            parts.push(self.codec.clone());
        }
        if !self.channels.is_empty() {
            parts.push(self.channels.clone());
        }
        if let Some(t) = &self.title {
            parts.push(t.clone());
        }
        parts.join(" · ")
    }
}

/// Sprachcode (ISO 639-2) → deutscher Name.
pub fn language_name(code: &str) -> String {
    match code.to_lowercase().as_str() {
        "ger" | "deu" | "de" => "Deutsch".into(),
        "gsw" => "Schweizerdeutsch".into(),
        "eng" | "en" => "Englisch".into(),
        "fre" | "fra" | "fr" => "Französisch".into(),
        "ita" | "it" => "Italienisch".into(),
        "spa" | "es" => "Spanisch".into(),
        "por" | "pt" => "Portugiesisch".into(),
        "dut" | "nld" | "nl" => "Niederländisch".into(),
        "rus" | "ru" => "Russisch".into(),
        "jpn" | "ja" => "Japanisch".into(),
        "kor" | "ko" => "Koreanisch".into(),
        "chi" | "zho" | "zh" => "Chinesisch".into(),
        "tur" | "tr" => "Türkisch".into(),
        "pol" | "pl" => "Polnisch".into(),
        other => other.to_uppercase(),
    }
}

/// Liest die Tonspuren aus der Ausgabe von `ffmpeg -i DATEI` (stderr).
pub fn parse_audio_tracks(text: &str) -> Vec<AudioTrack> {
    let mut out: Vec<AudioTrack> = Vec::new();
    let mut in_audio = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("Stream #") {
            in_audio = false;
            let Some(pos) = t.find(": Audio: ") else { continue };
            let head = &t[..pos];
            // Sprache steht in Klammern: «Stream #0:1(ger)» oder «Stream #0:2[0x1100](eng)».
            let lang = head
                .rfind('(')
                .and_then(|i| head[i + 1..].strip_suffix(')'))
                .filter(|l| !l.is_empty() && *l != "und")
                .map(str::to_string);
            let rest = &t[pos + 9..];
            let fields: Vec<&str> = rest.split(", ").collect();
            let codec = fields
                .first()
                .map(|c| c.split([' ', '(']).next().unwrap_or("").to_uppercase())
                .unwrap_or_default();
            let channels = fields
                .iter()
                .skip(1)
                .find(|f| !f.ends_with(" Hz"))
                .map(|c| {
                    let c = c.split('(').next().unwrap_or("").trim();
                    match c {
                        "mono" => "Mono".to_string(),
                        "stereo" => "Stereo".to_string(),
                        other => other.to_string(),
                    }
                })
                .unwrap_or_default();
            out.push(AudioTrack {
                index: out.len() as u32,
                lang,
                codec,
                channels,
                title: None,
                default: rest.contains("(default)"),
            });
            in_audio = true;
        } else if in_audio && t.starts_with("title") {
            if let (Some(tr), Some((_, v))) = (out.last_mut(), t.split_once(':')) {
                let v = v.trim();
                if !v.is_empty() {
                    tr.title = Some(v.to_string());
                }
            }
        } else if !line.starts_with("    ") && !t.is_empty() && !t.starts_with("Metadata") {
            in_audio = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MKV: &str = "Input #0, matroska,webm, from 'film.mkv':
  Metadata:
    title           : Ferien
  Duration: 01:32:10.04, start: 0.000000, bitrate: 4500 kb/s
  Stream #0:0(eng): Video: h264 (High), yuv420p(progressive), 1920x1080, 25 fps (default)
  Stream #0:1(ger): Audio: ac3, 48000 Hz, 5.1(side), fltp, 448 kb/s (default)
    Metadata:
      title           : Deutsch DD 5.1
  Stream #0:2[0x1100](eng): Audio: aac (LC), 48000 Hz, stereo, fltp
  Stream #0:3(ger): Subtitle: subrip
    Metadata:
      title           : Forced
At least one output file must be specified";

    use std::path::{Path, PathBuf};

    /// Frischer, leerer Testordner.
    fn testordner(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wandelwerk-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("Testordner");
        d
    }

    fn datei(p: &Path) {
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).expect("Ordner");
        }
        std::fs::write(p, b"x").expect("Datei");
    }

    #[test]
    fn ordner_durchsuchen() {
        let d = testordner("sammeln");
        datei(&d.join("b.mp3"));
        datei(&d.join("a.WAV"));
        datei(&d.join("notiz.txt"));
        datei(&d.join("Alben/Eins/01 Lied.wav"));
        datei(&d.join("Alben/Eins/cover.jpg"));
        datei(&d.join("Alben/Zwei/song.mp3"));
        datei(&d.join("Alben/Zwei/song.wav"));
        datei(&d.join("Filme/f.mkv"));
        std::fs::create_dir_all(d.join("Leer")).expect("leer");
        let f = sammeln(std::slice::from_ref(&d), MAX_TIEFE, MAX_DATEIEN);
        let rel: Vec<String> = f
            .dateien
            .iter()
            .map(|(p, _)| p.strip_prefix(&d).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default())
            .collect();
        assert_eq!(rel, ["a.WAV", "b.mp3", "Alben/Eins/01 Lied.wav", "Alben/Eins/cover.jpg", "Alben/Zwei/song.mp3", "Alben/Zwei/song.wav", "Filme/f.mkv"]);
        assert_eq!(f.zaehlen(), [5, 1, 1]);
        assert_eq!(f.ordner_uebersprungen, 0);
        assert!(!f.abgeschnitten);
        // Dieselbe Datei zweimal (Ordner + Datei darin) wird nur einmal aufgenommen.
        let f2 = sammeln(&[d.clone(), d.join("b.mp3")], MAX_TIEFE, MAX_DATEIEN);
        assert_eq!(f2.dateien.len(), 7);
        // Einzelne Datei als Wurzel; fehlende Wurzel zählt als übersprungen.
        let f3 = sammeln(&[d.join("a.WAV"), d.join("gibtsnicht")], MAX_TIEFE, MAX_DATEIEN);
        assert_eq!(f3.dateien.len(), 1);
        assert_eq!(f3.ordner_uebersprungen, 1);
        // Obergrenze
        let f4 = sammeln(std::slice::from_ref(&d), MAX_TIEFE, 3);
        assert_eq!(f4.dateien.len(), 3);
        assert!(f4.abgeschnitten);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn ordner_tiefe_begrenzt() {
        let d = testordner("tiefe");
        let mut p = d.clone();
        for i in 0..5 {
            p = p.join(format!("t{i}"));
        }
        datei(&p.join("tief.wav"));
        datei(&d.join("t0/oben.wav"));
        let f = sammeln(std::slice::from_ref(&d), 2, MAX_DATEIEN);
        assert_eq!(f.dateien.len(), 1);
        assert!(f.abgeschnitten);
        assert_eq!(sammeln(std::slice::from_ref(&d), MAX_TIEFE, MAX_DATEIEN).dateien.len(), 2);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn ordner_verknuepfungen() {
        let d = testordner("links");
        datei(&d.join("echt/lied.wav"));
        // Schleife: Unterordner verweist auf den Startordner.
        std::os::unix::fs::symlink(&d, d.join("echt/schleife")).expect("Link");
        std::os::unix::fs::symlink(d.join("echt/lied.wav"), d.join("verweis.mp3")).expect("Link");
        let f = sammeln(std::slice::from_ref(&d), MAX_TIEFE, MAX_DATEIEN);
        assert_eq!(f.dateien.len(), 2, "{:?}", f.dateien);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn ordner_unlesbar() {
        use std::os::unix::fs::PermissionsExt;
        let d = testordner("unlesbar");
        datei(&d.join("zu/geheim.wav"));
        datei(&d.join("offen.wav"));
        std::fs::set_permissions(d.join("zu"), std::fs::Permissions::from_mode(0o000)).expect("Rechte");
        let f = sammeln(std::slice::from_ref(&d), MAX_TIEFE, MAX_DATEIEN);
        let lesbar = std::fs::read_dir(d.join("zu")).is_ok(); // als root trotzdem lesbar
        std::fs::set_permissions(d.join("zu"), std::fs::Permissions::from_mode(0o755)).expect("Rechte");
        if !lesbar {
            assert_eq!(f.dateien.len(), 1);
            assert_eq!(f.ordner_uebersprungen, 1);
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn schon_im_zielformat() {
        let ogg = Target::Audio(AudioFmt::Ogg);
        assert!(schon_im_ziel(Path::new("a/lied.ogg"), ogg));
        assert!(schon_im_ziel(Path::new("a/LIED.OGG"), ogg));
        assert!(!schon_im_ziel(Path::new("a/lied.wav"), ogg));
        assert!(!schon_im_ziel(Path::new("a/lied.mp3"), ogg));
        assert!(!schon_im_ziel(Path::new("a/ogg"), ogg));
        assert!(schon_im_ziel(Path::new("bild.JPEG"), Target::Image(ImageFmt::Jpg)));
        assert!(schon_im_ziel(Path::new("bild.tif"), Target::Image(ImageFmt::Tiff)));
        assert!(!schon_im_ziel(Path::new("film.mkv"), Target::Audio(AudioFmt::Mp3)));
    }

    #[test]
    fn zielformat_gilt_fuer() {
        let ogg = Target::Audio(AudioFmt::Ogg);
        assert!(ziel_passt(MediaKind::Audio, ogg));
        assert!(ziel_passt(MediaKind::Video, ogg));
        assert!(!ziel_passt(MediaKind::Image, ogg));
        assert!(!ziel_passt(MediaKind::Audio, Target::Video(VideoFmt::Mp4)));
        assert!(ziel_passt(MediaKind::Image, Target::Image(ImageFmt::Png)));
    }

    #[test]
    fn ablage_ordner() {
        let aus = Path::new("/home/oli/Umgewandelt");
        let inp = Path::new("/musik/Alben/song.wav");
        assert_eq!(ziel_ordner(inp, Ablage::Ausgabeordner, aus), aus);
        assert_eq!(ziel_ordner(inp, Ablage::NebenOriginal, aus), Path::new("/musik/Alben"));
        assert_eq!(ziel_ordner(Path::new("song.wav"), Ablage::NebenOriginal, aus), Path::new("."));
        assert_eq!(Ablage::default(), Ablage::Ausgabeordner);
    }

    #[test]
    fn original_loeschen_nur_wenn_sicher() {
        let i = Path::new("/m/song.wav");
        let o = Path::new("/m/song.ogg");
        assert!(darf_original_loeschen(true, i, o, Some(1234), false));
        assert!(!darf_original_loeschen(false, i, o, Some(1234), false), "Auftrag fehlgeschlagen");
        assert!(!darf_original_loeschen(true, i, o, Some(0), false), "leere Ausgabe");
        assert!(!darf_original_loeschen(true, i, o, None, false), "Ausgabe fehlt");
        assert!(!darf_original_loeschen(true, i, o, Some(9), true), "dieselbe Datei");
        assert!(!darf_original_loeschen(true, i, i, Some(9), false), "gleicher Pfad");
        assert!(!darf_original_loeschen(true, i, Path::new(""), Some(9), false));
    }

    #[test]
    fn auswahl_vorschlag() {
        assert_eq!(Auswahl::vorschlag([900, 50, 3]), Auswahl::Ton);
        assert_eq!(Auswahl::vorschlag([1, 9, 0]), Auswahl::Bilder);
        assert_eq!(Auswahl::vorschlag([0, 0, 4]), Auswahl::Videos);
        assert_eq!(Auswahl::vorschlag([5, 5, 0]), Auswahl::Alles);
        assert_eq!(Auswahl::vorschlag([0, 0, 0]), Auswahl::Alles);
        assert!(Auswahl::Ton.passt(MediaKind::Audio) && !Auswahl::Ton.passt(MediaKind::Video));
        assert!(Auswahl::Alles.passt(MediaKind::Image));
        assert_eq!(gleichzeitig(1), 1);
        assert_eq!(gleichzeitig(4), 2);
        assert_eq!(gleichzeitig(64), 4);
        assert_eq!(gleichzeitig(0), 1);
    }

    #[test]
    fn ogg_argumente() {
        let a = ffmpeg_args("/m/song.wav", "/m/song.ogg", Target::Audio(AudioFmt::Ogg), Aspect::Original, Fit::Crop, Quality::Hoch, None, Sprache::default()).join(" ");
        assert_eq!(a, "-hide_banner -y -i /m/song.wav -vn -sn -dn -c:a libvorbis -q:a 8 /m/song.ogg");
        assert_eq!(audio_codec_args(AudioFmt::Ogg, Quality::Klein).join(" "), "-c:a libvorbis -q:a 4");
    }

    #[test]
    fn tonspuren_lesen() {
        let t = parse_audio_tracks(MKV);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].label(), "Deutsch · AC3 · 5.1 · Deutsch DD 5.1");
        assert!(t[0].default);
        assert_eq!(t[1].index, 1);
        assert_eq!(t[1].label(), "Englisch · AAC · Stereo");
        assert_eq!(t[1].title, None);
        let plain = parse_audio_tracks("  Stream #0:1: Audio: mp3, 44100 Hz, mono, fltp, 128 kb/s");
        assert_eq!(plain[0].label(), "Spur 1 · MP3 · Mono");
        assert!(parse_audio_tracks("  Stream #0:0: Video: h264").is_empty());
    }

    #[test]
    fn ton_aus_video() {
        let a = ffmpeg_args("film.mkv", "film.mp3", Target::Audio(AudioFmt::Mp3), Aspect::Original, Fit::Crop, Quality::Hoch, Some(1), Sprache::default());
        let s = a.join(" ");
        assert!(s.contains("-map 0:a:1 -vn -sn -dn -af volume=6dB,alimiter=limit=0.97:level=0 -c:a libmp3lame -b:a 256k"), "{s}");
        assert!(s.ends_with("film.mp3"));
        let b = ffmpeg_args("lied.wav", "lied.mp3", Target::Audio(AudioFmt::Mp3), Aspect::Original, Fit::Crop, Quality::Mittel, None, Sprache::default()).join(" ");
        assert!(!b.contains("-map"));
        // Reine Tondateien bleiben so laut, wie sie sind.
        assert!(!b.contains("volume="), "{b}");
        let c = ffmpeg_args("Ferien.MP4", "Ferien.m4a", Target::Audio(AudioFmt::M4a), Aspect::Original, Fit::Crop, Quality::Mittel, None, Sprache::default()).join(" ");
        assert!(c.contains("-af volume=6dB"), "{c}");
    }

    #[test]
    fn sprache_verbessern() {
        let alles = Sprache { angleichen: true, brummen: true, rauschen: Rauschen::Mittel, sprecher: false };
        let f = ton_filter("gespraech.m4a", alles).unwrap_or_default();
        assert_eq!(f, "highpass=f=80,highpass=f=80,highpass=f=80,arnndn=m=werk-stimme.rnnn:mix=0.8,speechnorm=e=12.5:r=0.0001:l=1,alimiter=limit=0.97:level=0");
        // Aus: Tondatei unverändert, Film +6 dB wie bisher.
        assert_eq!(ton_filter("lied.mp3", Sprache::default()), None);
        assert_eq!(ton_filter("film.mkv", Sprache::default()).as_deref(), Some(FILM_LAUTER));
        // Nur entrauschen aus einem Film: Film bleibt lauter.
        let r = ton_filter("film.mp4", Sprache { rauschen: Rauschen::Stark, ..Sprache::default() }).unwrap_or_default();
        assert!(r.contains("mix=1") && r.contains("volume=6dB"), "{r}");
        // Auch beim Video-Ziel wird der Ton bearbeitet.
        let v = ffmpeg_args("a.mov", "a.mp4", Target::Video(VideoFmt::Mp4), Aspect::Original, Fit::Crop, Quality::Hoch, None, alles).join(" ");
        assert!(v.contains("-af highpass"), "{v}");
    }

    #[test]
    fn zwei_sprecher() {
        // «Zwei Sprecher» schliesst speechnorm ein, auch ohne «angleichen».
        let s = Sprache { sprecher: true, ..Sprache::default() };
        assert!(s.aktiv());
        assert!(ton_filter("a.mp3", s).unwrap_or_default().contains("speechnorm"));
        assert_eq!(sprecher_filter(s, 0.0, 0.0, 0.0), "speechnorm=e=12.5:r=0.0001:l=1");
        let r = Sprache { sprecher: true, brummen: true, rauschen: Rauschen::Leicht, angleichen: false };
        assert_eq!(
            sprecher_filter(r, -2.9, 5.84, 7.2),
            "highpass=f=80,highpass=f=80,highpass=f=80,arnndn=m=werk-stimme.rnnn:mix=0.5,speechnorm=e=12.5:r=0.0001:l=1,\
equalizer=f=220:t=q:w=1:g=-2.9,equalizer=f=2500:t=q:w=1:g=5.8,highshelf=f=4500:g=7.2"
        );
        assert_eq!(audio_codec_args(AudioFmt::Opus, Quality::Klein).join(" "), "-c:a libopus -b:a 128k");
    }
}
