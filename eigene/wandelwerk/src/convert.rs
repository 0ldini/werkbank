//! Umwandel-Motor: Bilder über die image-Crate (ohne Fremdprogramm), Ton und
//! Video über ffmpeg. Jeder Auftrag läuft in einem eigenen Thread und meldet
//! Fortschritt/Status über einen geteilten Zustand.

use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::model::{self, Aspect, Fit, MediaKind, Plan, Quality, Target};

/// Ein Umwandel-Auftrag.
#[derive(Clone)]
pub struct Job {
    pub input: PathBuf,
    pub target: Target,
    pub aspect: Aspect,
    pub fit: Fit,
    pub quality: Quality,
    pub out_dir: PathBuf,
}

/// Geteilter Fortschritt/Status eines Auftrags.
pub struct JobState {
    pub progress: AtomicU32, // 0..=1000 (Promille); 1000 = fertig
    pub done: AtomicBool,
    pub ok: AtomicBool,
    pub running: AtomicBool,
    pub msg: Mutex<String>,
    pub out: Mutex<String>,
}

impl JobState {
    fn new() -> Arc<Self> {
        Arc::new(JobState {
            progress: AtomicU32::new(0),
            done: AtomicBool::new(false),
            ok: AtomicBool::new(false),
            running: AtomicBool::new(false),
            msg: Mutex::new(String::new()),
            out: Mutex::new(String::new()),
        })
    }
    pub fn percent(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1000.0
    }
    fn fail(&self, m: impl Into<String>) {
        *self.msg.lock().unwrap() = m.into();
        self.done.store(true, Ordering::SeqCst);
        self.running.store(false, Ordering::SeqCst);
    }
    fn finish(&self, out: &Path) {
        *self.out.lock().unwrap() = out.display().to_string();
        self.progress.store(1000, Ordering::SeqCst);
        self.ok.store(true, Ordering::SeqCst);
        self.done.store(true, Ordering::SeqCst);
        self.running.store(false, Ordering::SeqCst);
    }
}

/// Findet ffmpeg: neben der eigenen .exe oder auf dem PATH.
pub fn find_ffmpeg() -> Option<String> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
            let cand = dir.join(name);
            if cand.exists() {
                return Some(cand.display().to_string());
            }
        }
    }
    // Auf dem PATH?
    let ok = Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        Some("ffmpeg".into())
    } else {
        None
    }
}

/// Zielpfad ohne Kollision: hängt " (n)" an, falls nötig.
fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut n = 1;
    loop {
        let p = dir.join(model::name_with_index(stem, ext, n));
        if !p.exists() {
            return p;
        }
        n += 1;
        if n > 9999 {
            return dir.join(model::name_with_index(stem, ext, n));
        }
    }
}

/// Startet einen Auftrag in einem Hintergrund-Thread.
pub fn start_job(job: Job, ffmpeg: Option<String>) -> Arc<JobState> {
    let state = JobState::new();
    let st = state.clone();
    std::thread::spawn(move || {
        st.running.store(true, Ordering::SeqCst);
        let res = match job.target.kind() {
            MediaKind::Image => convert_image(&job, &st),
            MediaKind::Audio | MediaKind::Video => convert_av(&job, ffmpeg.as_deref(), &st),
        };
        if let Err(e) = res {
            st.fail(e);
        }
    });
    state
}

fn out_target(job: &Job) -> PathBuf {
    let stem = job
        .input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "ausgabe".into());
    let _ = std::fs::create_dir_all(&job.out_dir);
    unique_path(&job.out_dir, &stem, job.target.ext())
}

// ---------------- Bilder (image-Crate) ----------------

fn convert_image(job: &Job, st: &JobState) -> Result<(), String> {
    use image::{DynamicImage, Rgba, RgbaImage};

    let img = image::open(&job.input).map_err(|e| format!("Bild lesen: {e}"))?;
    st.progress.store(300, Ordering::Relaxed);
    let (sw, sh) = (img.width(), img.height());
    let plan = model::plan_box(sw, sh, job.aspect, job.fit, false);

    let out_img: DynamicImage = match plan {
        Plan::Keep => img,
        Plan::Crop { w, h, x, y } => img.crop_imm(x, y, w, h),
        Plan::Stretch { w, h } => img.resize_exact(w, h, image::imageops::FilterType::Lanczos3),
        Plan::Pad { cw, ch, x, y } => {
            let mut canvas = RgbaImage::from_pixel(cw, ch, Rgba([0, 0, 0, 255]));
            let top = img.to_rgba8();
            image::imageops::overlay(&mut canvas, &top, x as i64, y as i64);
            DynamicImage::ImageRgba8(canvas)
        }
    };
    st.progress.store(700, Ordering::Relaxed);

    let out = out_target(job);
    save_image(&out_img, &out, job.target, job.quality).map_err(|e| format!("Bild schreiben: {e}"))?;
    st.finish(&out);
    Ok(())
}

fn save_image(
    img: &image::DynamicImage,
    path: &Path,
    target: Target,
    q: Quality,
) -> Result<(), String> {
    use image::ImageFormat;
    use model::ImageFmt;
    match target {
        Target::Image(ImageFmt::Jpg) => {
            // JPEG kennt keine Transparenz -> RGB, mit Qualität.
            let rgb = img.to_rgb8();
            let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
            let mut w = std::io::BufWriter::new(f);
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, q.image_q());
            enc.encode_image(&rgb).map_err(|e| e.to_string())?;
            Ok(())
        }
        Target::Image(ImageFmt::Png) => img.save_with_format(path, ImageFormat::Png).map_err(|e| e.to_string()),
        Target::Image(ImageFmt::Webp) => img.save_with_format(path, ImageFormat::WebP).map_err(|e| e.to_string()),
        Target::Image(ImageFmt::Bmp) => img.save_with_format(path, ImageFormat::Bmp).map_err(|e| e.to_string()),
        Target::Image(ImageFmt::Gif) => img.save_with_format(path, ImageFormat::Gif).map_err(|e| e.to_string()),
        Target::Image(ImageFmt::Tiff) => img.save_with_format(path, ImageFormat::Tiff).map_err(|e| e.to_string()),
        _ => img.save(path).map_err(|e| e.to_string()),
    }
}

// ---------------- Ton & Video (ffmpeg) ----------------

fn convert_av(job: &Job, ffmpeg: Option<&str>, st: &JobState) -> Result<(), String> {
    let ffmpeg = ffmpeg.ok_or("ffmpeg wurde nicht gefunden. ffmpeg.exe neben das Programm legen oder installieren.")?;
    let out = out_target(job);
    let input = job.input.display().to_string();
    let output = out.display().to_string();
    let args = model::ffmpeg_args(&input, &output, job.target, job.aspect, job.fit, job.quality);

    let mut child = Command::new(ffmpeg)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg konnte nicht gestartet werden: {e}"))?;

    let stderr = child.stderr.take().ok_or("kein ffmpeg-Ausgabekanal")?;
    let mut reader = BufReader::new(stderr);
    let mut line: Vec<u8> = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    let mut dur = 0f64;
    let mut tail = String::new();

    loop {
        match reader.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                let b = byte[0];
                if b == b'\r' || b == b'\n' {
                    if !line.is_empty() {
                        let s = String::from_utf8_lossy(&line).into_owned();
                        if let Some(d) = parse_duration(&s) {
                            dur = d;
                        }
                        if let Some(t) = parse_time(&s) {
                            if dur > 0.0 {
                                let pm = ((t / dur) * 1000.0).clamp(0.0, 999.0) as u32;
                                st.progress.store(pm, Ordering::Relaxed);
                            }
                        }
                        push_tail(&mut tail, &s);
                        line.clear();
                    }
                } else {
                    line.push(b);
                }
            }
            Err(_) => break,
        }
    }

    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        st.finish(&out);
        Ok(())
    } else {
        Err(format!("ffmpeg-Fehler. {}", last_meaningful(&tail)))
    }
}

/// "  Duration: 00:02:14.00, start: ..." -> Sekunden
fn parse_duration(s: &str) -> Option<f64> {
    let i = s.find("Duration:")?;
    let rest = s[i + 9..].trim_start();
    let end = rest.find(',').unwrap_or(rest.len());
    parse_hms(rest[..end].trim())
}

/// "... time=00:00:30.12 ..." -> Sekunden
fn parse_time(s: &str) -> Option<f64> {
    let i = s.find("time=")?;
    let rest = &s[i + 5..];
    let end = rest.find(' ').unwrap_or(rest.len());
    parse_hms(rest[..end].trim())
}

/// "HH:MM:SS.ss" -> Sekunden
fn parse_hms(s: &str) -> Option<f64> {
    let p: Vec<&str> = s.split(':').collect();
    if p.len() != 3 {
        return None;
    }
    let h: f64 = p[0].parse().ok()?;
    let m: f64 = p[1].parse().ok()?;
    let sec: f64 = p[2].parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + sec)
}

fn push_tail(tail: &mut String, line: &str) {
    tail.push_str(line);
    tail.push('\n');
    if tail.len() > 2000 {
        let cut = tail.len() - 2000;
        *tail = tail[cut..].to_string();
    }
}

fn last_meaningful(tail: &str) -> String {
    tail.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .to_string()
}
