//! Aufnahme-Motor: verdrahtet die geprüften Bildfunktionen (frame.rs) mit
//! `windows-capture` (Windows.Graphics.Capture + Media Foundation → H.264/MP4).
//!
//! Ablauf: Je Bild liefert WGC einen BGRA-Puffer. Wir schneiden ihn optional auf
//! den gewählten Bereich zu, legen das Webcam-Overlay darüber, zeichnen eine
//! Klick-/Zeiger-Markierung an der Mausposition und schicken das Ergebnis an den
//! Encoder. Webcam läuft in einem eigenen Thread (nokhwa), der jeweils das
//! neueste Bild bereitstellt.
//!
//! Hinweis: Dieser native Teil lässt sich in dieser Umgebung nicht bauen
//! (crates.io gesperrt, Windows-only). Die Bild-/Datenlogik in frame.rs und
//! model.rs ist eigenständig getestet; hier ist die Verdrahtung gegen die
//! dokumentierte windows-capture-2.0.1-API.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::encoder::{
    AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

use crate::frame;
use crate::model::{Corner, Mode, Settings as Cfg};

type BoxErr = Box<dyn std::error::Error + Send + Sync>;

/// Ein Webcam-Einzelbild als BGRA.
pub struct WebFrame {
    pub data: Vec<u8>,
    pub w: u32,
    pub h: u32,
}

/// Von aussen sichtbarer Zustand (Dauer/Anzahl/Stopp).
#[derive(Clone)]
pub struct Shared {
    pub stop: Arc<AtomicBool>,
    pub frames: Arc<AtomicU64>,
    pub done: Arc<AtomicBool>,
    pub started: Instant,
}

/// Alles, was der Bild-Callback braucht (wird als Flags übergeben).
pub struct GrabFlags {
    pub out_w: u32,
    pub out_h: u32,
    pub fps: u32,
    pub bitrate: u32,
    pub system_audio: bool,
    /// Zuschnitt in Aufnahme-Pixeln (x, y, w, h), falls Bereich gewählt.
    pub crop: Option<(u32, u32, u32, u32)>,
    /// obere linke Ecke des Ergebnisbilds in Bildschirmkoordinaten (Mauspunkt-Mapping).
    pub origin: (i32, i32),
    pub webcam: bool,
    pub webcam_corner: Corner,
    pub webcam_percent: u32,
    pub click: bool,
    pub cursor_hl: bool,
    pub path: String,
    pub shared: Shared,
    pub webcam_frame: Arc<Mutex<Option<WebFrame>>>,
}

/// Griff auf eine laufende Aufnahme.
pub struct Recording {
    pub shared: Shared,
    control: Option<windows_capture::capture::CaptureControl<Grab, BoxErr>>,
}

impl Recording {
    pub fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(c) = self.control.take() {
            let _ = c.stop();
        }
    }
    pub fn is_finished(&self) -> bool {
        self.shared.done.load(Ordering::SeqCst)
            || self.control.as_ref().map(|c| c.is_finished()).unwrap_or(true)
    }
    pub fn seconds(&self) -> u64 {
        self.shared.started.elapsed().as_secs()
    }
}

/// Der Bild-Callback-Handler von windows-capture.
pub struct Grab {
    encoder: Option<VideoEncoder>,
    f: GrabFlags,
    stride: usize,
}

impl GraphicsCaptureApiHandler for Grab {
    type Flags = GrabFlags;
    type Error = BoxErr;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let f = ctx.flags;
        let video = VideoSettingsBuilder::new(f.out_w, f.out_h)
            .frame_rate(f.fps)
            .bitrate(f.bitrate);
        let audio = AudioSettingsBuilder::default().disabled(!f.system_audio);
        let encoder = VideoEncoder::new(
            video,
            audio,
            ContainerSettingsBuilder::default(),
            &f.path,
        )?;
        let stride = f.out_w as usize * 4;
        Ok(Grab {
            encoder: Some(encoder),
            f,
            stride,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        // Stoppwunsch: abschliessen und beenden.
        if self.f.shared.stop.load(Ordering::SeqCst) {
            if let Some(enc) = self.encoder.take() {
                enc.finish()?;
            }
            self.f.shared.done.store(true, Ordering::SeqCst);
            control.stop();
            return Ok(());
        }

        let fw = frame.width();
        let fh = frame.height();
        let mut fb = frame.buffer()?;
        let src = fb.as_raw_nopadding_buffer()?;
        let src_stride = fw as usize * 4;

        // 1) Zuschnitt oder Vollbild in einen eigenen Puffer.
        let mut buf = if let Some((cx, cy, cw, ch)) = self.f.crop {
            frame::crop(src, src_stride, fw, fh, cx, cy, cw, ch)
        } else {
            src.to_vec()
        };
        let (ow, oh) = (self.f.out_w, self.f.out_h);

        // 2) Webcam-Overlay.
        if self.f.webcam {
            if let Ok(guard) = self.f.webcam_frame.lock() {
                if let Some(wf) = guard.as_ref() {
                    let mut cam_w = frame::even(ow * self.f.webcam_percent / 100).max(2);
                    let mut cam_h = frame::even(cam_w * wf.h / wf.w.max(1)).max(2);
                    cam_w = cam_w.min(ow);
                    cam_h = cam_h.min(oh);
                    let scaled = frame::scale_nn(&wf.data, wf.w, wf.h, cam_w, cam_h);
                    let margin = (ow / 80).max(8) as i32;
                    let (px, py) =
                        frame::corner_pos(self.f.webcam_corner, ow, oh, cam_w, cam_h, margin);
                    frame::composite(&mut buf, self.stride, ow, oh, &scaled, cam_w, cam_h, px, py);
                    frame::draw_border(
                        &mut buf,
                        self.stride,
                        ow,
                        oh,
                        px,
                        py,
                        cam_w,
                        cam_h,
                        2,
                        [255, 255, 255, 255],
                    );
                }
            }
        }

        // 3) Klick-/Zeiger-Markierung an der Mausposition.
        if self.f.click || self.f.cursor_hl {
            if let Some((mx, my, pressed)) = cursor_in(self.f.origin, ow, oh) {
                let r = (ow / 55).max(10) as i32;
                if self.f.click && pressed {
                    // kräftige Markierung in Aufnahme-Rot (BGRA)
                    frame::draw_click_marker(&mut buf, self.stride, ow, oh, mx, my, r, [46, 52, 229, 255]);
                } else if self.f.cursor_hl {
                    // dezenter gelber Hof um den Zeiger
                    frame::draw_click_marker(&mut buf, self.stride, ow, oh, mx, my, r, [0, 200, 255, 255]);
                }
            }
        }

        // 4) An den Encoder (Zeitstempel in 100-ns-Einheiten seit Start).
        let ts = (self.f.shared.started.elapsed().as_nanos() / 100) as i64;
        if let Some(enc) = self.encoder.as_mut() {
            enc.send_frame_buffer(&buf, ts)?;
        }
        self.f.shared.frames.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        if let Some(enc) = self.encoder.take() {
            enc.finish()?;
        }
        self.f.shared.done.store(true, Ordering::SeqCst);
        Ok(())
    }
}

/// Mausposition abgefragt und in Ergebnisbild-Koordinaten umgerechnet.
/// Gibt (x, y, linke_Taste_gedrückt) zurück, falls der Zeiger im Bild liegt.
fn cursor_in(origin: (i32, i32), w: u32, h: u32) -> Option<(i32, i32, bool)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT::default();
    unsafe {
        if GetCursorPos(&mut p).is_err() {
            return None;
        }
    }
    let x = p.x - origin.0;
    let y = p.y - origin.1;
    if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
        return None;
    }
    let pressed = unsafe { (GetAsyncKeyState(0x01) as u16 & 0x8000) != 0 }; // VK_LBUTTON
    Some((x, y, pressed))
}

/// Startet eine Aufnahme nach den Einstellungen. `dims` ist die Ziel-Auflösung,
/// `path` die Zieldatei.
pub fn start(cfg: &Cfg, dims: (u32, u32), bitrate: u32, path: String) -> Result<Recording, BoxErr> {
    let shared = Shared {
        stop: Arc::new(AtomicBool::new(false)),
        frames: Arc::new(AtomicU64::new(0)),
        done: Arc::new(AtomicBool::new(false)),
        started: Instant::now(),
    };
    let webcam_frame: Arc<Mutex<Option<WebFrame>>> = Arc::new(Mutex::new(None));

    // Webcam-Thread (optional).
    if cfg.webcam {
        let wf = webcam_frame.clone();
        let stop = shared.stop.clone();
        std::thread::spawn(move || {
            if let Err(e) = run_webcam(wf, stop) {
                eprintln!("Webcam nicht verfügbar: {e}");
            }
        });
    }

    let flags = GrabFlags {
        out_w: dims.0,
        out_h: dims.1,
        fps: cfg.fps,
        bitrate,
        system_audio: cfg.system_audio,
        crop: match cfg.mode {
            Mode::Region => Some((
                cfg.region.0.max(0) as u32,
                cfg.region.1.max(0) as u32,
                cfg.region.2,
                cfg.region.3,
            )),
            _ => None,
        },
        origin: capture_origin(cfg),
        webcam: cfg.webcam,
        webcam_corner: cfg.webcam_corner,
        webcam_percent: cfg.webcam_percent,
        click: cfg.highlight_clicks,
        cursor_hl: cfg.highlight_cursor,
        path,
        shared: shared.clone(),
        webcam_frame,
    };

    let cursor = CursorCaptureSettings::WithCursor; // echter Systemzeiger
    let border = DrawBorderSettings::WithoutBorder; // kein gelber Rahmen (Win11)

    // Monitor oder Fenster als Aufnahmequelle.
    let control = match cfg.mode {
        Mode::Window => {
            let win = Window::from_contains_name(&cfg.window_title)?;
            let settings = Settings::new(
                win,
                cursor,
                border,
                SecondaryWindowSettings::Default,
                MinimumUpdateIntervalSettings::Default,
                DirtyRegionSettings::Default,
                ColorFormat::Bgra8,
                flags,
            );
            Grab::start_free_threaded(settings)?
        }
        _ => {
            let mon = monitor_at(cfg.monitor_index)?;
            let settings = Settings::new(
                mon,
                cursor,
                border,
                SecondaryWindowSettings::Default,
                MinimumUpdateIntervalSettings::Default,
                DirtyRegionSettings::Default,
                ColorFormat::Bgra8,
                flags,
            );
            Grab::start_free_threaded(settings)?
        }
    };

    Ok(Recording {
        shared,
        control: Some(control),
    })
}

/// Obere linke Ecke des aufgenommenen Bereichs in Bildschirmkoordinaten.
/// (Primärmonitor liegt bei 0,0; Mehrmonitor-Versatz für Zweitschirme folgt
/// in Etappe 2.)
fn capture_origin(cfg: &Cfg) -> (i32, i32) {
    match cfg.mode {
        Mode::Region => (cfg.region.0, cfg.region.1),
        _ => (0, 0),
    }
}

/// Auflösung der gewählten Quelle (gerade Kanten, H.264-tauglich).
pub fn source_dims(cfg: &Cfg) -> Result<(u32, u32), BoxErr> {
    use crate::model::even;
    match cfg.mode {
        Mode::Region => Ok((even(cfg.region.2).max(2), even(cfg.region.3).max(2))),
        Mode::Window => {
            // Grösse selbst über Win32 bestimmen (unabhängig von windows-capture-Typen).
            if let Some((w, h)) = window_client_size(&cfg.window_title) {
                Ok((even(w).max(2), even(h).max(2)))
            } else {
                // Rückfall: primärer Monitor als Leinwand.
                let m = monitor_at(0)?;
                Ok((even(m.width()?).max(2), even(m.height()?).max(2)))
            }
        }
        Mode::Fullscreen => {
            let m = monitor_at(cfg.monitor_index)?;
            Ok((even(m.width()?).max(2), even(m.height()?).max(2)))
        }
    }
}

/// Liefert den Monitor am gegebenen Index (0-basiert); fällt auf den primären
/// zurück.
pub fn monitor_at(index: usize) -> Result<Monitor, BoxErr> {
    let all = Monitor::enumerate()?;
    if let Some(m) = all.into_iter().nth(index) {
        Ok(m)
    } else {
        Ok(Monitor::primary()?)
    }
}

/// Client-Grösse des ersten sichtbaren Fensters, dessen Titel `needle` enthält
/// (ohne Gross-/Kleinschreibung). Per EnumWindows + GetClientRect.
fn window_client_size(needle: &str) -> Option<(u32, u32)> {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT, TRUE};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClientRect, GetWindowTextLengthW, GetWindowTextW, IsWindowVisible,
    };

    struct Ctx {
        needle: String,
        found: Option<(u32, u32)>,
    }

    unsafe extern "system" fn cb(hwnd: HWND, lp: LPARAM) -> BOOL {
        let ctx = &mut *(lp.0 as *mut Ctx);
        if !IsWindowVisible(hwnd).as_bool() {
            return TRUE;
        }
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return TRUE;
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, &mut buf);
        let title = String::from_utf16_lossy(&buf[..n as usize]);
        if title.to_lowercase().contains(&ctx.needle) {
            let mut r = RECT::default();
            if GetClientRect(hwnd, &mut r).is_ok() {
                let w = (r.right - r.left).max(0) as u32;
                let h = (r.bottom - r.top).max(0) as u32;
                if w > 0 && h > 0 {
                    ctx.found = Some((w, h));
                    return BOOL(0); // gefunden -> stoppen
                }
            }
        }
        TRUE
    }

    let mut ctx = Ctx {
        needle: needle.to_lowercase(),
        found: None,
    };
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut ctx as *mut _ as isize));
    }
    ctx.found
}

/// Namen und Auflösung aller Monitore für das Dropdown.
pub fn list_monitors() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(all) = Monitor::enumerate() {
        for (i, m) in all.iter().enumerate() {
            let w = m.width().unwrap_or(0);
            let h = m.height().unwrap_or(0);
            out.push(format!("Monitor {} · {} × {}", i + 1, w, h));
        }
    }
    if out.is_empty() {
        out.push("Monitor 1".into());
    }
    out
}

/// Titel aller sichtbaren Fenster für das Dropdown.
pub fn list_windows() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(all) = Window::enumerate() {
        for w in all {
            if let Ok(t) = w.title() {
                if !t.trim().is_empty() {
                    out.push(t);
                }
            }
        }
    }
    out
}

/// Webcam-Schleife: holt Bilder und legt jeweils das neueste als BGRA ab.
fn run_webcam(slot: Arc<Mutex<Option<WebFrame>>>, stop: Arc<AtomicBool>) -> Result<(), BoxErr> {
    use nokhwa::pixel_format::RgbFormat;
    use nokhwa::utils::{CameraIndex, RequestedFormat, RequestedFormatType};
    use nokhwa::Camera;

    let fmt = RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestFrameRate);
    let mut cam = Camera::new(CameraIndex::Index(0), fmt)?;
    cam.open_stream()?;
    while !stop.load(Ordering::SeqCst) {
        let buffer = cam.frame()?;
        let img = buffer.decode_image::<RgbFormat>()?;
        let (w, h) = (img.width(), img.height());
        let rgb = img.into_raw();
        let n = (w * h) as usize;
        let mut bgra = vec![0u8; n * 4];
        for i in 0..n {
            bgra[i * 4] = rgb[i * 3 + 2]; // B
            bgra[i * 4 + 1] = rgb[i * 3 + 1]; // G
            bgra[i * 4 + 2] = rgb[i * 3]; // R
            bgra[i * 4 + 3] = 255;
        }
        if let Ok(mut g) = slot.lock() {
            *g = Some(WebFrame { data: bgra, w, h });
        }
    }
    Ok(())
}
