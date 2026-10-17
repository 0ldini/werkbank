//! ffmpeg finden: neben der eigenen .exe, eingebaut (Release-Build) oder auf dem PATH.

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Im Release-Build steckt ffmpeg in der Programmdatei (siehe build.rs, werk-prebuild.sh).
#[cfg(embedded_ffmpeg)]
const EMBEDDED_FFMPEG: &[u8] = include_bytes!(env!("WERK_FFMPEG_PATH"));

/// true, wenn diese Programmdatei ffmpeg mitbringt.
pub const HAS_EMBEDDED_FFMPEG: bool = cfg!(embedded_ffmpeg);

/// Legt das eingebaute ffmpeg einmalig im Benutzerordner ab und gibt den Pfad zurück.
/// Der Dateiname enthält Programmversion und Grösse, damit ein Update eine neue Kopie anlegt.
#[cfg(embedded_ffmpeg)]
fn embedded_ffmpeg() -> Option<String> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
        .join("Diawerk");
    let name = format!(
        "ffmpeg-{}-{}{}",
        env!("CARGO_PKG_VERSION"),
        EMBEDDED_FFMPEG.len(),
        if cfg!(windows) { ".exe" } else { "" }
    );
    let path = base.join(name);
    let ready = std::fs::metadata(&path).map(|m| m.len() == EMBEDDED_FFMPEG.len() as u64).unwrap_or(false);
    if !ready {
        std::fs::create_dir_all(&base).ok()?;
        // Erst unter anderem Namen schreiben, dann umbenennen: nie eine halbe Datei benutzen.
        let tmp = path.with_extension("teil");
        std::fs::write(&tmp, EMBEDDED_FFMPEG).ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).ok()?;
        }
        std::fs::rename(&tmp, &path).ok()?;
    }
    Some(path.display().to_string())
}

/// Findet ffmpeg: neben der eigenen .exe, eingebaut oder auf dem PATH.
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
    #[cfg(embedded_ffmpeg)]
    if let Some(p) = embedded_ffmpeg() {
        return Some(p);
    }
    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-version").stdout(Stdio::null()).stderr(Stdio::null());
    crate::model::hide_window(&mut cmd);
    let ok = cmd.status().map(|s| s.success()).unwrap_or(false);
    if ok {
        Some("ffmpeg".into())
    } else {
        None
    }
}

/// Vorschlag für die Ausgabedatei: Ordner „Videos" des Benutzers.
pub fn default_output() -> PathBuf {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from);
    match home {
        Some(h) => {
            let v = h.join("Videos");
            (if v.is_dir() { v } else { h }).join("Diashow.mp4")
        }
        None => PathBuf::from("Diashow.mp4"),
    }
}
