//! Motor: baut aus der Liste die Diashow und schreibt sie als MP4.
//!
//! Jedes Bild wird von ffmpeg einmal in der Zielgrösse geladen (Drehung, Skalierung,
//! unscharfer Hintergrund), danach rechnet Diawerk jeden Frame selbst (Schwenk, Zoom,
//! Übergänge) und reicht die rohen Frames an einen zweiten ffmpeg zum Kodieren weiter.
//! Videos laufen über einen eigenen ffmpeg-Dekoder. Ein Ladethread bereitet das nächste
//! Bild vor, während das aktuelle berechnet wird.

use std::io::{BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::thread;

use crate::frames;
use crate::model::{self, Item, Kind, Look, Settings, Slot, Timeline, Transition};

/// Gemeinsamer Zustand zwischen Motor und Oberfläche.
pub struct Control {
    pub done: AtomicU32,
    pub total: AtomicU32,
    pub cancel: AtomicBool,
    pub finished: AtomicBool,
    pub result: Mutex<Option<Result<(), String>>>,
    pub log: Mutex<Vec<String>>,
}

impl Control {
    pub fn new() -> Arc<Control> {
        Arc::new(Control {
            done: AtomicU32::new(0),
            total: AtomicU32::new(0),
            cancel: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            result: Mutex::new(None),
            log: Mutex::new(Vec::new()),
        })
    }
    pub fn say(&self, msg: impl Into<String>) {
        if let Ok(mut l) = self.log.lock() {
            l.push(msg.into());
        }
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Startet das Rendern in einem eigenen Thread.
pub fn start(ffmpeg: String, items: Vec<Item>, s: Settings, out: std::path::PathBuf) -> Arc<Control> {
    let ctl = Control::new();
    let c2 = ctl.clone();
    thread::spawn(move || {
        let r = render(&ffmpeg, &items, &s, &out, &c2);
        if r.is_err() || c2.cancelled() {
            let _ = std::fs::remove_file(&out);
        }
        *c2.result.lock().unwrap() = Some(r);
        c2.finished.store(true, Ordering::SeqCst);
    });
    ctl
}

fn ffmpeg_cmd(ffmpeg: &str) -> Command {
    let mut c = Command::new(ffmpeg);
    c.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
    model::hide_window(&mut c);
    c
}

// ---------------------------------------------------------------- Quellen

/// Ein Clip, aus dem Frame für Frame gelesen wird.
enum Source {
    Still { canvas: Vec<u8>, cw: usize, ch: usize, zoom_in: bool, n: u32 },
    Pano { img: Vec<u8>, pw: usize, n: u32, edge: u32 },
    Video(VideoDec),
    /// Vor-/Abspann: fertige Karte in Ausgabegrösse, ohne Bewegung.
    Card(Vec<u8>),
    Blank,
}

impl Source {
    fn frame(&mut self, local: u32, out: &mut [u8], s: &Settings) {
        let (ow, oh) = (s.width as usize, s.height as usize);
        match self {
            Source::Still { canvas, cw, ch, zoom_in, n } => {
                let kb = s.zoom_pct.max(0.0) / 100.0;
                let p = frames::ease_move(local as f32 / (*n as f32 - 1.0).max(1.0));
                // Ausschnitt in Einheiten der Ausgabegrösse: 1+kb (alles) … 1 (näher dran)
                let win = if *zoom_in { 1.0 + kb - kb * p } else { 1.0 + kb * p };
                let (ww, hh) = (ow as f32 * win, oh as f32 * win);
                let x0 = (*cw as f32 - ww) / 2.0;
                let y0 = (*ch as f32 - hh) / 2.0;
                frames::sample(out, ow, oh, canvas, *cw, *ch, x0, y0, ww, hh);
            }
            Source::Pano { img, pw, n, edge } => {
                // Am Rand ruhig stehen, dazwischen von links nach rechts fahren.
                let (a, b) = (*edge, n.saturating_sub(*edge));
                let q = if b > a { (local.saturating_sub(a)) as f32 / (b - a) as f32 } else { 0.0 };
                let travel = (*pw as f32 - ow as f32).max(0.0);
                let x0 = frames::ease_move(q) * travel;
                frames::sample(out, ow, oh, img, *pw, oh, x0, 0.0, ow as f32, oh as f32);
            }
            Source::Card(c) => out.copy_from_slice(c),
            Source::Video(v) => v.next_frame(out),
            Source::Blank => out.fill(0),
        }
    }
}

/// ffmpeg-Dekoder: liefert ein Video als rohe Frames in Ausgabegrösse und Bildrate.
struct VideoDec {
    child: Child,
    out: BufReader<ChildStdout>,
    last: Vec<u8>,
    eof: bool,
}

impl VideoDec {
    fn open(ffmpeg: &str, it: &Item, s: &Settings) -> Result<VideoDec, String> {
        let mut cmd = ffmpeg_cmd(ffmpeg);
        cmd.arg("-i").arg(&it.path);
        if s.video_max_secs > 0.0 {
            cmd.arg("-t").arg(format!("{:.3}", s.video_max_secs));
        }
        cmd.args(["-an", "-vf"])
            .arg(model::video_filter(s))
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = cmd.spawn().map_err(|e| format!("ffmpeg startet nicht: {e}"))?;
        let out = BufReader::with_capacity(1 << 20, child.stdout.take().ok_or("kein ffmpeg-Ausgang")?);
        Ok(VideoDec {
            child,
            out,
            last: vec![0; (s.width * s.height * 3) as usize],
            eof: false,
        })
    }

    fn next_frame(&mut self, out: &mut [u8]) {
        if !self.eof {
            if self.out.read_exact(&mut self.last).is_err() {
                self.eof = true; // Video zu Ende: letztes Bild halten
            }
        }
        out.copy_from_slice(&self.last);
    }
}

impl Drop for VideoDec {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Lädt ein Bild fertig vorbereitet (läuft im Ladethread).
fn load_still(ffmpeg: &str, it: &Item, s: &Settings, n: u32, idx: usize, edge: u32) -> Result<Source, String> {
    let (filter, w, h) = model::image_filter(it, s);
    let mut cmd = ffmpeg_cmd(ffmpeg);
    cmd.arg("-noautorotate")
        .arg("-i")
        .arg(&it.path)
        .args(["-frames:v", "1", "-an", "-vf"])
        .arg(&filter)
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let o = cmd.output().map_err(|e| format!("ffmpeg startet nicht: {e}"))?;
    let want = w as usize * h as usize * 3;
    if o.stdout.len() != want {
        return Err(format!("{} lässt sich nicht vollständig lesen (beschädigt?)", it.name));
    }
    Ok(match model::look_for(it, s) {
        Look::Pano => Source::Pano { img: o.stdout, pw: w as usize, n, edge },
        _ => Source::Still { canvas: o.stdout, cw: w as usize, ch: h as usize, zoom_in: idx % 2 == 0, n },
    })
}

/// Zeichnet Vor- bzw. Abspann (Hintergrund, Text, goldene Linie, Porträt) mit ffmpeg.
fn load_card(ffmpeg: &str, intro: bool, s: &Settings) -> Result<Source, String> {
    let c = &s.credits;
    let font = model::find_font(c.font.as_deref())
        .ok_or("Keine Schrift gefunden – unter «Vor- und Abspann» eine Schriftdatei (.ttf) wählen")?;
    let portrait = c.portrait.as_ref().filter(|p| p.is_file());
    let (kicker, text) = if intro { (&c.intro_kicker, &c.intro_text) } else { (&c.outro_kicker, &c.outro_text) };
    let layout = model::card_layout(s.width, s.height, portrait.is_some(), kicker, &c.name, text);
    // Texte in Dateien, damit Sonderzeichen (Umlaute, Doppelpunkt, Apostroph) nie stören.
    let tag = format!("diawerk-{}-{}", std::process::id(), if intro { "a" } else { "b" });
    let mut files = Vec::new();
    for (i, b) in layout.blocks.iter().enumerate() {
        let f = std::env::temp_dir().join(format!("{tag}-{i}.txt"));
        std::fs::write(&f, &b.text).map_err(|e| format!("Textdatei: {e}"))?;
        files.push(f);
    }
    let orient = portrait.and_then(|p| model::jpeg_info(p)).map(|t| t.2).unwrap_or(1);
    let graph = model::card_graph(&layout, &font, &files, orient);
    let mut cmd = ffmpeg_cmd(ffmpeg);
    cmd.args(["-f", "lavfi", "-i"])
        .arg(format!("color=c={}:s={}x{}:d=1", model::CARD_BG, s.width, s.height));
    if let Some(p) = portrait {
        cmd.arg("-noautorotate").arg("-i").arg(p);
    }
    cmd.arg("-filter_complex")
        .arg(&graph)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let o = cmd.output();
    for f in &files {
        let _ = std::fs::remove_file(f);
    }
    let o = o.map_err(|e| format!("ffmpeg startet nicht: {e}"))?;
    if o.stdout.len() != (s.width * s.height * 3) as usize {
        let err = String::from_utf8_lossy(&o.stderr);
        let err = err.lines().last().unwrap_or("unbekannter Fehler");
        return Err(format!("{}: {err}", if intro { "Vorspann" } else { "Abspann" }));
    }
    Ok(Source::Card(o.stdout))
}

// ---------------------------------------------------------------- Rendern

pub fn render(ffmpeg: &str, items: &[Item], s: &Settings, out: &Path, ctl: &Control) -> Result<(), String> {
    if items.is_empty() {
        return Err("Die Liste ist leer.".into());
    }
    let tl: Timeline = model::build_timeline(items, s);
    ctl.total.store(tl.total, Ordering::SeqCst);
    ctl.done.store(0, Ordering::SeqCst);
    let (w, h) = (s.width as usize, s.height as usize);
    let frame_len = w * h * 3;
    let edge = model::pano_edge_frames(s);

    // --- Musik: Reihenfolge der Titel, bei Bedarf wiederholt
    let total_secs = tl.total_secs(s.fps);
    let mut playlist: Vec<std::path::PathBuf> = Vec::new();
    {
        let mut known: Vec<(std::path::PathBuf, f32)> = Vec::new();
        for m in s.music.iter().filter(|p| p.is_file()) {
            match model::audio_secs(ffmpeg, m) {
                Some(d) if d > 1.0 => known.push((m.clone(), d)),
                _ => ctl.say(format!("Musik nicht lesbar, ausgelassen: {}", m.display())),
            }
        }
        let sum: f32 = known.iter().map(|k| k.1).sum();
        if sum > 0.0 {
            let mut have = 0.0;
            'fill: loop {
                for (p, d) in &known {
                    playlist.push(p.clone());
                    have += d;
                    if have >= total_secs + 1.0 || playlist.len() >= 80 {
                        break 'fill;
                    }
                }
            }
        }
    }

    // --- Kodierer
    let mut enc = ffmpeg_cmd(ffmpeg);
    enc.arg("-y")
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-s"])
        .arg(format!("{}x{}", s.width, s.height))
        .arg("-framerate")
        .arg(s.fps.to_string())
        .args(["-i", "-"]);
    for m in &playlist {
        enc.arg("-i").arg(m);
    }
    enc.args(["-vf", "scale=out_color_matrix=bt709:out_range=tv:flags=bicubic+accurate_rnd,format=yuv420p"])
        .args(s.format.codec_args(s.quality, !playlist.is_empty()));
    if !playlist.is_empty() {
        let n = playlist.len();
        let mut fc = String::new();
        for i in 1..=n {
            fc.push_str(&format!("[{i}:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo[a{i}];"));
        }
        for i in 1..=n {
            fc.push_str(&format!("[a{i}]"));
        }
        fc.push_str(&format!(
            "concat=n={n}:v=0:a=1,afade=t=in:st=0:d=1.5,afade=t=out:st={:.2}:d=3[am]",
            (total_secs - 3.0).max(0.0)
        ));
        enc.arg("-filter_complex")
            .arg(fc)
            .args(["-map", "0:v", "-map", "[am]"])
            .arg("-t")
            .arg(format!("{total_secs:.3}"));
    }
    enc.arg(out).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = enc
        .spawn()
        .map_err(|e| format!("ffmpeg (Kodierer) startet nicht: {e}"))?;
    let stdin = child.stdin.take().ok_or("kein ffmpeg-Eingang")?;
    let mut stderr = child.stderr.take().ok_or("kein ffmpeg-Ausgang")?;
    let err_thread = thread::spawn(move || {
        let mut t = String::new();
        let _ = stderr.read_to_string(&mut t);
        t
    });

    // --- Schreiber: reicht fertige Frames an ffmpeg weiter
    let (ftx, frx) = sync_channel::<Vec<u8>>(6);
    let writer = thread::spawn(move || -> std::io::Result<()> {
        let mut stdin = stdin;
        for fr in frx {
            stdin.write_all(&fr)?;
        }
        Ok(())
    });

    // --- Lader: bereitet die Bilder der Reihe nach vor
    let (ltx, lrx) = sync_channel::<(usize, Result<Source, String>)>(2);
    {
        let (ff, items2, s2) = (ffmpeg.to_string(), items.to_vec(), s.clone());
        let slots: Vec<Slot> = tl.slots.clone();
        thread::spawn(move || {
            for sl in slots {
                let it = &items2[sl.item];
                let r = match it.kind {
                    Kind::Video => continue,
                    Kind::Intro => load_card(&ff, true, &s2),
                    Kind::Outro => load_card(&ff, false, &s2),
                    Kind::Image => load_still(&ff, it, &s2, sl.frames, sl.item, edge),
                };
                if ltx.send((sl.item, r)).is_err() {
                    return; // Hauptschleife ist fertig oder abgebrochen
                }
            }
        });
    }

    // --- Hauptschleife
    let mut active: Vec<(usize, Source)> = Vec::new(); // (Slot-Nummer, Quelle)
    let mut next = 0usize;
    let mut fa = vec![0u8; frame_len];
    let mut fb = vec![0u8; frame_len];
    let mut ta = vec![0u8; frame_len];
    let mut tb = vec![0u8; frame_len];
    let fade_frames = tl.trans.max(frames_of_fade(s));
    let mut failure: Option<String> = None;

    'frames: for f in 0..tl.total {
        if ctl.cancelled() {
            break;
        }
        while next < tl.slots.len() && tl.slots[next].start <= f {
            let sl = tl.slots[next];
            let it = &items[sl.item];
            let src = match it.kind {
                Kind::Image | Kind::Intro | Kind::Outro => loop {
                    match lrx.recv() {
                        Ok((i, r)) if i == sl.item => {
                            break r.unwrap_or_else(|e| {
                                ctl.say(format!("Als schwarzes Bild eingesetzt: {e}"));
                                Source::Blank
                            })
                        }
                        Ok(_) => continue,
                        Err(_) => {
                            failure = Some("Bildlader ist unerwartet beendet.".into());
                            break 'frames;
                        }
                    }
                },
                Kind::Video => match VideoDec::open(ffmpeg, it, s) {
                    Ok(v) => Source::Video(v),
                    Err(e) => {
                        ctl.say(format!("Video {} nicht lesbar: {e}", it.name));
                        Source::Blank
                    }
                },
            };
            active.push((next, src));
            next += 1;
        }
        active.retain(|(si, _)| {
            let sl = &tl.slots[*si];
            sl.start + sl.frames > f
        });

        let mut out = vec![0u8; frame_len];
        if active.len() >= 2 {
            let (si_a, si_b) = (active[0].0, active[1].0);
            let (sa, sb) = (tl.slots[si_a], tl.slots[si_b]);
            let local_a = f - sa.start;
            let local_b = f - sb.start;
            active[0].1.frame(local_a, &mut fa, s);
            active[1].1.frame(local_b, &mut fb, s);
            let t = frames::smooth((local_b as f32 + 0.5) / tl.trans.max(1) as f32);
            let card = |k: Kind| k == Kind::Intro || k == Kind::Outro;
            let soft_a = sa.pano || card(items[sa.item].kind);
            let soft_b = sb.pano || card(items[sb.item].kind);
            match model::transition_for(si_a, s, soft_a, soft_b) {
                Transition::Push => frames::push(&mut out, w, &fa, &fb, t),
                Transition::Wipe => frames::wipe(&mut out, w, &fa, &fb, t),
                Transition::Zoom => frames::zoom(&mut out, w, h, &fa, &fb, t, &mut ta, &mut tb),
                _ => frames::blend(&mut out, w, &fa, &fb, t),
            }
        } else if let Some((si, src)) = active.first_mut() {
            let local = f - tl.slots[*si].start;
            src.frame(local, &mut out, s);
        }

        // Am Anfang einblenden, am Ende ausblenden.
        if f < fade_frames {
            frames::fade(&mut out, w, (f as f32 + 1.0) / fade_frames as f32);
        } else if tl.total - f <= fade_frames {
            frames::fade(&mut out, w, (tl.total - f) as f32 / fade_frames as f32);
        }

        if ftx.send(out).is_err() {
            break; // Schreiber ist ausgefallen, Fehler unten
        }
        ctl.done.store(f + 1, Ordering::Relaxed);
    }

    drop(active);
    drop(lrx);
    drop(ftx);
    let write_res = writer.join().map_err(|_| "Schreibthread abgestürzt".to_string())?;
    if ctl.cancelled() {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|e| format!("ffmpeg: {e}"))?;
    let err_text = err_thread.join().unwrap_or_default();

    if ctl.cancelled() {
        return Err("Abgebrochen.".into());
    }
    if let Some(e) = failure {
        return Err(e);
    }
    if !status.success() || write_res.is_err() {
        let last = err_text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("unbekannter Fehler");
        return Err(format!("ffmpeg hat beim Kodieren abgebrochen: {last}"));
    }
    Ok(())
}

/// Mindestens 0,8 s Ein-/Ausblenden, auch wenn der Übergang kürzer oder 0 ist.
fn frames_of_fade(s: &Settings) -> u32 {
    model::frames_of(0.8, s.fps)
}
