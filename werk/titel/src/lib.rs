//! Werkbank: färbt die Titelleiste der eigenen Fenster in der Farbe des Programm-Logos,
//! hell oder dunkel wie im Launcher eingestellt. Nur unter Windows (ab 11); sonst ohne Wirkung.
//!
//! Eigene Kiste, weil die Programme selbst `unsafe` verbieten, der Windows-Aufruf es aber braucht.
//! Ein Hintergrund-Thread sucht die Fenster des eigenen Prozesses und färbt jedes einmal.

/// Hell (`false`) oder dunkel (`true`) wie im Launcher: `WERKBANK_THEME` oder `thema.txt`.
pub fn launcher_dunkel() -> Option<bool> {
    let wert = std::env::var("WERKBANK_THEME").ok().or_else(|| {
        let base = if cfg!(windows) {
            std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Werkbank").join("config")
        } else if cfg!(target_os = "macos") {
            std::path::PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/Werkbank")
        } else {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(std::path::PathBuf::from)
                .filter(|p| p.is_absolute())
                .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?
                .join("werkbank")
        };
        std::fs::read_to_string(base.join("thema.txt")).ok()
    })?;
    parse(&wert)
}

fn parse(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "dunkel" | "dark" => Some(true),
        "hell" | "light" => Some(false),
        _ => None,
    }
}

/// Farben als COLORREF (0x00BBGGRR): Leiste, Schrift, Rand.
pub fn farben(logo: [u8; 3], dunkel: bool) -> [u32; 3] {
    let mix = |base: [u8; 3], a: f32| -> u32 {
        let m = |c: u8, b: u8| (b as f32 + (c as f32 - b as f32) * a).round().clamp(0.0, 255.0) as u32;
        m(logo[0], base[0]) | m(logo[1], base[1]) << 8 | m(logo[2], base[2]) << 16
    };
    let (base, text) = if dunkel { ([0x17, 0x18, 0x1C], 0x00F2_EEEC) } else { ([0xFF, 0xFF, 0xFF], 0x001C_1817) };
    [mix(base, if dunkel { 0.22 } else { 0.14 }), text, mix(base, 0.4)]
}

/// Startet das Färben im Hintergrund. Gefahrlos mehrfach aufrufbar; ohne Windows ohne Wirkung.
pub fn start(logo: [u8; 3]) {
    #[cfg(windows)]
    win::start(logo);
    #[cfg(not(windows))]
    let _ = logo;
}

#[cfg(windows)]
mod win {
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicBool, Ordering};
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR};
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible};

    static LAEUFT: AtomicBool = AtomicBool::new(false);

    pub fn start(logo: [u8; 3]) {
        if LAEUFT.swap(true, Ordering::SeqCst) {
            return;
        }
        // Ohne Einstellung im Launcher: hell (wie die Programme ohne Launcher meist starten).
        let [leiste, text, rand] = super::farben(logo, super::launcher_dunkel().unwrap_or(false));
        let _ = std::thread::Builder::new().name("werk-titel".into()).spawn(move || {
            let mut erledigt: HashSet<usize> = HashSet::new();
            let mut runde = 0u32;
            loop {
                for hwnd in eigene_fenster() {
                    if erledigt.insert(hwnd as usize) {
                        for (attr, wert) in [(DWMWA_CAPTION_COLOR, leiste), (DWMWA_TEXT_COLOR, text), (DWMWA_BORDER_COLOR, rand)] {
                            // SAFETY: Fenster des eigenen Prozesses; der Wert ist ein COLORREF (u32, 4 Byte)
                            // und lebt bis nach dem Aufruf. Fehler (z. B. Windows 10) sind ohne Folgen.
                            unsafe {
                                DwmSetWindowAttribute(hwnd, attr as u32, &wert as *const u32 as *const core::ffi::c_void, 4);
                            }
                        }
                    }
                }
                runde = runde.saturating_add(1);
                // Am Anfang oft (das Hauptfenster soll gleich farbig sein), danach selten für neue Fenster.
                let pause = if runde < 120 { 100 } else { 1500 };
                std::thread::sleep(std::time::Duration::from_millis(pause));
            }
        });
    }

    fn eigene_fenster() -> Vec<HWND> {
        unsafe extern "system" fn sammeln(hwnd: HWND, lparam: LPARAM) -> BOOL {
            // SAFETY: lparam ist der Zeiger auf `liste` aus eigene_fenster, gültig während EnumWindows läuft.
            let liste = unsafe { &mut *(lparam as *mut Vec<HWND>) };
            let mut pid = 0u32;
            // SAFETY: hwnd kommt von EnumWindows; pid ist eine gültige Ausgabe-Variable.
            unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
            // SAFETY: wie oben.
            if pid == unsafe { GetCurrentProcessId() } && unsafe { IsWindowVisible(hwnd) } != 0 {
                liste.push(hwnd);
            }
            1
        }
        let mut liste: Vec<HWND> = Vec::new();
        // SAFETY: Rückruf und Zeiger bleiben für die Dauer des Aufrufs gültig.
        unsafe { EnumWindows(Some(sammeln), &mut liste as *mut Vec<HWND> as LPARAM) };
        liste
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn thema_lesen() {
        assert_eq!(super::parse(" Dunkel\n"), Some(true));
        assert_eq!(super::parse("light"), Some(false));
        assert_eq!(super::parse("bunt"), None);
    }

    #[test]
    fn farben_mischen() {
        // Rot im hellen Modus: kräftiges Rot mit 14 % auf Weiss, Schrift dunkel.
        let [leiste, text, _] = super::farben([0xC9, 0x36, 0x36], false);
        assert_eq!(leiste & 0xFF, 0xF7);
        assert_eq!(text, 0x001C_1817);
        let [leiste, ..] = super::farben([0xFF, 0xFF, 0xFF], true);
        assert!(leiste & 0xFF > 0x17);
    }
}
