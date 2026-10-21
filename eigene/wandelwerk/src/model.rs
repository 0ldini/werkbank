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

/// Baut die vollständige ffmpeg-Argumentliste für Ton oder Video.
/// (Bilder laufen über die image-Crate, nicht über ffmpeg.)
/// `audio_track`: bei Ton als Ziel die gewünschte Tonspur (0 = erste Tonspur der Datei).
pub fn ffmpeg_args(input: &str, output: &str, target: Target, aspect: Aspect, fit: Fit, q: Quality, audio_track: Option<u32>) -> Vec<String> {
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
            if is_video_path(input) {
                push(&mut a, &["-af", FILM_LAUTER]);
            }
            match f {
                AudioFmt::Mp3 => push(&mut a, &["-c:a", "libmp3lame", "-b:a", &kbps(q)]),
                AudioFmt::M4a => push(&mut a, &["-c:a", "aac", "-b:a", &kbps(q)]),
                AudioFmt::Ogg => push(&mut a, &["-c:a", "libvorbis", "-b:a", &kbps(q)]),
                AudioFmt::Opus => push(&mut a, &["-c:a", "libopus", "-b:a", &kbps(q)]),
                AudioFmt::Flac => push(&mut a, &["-c:a", "flac"]),
                AudioFmt::Wav => push(&mut a, &["-c:a", "pcm_s16le"]),
            }
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
                }
                VideoFmt::Webm => {
                    push(&mut a, &["-vf", &vf]);
                    push(&mut a, &["-c:v", "libvpx-vp9", "-crf", &crf, "-b:v", "0"]);
                    push(&mut a, &["-c:a", "libopus", "-b:a", "128k"]);
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
        let a = ffmpeg_args("film.mkv", "film.mp3", Target::Audio(AudioFmt::Mp3), Aspect::Original, Fit::Crop, Quality::Hoch, Some(1));
        let s = a.join(" ");
        assert!(s.contains("-map 0:a:1 -vn -sn -dn -af volume=6dB,alimiter=limit=0.97:level=0 -c:a libmp3lame -b:a 256k"), "{s}");
        assert!(s.ends_with("film.mp3"));
        let b = ffmpeg_args("lied.wav", "lied.mp3", Target::Audio(AudioFmt::Mp3), Aspect::Original, Fit::Crop, Quality::Mittel, None).join(" ");
        assert!(!b.contains("-map"));
        // Reine Tondateien bleiben so laut, wie sie sind.
        assert!(!b.contains("volume="), "{b}");
        let c = ffmpeg_args("Ferien.MP4", "Ferien.m4a", Target::Audio(AudioFmt::M4a), Aspect::Original, Fit::Crop, Quality::Mittel, None).join(" ");
        assert!(c.contains("-af volume=6dB"), "{c}");
    }
}
