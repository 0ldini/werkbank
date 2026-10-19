//! Ansicht des gewählten Eintrags: ein verkleinertes Standbild (Bild oder ein Frame aus
//! dem Video) und das Abspielen eines Videos direkt im Programmfenster. Beides über ffmpeg,
//! ohne egui – die Oberfläche holt sich die rohen RGB-Daten ab.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::model::{self, Item, Kind};

/// Längste Seite der Ansicht in Pixeln.
pub const MAX_EDGE: u32 = 960;
/// Bilder pro Sekunde beim Abspielen in der Ansicht.
pub const PLAY_FPS: u32 = 25;

/// Grösse, in die ein Bild w × h ohne Verzerrung in `max` × `max` passt (nie grösser als
/// das Original, mindestens 2 × 2, gerade Zahlen).
pub fn fit(w: u32, h: u32, max: u32) -> (u32, u32) {
    let (w, h) = (w.max(1) as f64, h.max(1) as f64);
    let f = (max as f64 / w.max(h)).min(1.0);
    let even = |v: f64| (((v * f) / 2.0).round() as u32 * 2).max(2);
    (even(w), even(h))
}

/// Stelle im Video, von der das Standbild genommen wird (nicht ganz am Anfang,
/// dort ist es oft schwarz).
pub fn still_at(secs: f32) -> f32 {
    if secs <= 0.0 {
        0.0
    } else {
        (secs / 3.0).min(1.0)
    }
}

/// ffmpeg-Argumente für ein Standbild in der Grösse tw × th als rohes RGB.
pub fn still_args(it: &Item, tw: u32, th: u32) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    let scale = format!("scale={tw}:{th}:flags=bilinear,format=rgb24");
    match it.kind {
        Kind::Video => {
            a.extend(["-ss".into(), format!("{:.2}", still_at(it.secs))]);
            a.extend(["-i".into(), it.path.display().to_string()]);
            a.extend(["-frames:v".into(), "1".into(), "-an".into(), "-vf".into(), scale]);
        }
        _ => {
            let o = model::orient_filter(it.orient);
            let vf = if o.is_empty() { scale } else { format!("{o},{scale}") };
            a.extend(["-noautorotate".into(), "-i".into(), it.path.display().to_string()]);
            a.extend(["-frames:v".into(), "1".into(), "-an".into(), "-vf".into(), vf]);
        }
    }
    a.extend(["-f".into(), "rawvideo".into(), "-pix_fmt".into(), "rgb24".into(), "-".into()]);
    a
}

/// ffmpeg-Argumente zum Abspielen eines Videos ab `from` Sekunden in Echtzeit.
pub fn play_args(it: &Item, tw: u32, th: u32, from: f32) -> Vec<String> {
    vec![
        "-re".into(),
        "-ss".into(),
        format!("{:.2}", from.max(0.0)),
        "-i".into(),
        it.path.display().to_string(),
        "-an".into(),
        "-vf".into(),
        format!("fps={PLAY_FPS},scale={tw}:{th}:flags=bilinear,format=rgb24"),
        "-f".into(),
        "rawvideo".into(),
        "-pix_fmt".into(),
        "rgb24".into(),
        "-".into(),
    ]
}

fn ffmpeg(ff: &str) -> Command {
    let mut c = Command::new(ff);
    c.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
    model::hide_window(&mut c);
    c
}

/// Ein verkleinertes Standbild.
pub struct Still {
    pub w: u32,
    pub h: u32,
    pub rgb: Vec<u8>,
}

/// Lädt das Standbild eines Eintrags (blockiert, also im Hilfsthread aufrufen).
pub fn load_still(ff: &str, it: &Item) -> Result<Still, String> {
    let (w, h) = fit(it.w, it.h, MAX_EDGE);
    let o = ffmpeg(ff)
        .args(still_args(it, w, h))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("ffmpeg startet nicht: {e}"))?;
    if o.stdout.len() != (w * h * 3) as usize {
        return Err(format!("{} lässt sich nicht vollständig lesen (beschädigt?)", it.name));
    }
    Ok(Still { w, h, rgb: o.stdout })
}

struct Shared {
    frame: Mutex<Option<Vec<u8>>>,
    count: AtomicU32,
    done: AtomicBool,
}

/// Spielt ein Video verkleinert ab. Beim Verwerfen wird ffmpeg beendet.
pub struct Player {
    child: Child,
    shared: Arc<Shared>,
    pub path: std::path::PathBuf,
    pub w: u32,
    pub h: u32,
    from: f32,
    pub secs: f32,
}

impl Player {
    /// Startet das Abspielen; `repaint` wird bei jedem neuen Bild aufgerufen.
    pub fn start(ff: &str, it: &Item, from: f32, repaint: Box<dyn Fn() + Send>) -> Result<Player, String> {
        let (w, h) = fit(it.w, it.h, MAX_EDGE);
        let from = if from >= it.secs - 0.2 { 0.0 } else { from.max(0.0) };
        let mut child = ffmpeg(ff)
            .args(play_args(it, w, h, from))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("ffmpeg startet nicht: {e}"))?;
        let mut out = child.stdout.take().ok_or("kein ffmpeg-Ausgang")?;
        let shared = Arc::new(Shared { frame: Mutex::new(None), count: AtomicU32::new(0), done: AtomicBool::new(false) });
        let sh = shared.clone();
        let size = (w * h * 3) as usize;
        std::thread::spawn(move || {
            let mut buf = vec![0u8; size];
            while out.read_exact(&mut buf).is_ok() {
                if let Ok(mut f) = sh.frame.lock() {
                    *f = Some(buf.clone());
                }
                sh.count.fetch_add(1, Ordering::Relaxed);
                repaint();
            }
            sh.done.store(true, Ordering::SeqCst);
            repaint();
        });
        Ok(Player { child, shared, path: it.path.clone(), w, h, from, secs: it.secs })
    }

    /// Neuestes Bild, falls seit dem letzten Abholen eines dazugekommen ist.
    pub fn take_frame(&self) -> Option<Vec<u8>> {
        self.shared.frame.lock().ok().and_then(|mut f| f.take())
    }

    /// Aktuelle Stelle im Video in Sekunden.
    pub fn position(&self) -> f32 {
        (self.from + self.shared.count.load(Ordering::Relaxed) as f32 / PLAY_FPS as f32).min(self.secs.max(0.0))
    }

    pub fn finished(&self) -> bool {
        self.shared.done.load(Ordering::SeqCst)
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn item(kind: Kind, w: u32, h: u32, orient: u8, secs: f32) -> Item {
        Item { path: PathBuf::from("x"), name: "x".into(), kind, w, h, orient, secs, modified: None }
    }

    #[test]
    fn groesse_passt() {
        assert_eq!(fit(6000, 4000, 960), (960, 640));
        assert_eq!(fit(4000, 6000, 960), (640, 960));
        assert_eq!(fit(12000, 2000, 960), (960, 160));
        assert_eq!(fit(300, 201, 960), (300, 202)); // nie vergrössert, gerade
        assert_eq!(fit(0, 0, 960), (2, 2));
    }

    #[test]
    fn standbild_und_abspielen() {
        let a = still_args(&item(Kind::Image, 4000, 6000, 6, 0.0), 640, 960);
        assert!(a.contains(&"-noautorotate".to_string()));
        assert!(a.iter().any(|s| s.starts_with("transpose=1,scale=640:960")));
        let v = still_args(&item(Kind::Video, 3840, 2160, 1, 12.0), 960, 540);
        assert_eq!(v[0..2], ["-ss".to_string(), "1.00".to_string()]);
        assert_eq!(still_at(0.6), 0.2);
        let p = play_args(&item(Kind::Video, 3840, 2160, 1, 12.0), 960, 540, 3.0);
        assert_eq!(p[0], "-re");
        assert!(p.iter().any(|s| s == "fps=25,scale=960:540:flags=bilinear,format=rgb24"));
    }
}
