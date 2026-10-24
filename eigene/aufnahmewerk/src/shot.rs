//! Bildschirmfoto: ein einzelnes Bild über dieselbe Schnittstelle wie die
//! Videoaufnahme (`windows-capture`, Windows.Graphics.Capture), dazu
//! Win32-Helfer (Monitor-Rechtecke, aktives Fenster, eigene Fenster kurz
//! ausblenden), Speichern als PNG/JPG, Zwischenablage und Ordner öffnen.
//!
//! Die fehleranfällige Rechnerei (Zuschnitt, Zuordnung Fenster → Monitor) sitzt
//! in schnapp.rs und ist dort getestet. Dieser Teil lässt sich – wie record.rs –
//! hier nicht bauen (Windows-only, crates.io gesperrt).

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND;
use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};

use crate::schnapp::{self, Img};
use crate::sprache::t;

type BoxErr = Box<dyn std::error::Error + Send + Sync>;

/// Was fotografiert wird.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShotMode {
    /// Ganzer Monitor.
    Screen,
    /// Bereich mit der Maus aufziehen (auf eingefrorenem Bild).
    Region,
    /// Ein gewähltes Fenster (aus der Fensterliste), auch wenn es verdeckt ist.
    Window,
    /// Das Fenster, das beim Auslösen vorne ist.
    ActiveWindow,
}

/// Ergebnis einer Aufnahme.
pub struct Captured {
    pub img: Img,
    pub mode: ShotMode,
    /// Monitor-Rechteck (x, y, w, h) in Bildschirmpixeln, falls bekannt.
    pub mon_rect: Option<(i32, i32, u32, u32)>,
}

/// Ablage, in die der Hintergrund-Thread sein Ergebnis legt.
pub type Slot = Arc<Mutex<Option<Result<Captured, String>>>>;

// ---------- ein Einzelbild über windows-capture ----------

struct OneFrame {
    slot: Arc<Mutex<Option<Img>>>,
}

impl GraphicsCaptureApiHandler for OneFrame {
    type Flags = Arc<Mutex<Option<Img>>>;
    type Error = BoxErr;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(OneFrame { slot: ctx.flags })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let fw = frame.width();
        let fh = frame.height();
        #[allow(unused_mut)]
        let mut fb = frame.buffer()?;
        let mut unpadded = Vec::new();
        let src = fb.as_nopadding_buffer(&mut unpadded);
        if let Some(img) = Img::from_bgra(fw, fh, src) {
            if let Ok(mut g) = self.slot.lock() {
                *g = Some(img);
            }
        }
        // Ein Bild genügt.
        control.stop();
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Ein Einzelbild von einer Aufnahmequelle (Monitor oder Fenster) holen.
/// Als Makro, damit beide Quelltypen denselben Ablauf nutzen, ohne dass hier
/// die genauen Typgrenzen von `Settings::new` nachgebaut werden müssen.
macro_rules! grab_one {
    ($item:expr) => {{
        let slot: Arc<Mutex<Option<Img>>> = Arc::new(Mutex::new(None));
        let settings = Settings::new(
            $item,
            CursorCaptureSettings::WithoutCursor,
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Default,
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            slot.clone(),
        );
        let control = OneFrame::start_free_threaded(settings)?;
        let t0 = Instant::now();
        loop {
            let have = slot.lock().map(|g| g.is_some()).unwrap_or(false);
            if have || control.is_finished() || t0.elapsed() > Duration::from_secs(4) {
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        let _ = control.stop();
        let img = slot.lock().ok().and_then(|mut g| g.take());
        img
    }};
}

/// Fotografiert den Monitor am Index (Reihenfolge wie im Dropdown).
pub fn grab_monitor(index: usize) -> Result<Img, BoxErr> {
    let mon = crate::record::monitor_at(index)?;
    let img: Option<Img> = grab_one!(mon);
    img.ok_or_else(|| t("Kein Bild vom Bildschirm erhalten.").into())
}

/// Fotografiert das Fenster, dessen Titel `title` enthält – direkt über
/// Windows.Graphics.Capture, also auch, wenn es von anderen Fenstern verdeckt
/// ist. Ein minimiertes Fenster wird vorher wiederhergestellt (minimiert
/// liefert Windows kein Bild). Der unsichtbare Rand wird abgeschnitten.
pub fn grab_window(title: &str) -> Result<Img, BoxErr> {
    if title.trim().is_empty() {
        return Err(t("Bitte zuerst ein Fenster wählen.").into());
    }
    let hwnd = find_window(title);
    if let Some(h) = hwnd {
        if restore_if_minimized(h) {
            std::thread::sleep(Duration::from_millis(450));
        }
    }
    let win = windows_capture::window::Window::from_contains_name(title)
        .map_err(|_| BoxErr::from(t("Das Fenster ist nicht mehr offen.")))?;
    let img: Option<Img> = grab_one!(win);
    let img = img.ok_or_else(|| BoxErr::from(t("Kein Bild vom Fenster erhalten.")))?;
    if let Some((outer, vis)) = hwnd.and_then(window_rects) {
        if let Some(r) = schnapp::border_trim(outer, vis, img.w, img.h) {
            if let Some(c) = schnapp::crop(&img, r) {
                return Ok(c);
            }
        }
    }
    Ok(img)
}

/// Startet ein Bildschirmfoto im Hintergrund: eigene Fenster ausblenden,
/// Verzögerung abwarten, aufnehmen, Fenster wieder zeigen. Das Ergebnis
/// landet in der zurückgegebenen Ablage; danach wird neu gezeichnet.
pub fn spawn(mode: ShotMode, monitor: usize, window_title: String, delay_s: u32, ctx: egui::Context) -> Slot {
    let slot: Slot = Arc::new(Mutex::new(None));
    let out = slot.clone();
    std::thread::spawn(move || {
        let hidden = hide_own_windows();
        // Kurz warten, bis das Ausblenden auf dem Bildschirm sichtbar ist.
        std::thread::sleep(Duration::from_millis(350 + delay_s as u64 * 1000));
        let res = take(mode, monitor, &window_title).map_err(|e| e.to_string());
        show_windows(&hidden);
        if let Ok(mut g) = out.lock() {
            *g = Some(res);
        }
        ctx.request_repaint();
    });
    slot
}

fn take(mode: ShotMode, monitor: usize, window_title: &str) -> Result<Captured, BoxErr> {
    let rects = monitor_rects();
    match mode {
        ShotMode::Screen | ShotMode::Region => {
            let img = grab_monitor(monitor)?;
            Ok(Captured { img, mode, mon_rect: rects.get(monitor).copied() })
        }
        ShotMode::Window => {
            let img = grab_window(window_title)?;
            Ok(Captured { img, mode, mon_rect: None })
        }
        ShotMode::ActiveWindow => {
            let win = foreground_rect().ok_or_else(|| BoxErr::from(t("Kein aktives Fenster gefunden.")))?;
            let cx = win.0 / 2 + win.2 / 2;
            let cy = win.1 / 2 + win.3 / 2;
            let idx = schnapp::pick_monitor(&rects, cx, cy).unwrap_or(0);
            let img = grab_monitor(idx)?;
            let mon = rects.get(idx).copied().unwrap_or((0, 0, img.w, img.h));
            let r = schnapp::window_crop(win, mon, img.w, img.h)
                .ok_or_else(|| BoxErr::from(t("Das Fenster liegt ausserhalb des Bildschirms.")))?;
            let img = schnapp::crop(&img, r)
                .ok_or_else(|| BoxErr::from(t("Das Fenster liegt ausserhalb des Bildschirms.")))?;
            Ok(Captured { img, mode, mon_rect: Some(mon) })
        }
    }
}

// ---------- Win32-Helfer ----------

/// Rechtecke (x, y, w, h) aller Monitore in Bildschirmpixeln, in der
/// Reihenfolge von EnumDisplayMonitors (dieselbe, die windows-capture nutzt).
pub fn monitor_rects() -> Vec<(i32, i32, u32, u32)> {
    use windows::Win32::Foundation::{BOOL, LPARAM, RECT, TRUE};
    use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};

    unsafe extern "system" fn cb(_m: HMONITOR, _dc: HDC, r: *mut RECT, lp: LPARAM) -> BOOL {
        let out = &mut *(lp.0 as *mut Vec<(i32, i32, u32, u32)>);
        if let Some(r) = r.as_ref() {
            let w = (r.right - r.left).max(0) as u32;
            let h = (r.bottom - r.top).max(0) as u32;
            out.push((r.left, r.top, w, h));
        }
        TRUE
    }

    let mut out: Vec<(i32, i32, u32, u32)> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(HDC::default(), None, Some(cb), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

/// Rechteck (links, oben, rechts, unten) des Vordergrund-Fensters ohne den
/// unsichtbaren Rand (DWM). `None`, wenn es keines gibt, es zu diesem Programm
/// gehört oder minimiert ist.
fn foreground_rect() -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, IsIconic,
    };
    let h = unsafe {
        let h = GetForegroundWindow();
        if h == HWND::default() {
            return None;
        }
        let mut pid: u32 = 0;
        let _ = GetWindowThreadProcessId(h, Some(&mut pid as *mut u32));
        if pid == std::process::id() {
            return None;
        }
        if IsIconic(h).as_bool() {
            return None;
        }
        h
    };
    window_rects(h).map(|(_, vis)| vis)
}

/// Ganzes Fensterrechteck (mit unsichtbarem Rand) und sichtbarer Teil (DWM),
/// beide als (links, oben, rechts, unten). Fehlt der DWM-Wert, gilt das ganze
/// Rechteck als sichtbar.
fn window_rects(h: HWND) -> Option<((i32, i32, i32, i32), (i32, i32, i32, i32))> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let ok = |r: &RECT| r.right > r.left && r.bottom > r.top;
    unsafe {
        let mut outer = RECT::default();
        let have_outer = GetWindowRect(h, &mut outer).is_ok() && ok(&outer);
        let mut vis = RECT::default();
        let have_vis = DwmGetWindowAttribute(
            h,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut vis as *mut RECT as *mut std::ffi::c_void,
            std::mem::size_of::<RECT>() as u32,
        )
        .is_ok()
            && ok(&vis);
        let o = (outer.left, outer.top, outer.right, outer.bottom);
        let v = (vis.left, vis.top, vis.right, vis.bottom);
        match (have_outer, have_vis) {
            (true, true) => Some((o, v)),
            (true, false) => Some((o, o)),
            (false, true) => Some((v, v)),
            (false, false) => None,
        }
    }
}

/// Erstes sichtbares Fenster eines anderen Programms, dessen Titel `needle`
/// enthält (ohne Gross-/Kleinschreibung).
fn find_window(needle: &str) -> Option<HWND> {
    use windows::Win32::Foundation::{BOOL, LPARAM, TRUE};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
    };

    struct Ctx {
        needle: String,
        found: Option<HWND>,
    }

    unsafe extern "system" fn cb(hwnd: HWND, lp: LPARAM) -> BOOL {
        let ctx = &mut *(lp.0 as *mut Ctx);
        if !IsWindowVisible(hwnd).as_bool() {
            return TRUE;
        }
        let mut pid: u32 = 0;
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid as *mut u32));
        if pid == std::process::id() {
            return TRUE;
        }
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 || len > 4096 {
            return TRUE;
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, &mut buf).max(0) as usize;
        let title = String::from_utf16_lossy(buf.get(..n).unwrap_or(&[]));
        if title.to_lowercase().contains(&ctx.needle) {
            ctx.found = Some(hwnd);
            return BOOL(0); // gefunden -> stoppen
        }
        TRUE
    }

    let mut ctx = Ctx { needle: needle.to_lowercase(), found: None };
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut ctx as *mut _ as isize));
    }
    ctx.found
}

/// Stellt ein minimiertes Fenster wieder her; `true`, wenn es minimiert war.
fn restore_if_minimized(h: HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{IsIconic, ShowWindow, SW_RESTORE};
    unsafe {
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
            true
        } else {
            false
        }
    }
}

/// Blendet alle sichtbaren Fenster dieses Programms aus (Hauptfenster,
/// Editor) und gibt sie zurück, damit sie danach wieder gezeigt werden.
fn hide_own_windows() -> Vec<HWND> {
    use windows::Win32::Foundation::{BOOL, LPARAM, TRUE};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible, ShowWindowAsync, SW_HIDE,
    };

    unsafe extern "system" fn cb(hwnd: HWND, lp: LPARAM) -> BOOL {
        let out = &mut *(lp.0 as *mut Vec<HWND>);
        if IsWindowVisible(hwnd).as_bool() {
            let mut pid: u32 = 0;
            let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid as *mut u32));
            if pid == std::process::id() {
                out.push(hwnd);
            }
        }
        TRUE
    }

    let mut own: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut own as *mut _ as isize));
        for h in &own {
            let _ = ShowWindowAsync(*h, SW_HIDE);
        }
    }
    own
}

/// Zeigt die zuvor ausgeblendeten Fenster wieder (minimierte werden
/// wiederhergestellt, damit Editor und Bereichswahl laufen).
fn show_windows(list: &[HWND]) {
    use windows::Win32::UI::WindowsAndMessaging::{
        IsIconic, SetForegroundWindow, ShowWindowAsync, SW_RESTORE, SW_SHOW,
    };
    unsafe {
        for h in list {
            let cmd = if IsIconic(*h).as_bool() { SW_RESTORE } else { SW_SHOW };
            let _ = ShowWindowAsync(*h, cmd);
        }
        if let Some(h) = list.first() {
            let _ = SetForegroundWindow(*h);
        }
    }
}

// ---------- Speichern, Zwischenablage, Ordner ----------

/// Speichert das Bild als PNG oder JPG (Qualität 92).
pub fn save(img: &Img, path: &Path, jpg: bool) -> Result<(), String> {
    let buf = image::RgbaImage::from_raw(img.w, img.h, img.px.clone())
        .ok_or_else(|| t("Bilddaten ungültig.").to_string())?;
    if jpg {
        use std::io::Write;
        let rgb = image::DynamicImage::ImageRgba8(buf).to_rgb8();
        let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
        let mut w = std::io::BufWriter::new(f);
        {
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, 92);
            enc.encode_image(&rgb).map_err(|e| e.to_string())?;
        }
        w.flush().map_err(|e| e.to_string())
    } else {
        buf.save_with_format(path, image::ImageFormat::Png).map_err(|e| e.to_string())
    }
}

/// Legt das Bild in die Zwischenablage.
pub fn copy_to_clipboard(img: &Img) -> Result<(), String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    cb.set_image(arboard::ImageData {
        width: img.w as usize,
        height: img.h as usize,
        bytes: std::borrow::Cow::Borrowed(&img.px),
    })
    .map_err(|e| e.to_string())
}

/// Öffnet den Ordner im Explorer.
pub fn open_folder(dir: &str) {
    let _ = std::process::Command::new("explorer").arg(dir).spawn();
}

/// Öffnet den Explorer und markiert die Datei.
pub fn show_in_folder(path: &str) {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("explorer")
        .raw_arg(format!("/select,\"{}\"", path))
        .spawn();
}
