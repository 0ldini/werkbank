// Testlauf ohne Oberfläche: baut eine Diashow aus Dateien/Ordnern.
//   rustc --edition 2021 -O testlauf/cli.rs -o diawerk-cli
//   ./diawerk-cli [Optionen] <Dateien oder Ordner…> -o diashow.mp4
// Zum Prüfen des Motors (Panorama-Schwenk, Übergänge, Videos) ohne egui.
#![allow(dead_code)]
#[path = "../src/frames.rs"]
mod frames;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/render.rs"]
mod render;

use std::path::PathBuf;
use std::sync::atomic::Ordering;

fn main() {
    let mut s = model::Settings::default();
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut out = PathBuf::from("diashow.mp4");
    let mut ffmpeg = "ffmpeg".to_string();
    let mut spread = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = |n: &str| args.next().unwrap_or_else(|| panic!("{n} braucht einen Wert"));
        match a.as_str() {
            "-o" => out = PathBuf::from(val("-o")),
            "--ffmpeg" => ffmpeg = val("--ffmpeg"),
            "--draft" => s.quality = model::Quality::Draft,
            "--best" => s.quality = model::Quality::Best,
            "--size" => {
                let v = val("--size");
                let (w, h) = v.split_once('x').expect("--size 1920x1080");
                s.width = w.parse().unwrap();
                s.height = h.parse().unwrap();
            }
            "--fps" => s.fps = val("--fps").parse().unwrap(),
            "--still" => s.still_secs = val("--still").parse().unwrap(),
            "--trans" => s.trans_secs = val("--trans").parse().unwrap(),
            "--zoom" => s.zoom_pct = val("--zoom").parse().unwrap(),
            "--pano-ratio" => s.pano_ratio = val("--pano-ratio").parse().unwrap(),
            "--pano-speed" => s.pano_speed = val("--pano-speed").parse().unwrap(),
            "--pano-min" => s.pano_min_secs = val("--pano-min").parse().unwrap(),
            "--video-max" => s.video_max_secs = val("--video-max").parse().unwrap(),
            "--music" => s.music.push(PathBuf::from(val("--music"))),
            "--portrait" => s.credits.portrait = Some(PathBuf::from(val("--portrait"))),
            "--no-credits" => {
                s.credits.intro = false;
                s.credits.outro = false;
            }
            "--card-secs" => s.credits.card_secs = val("--card-secs").parse().unwrap(),
            "--format" => {
                s.format = match val("--format").as_str() {
                    "mkv" => model::OutFmt::Mkv,
                    "mov" => model::OutFmt::Mov,
                    "webm" => model::OutFmt::Webm,
                    _ => model::OutFmt::Mp4,
                }
            }
            "--no-crop" => s.crop_to_fill = false,
            "--spread" => spread = true,
            "--transition" => {
                s.transition = match val("--transition").as_str() {
                    "fade" => model::Transition::Crossfade,
                    "push" => model::Transition::Push,
                    "wipe" => model::Transition::Wipe,
                    "zoom" => model::Transition::Zoom,
                    _ => model::Transition::Mix,
                }
            }
            p => paths.push(PathBuf::from(p)),
        }
    }
    let mut files = Vec::new();
    for p in paths {
        if p.is_dir() {
            files.extend(model::collect_files(&p, true));
        } else {
            files.push(p);
        }
    }
    let mut items: Vec<model::Item> = Vec::new();
    for f in &files {
        match model::probe(&ffmpeg, f) {
            Ok(i) => items.push(i),
            Err(e) => eprintln!("übersprungen {}: {e}", f.display()),
        }
    }
    if spread {
        items = model::spread_videos(items);
    }
    let items = model::with_credits(&items, &s);
    let tl = model::build_timeline(&items, &s);
    for sl in &tl.slots {
        let it = &items[sl.item];
        eprintln!(
            "{:>3} {:<22} {:>5}x{:<5} o{} {:?} {:>6.2}s ab {:>6.2}s{}",
            sl.item + 1,
            it.name,
            it.w,
            it.h,
            it.orient,
            it.kind,
            sl.frames as f32 / s.fps as f32,
            sl.start as f32 / s.fps as f32,
            if sl.pano { "  PANORAMA" } else { "" }
        );
    }
    eprintln!("Gesamt: {} Frames = {:.2}s", tl.total, tl.total_secs(s.fps));
    let t0 = std::time::Instant::now();
    let ctl = render::start(ffmpeg, items, s, out.clone());
    while !ctl.finished.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(500));
        eprint!(
            "\r{} / {}   ",
            ctl.done.load(Ordering::Relaxed),
            ctl.total.load(Ordering::Relaxed)
        );
    }
    eprintln!();
    for l in ctl.log.lock().unwrap().iter() {
        eprintln!("Hinweis: {l}");
    }
    let res = ctl.result.lock().unwrap().take();
    match res {
        Some(Ok(())) => eprintln!("Fertig in {:.1}s: {}", t0.elapsed().as_secs_f32(), out.display()),
        Some(Err(e)) => {
            eprintln!("FEHLER: {e}");
            std::process::exit(1);
        }
        None => {}
    }
}
