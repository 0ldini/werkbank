//! Dateitypen: welche Endungen mit welchem Werkbank-Programm geöffnet werden.
//!
//! Die Zuordnung zeigt immer auf `werkbank.exe --open <id> <datei>`, nie auf die Programmdatei
//! selbst, denn die liegt in einem Ordner mit Versionsnummer und wandert bei jedem Update.
//! Unter Windows landen die Einträge im Benutzerprofil (HKCU\Software\Classes), ohne
//! Administratorrechte. Windows selbst entscheidet, welches Programm Standard ist; die
//! Werkbank erscheint danach unter «Öffnen mit» und in den Windows-Einstellungen «Standard-Apps».

use crate::core::{App, Manifest};
use anyhow::Result;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Gewünschte Zuordnung: Programm-Id → Endungen (mit Punkt, klein).
pub type Choice = BTreeMap<String, Vec<String>>;

/// Alle Endungen, die ein Programm laut Programmliste öffnen kann.
pub fn offered(app: &App) -> Vec<String> {
    app.types.iter().map(|t| t.trim().to_lowercase()).filter(|t| t.starts_with('.')).collect()
}

/// Standardvorschlag: jedes Programm mit allen seinen Typen.
pub fn all(manifest: &Manifest) -> Choice {
    manifest.app.iter().filter(|a| !a.types.is_empty()).map(|a| (a.id.clone(), offered(a))).collect()
}

pub fn supported() -> bool {
    cfg!(windows)
}

/// Schreibt die Zuordnung ins System: entfernt, was nicht mehr gewählt ist, trägt den Rest ein.
pub fn apply(manifest: &Manifest, previous: &Choice, wanted: &Choice) -> Result<()> {
    #[cfg(windows)]
    {
        windows::apply(manifest, previous, wanted)
    }
    #[cfg(not(windows))]
    {
        let _ = (manifest, previous, wanted);
        anyhow::bail!("Dateitypen lassen sich nur unter Windows zuordnen")
    }
}

/// Öffnet die Windows-Einstellung «Standard-Apps» für die Werkbank.
pub fn open_system_settings() {
    let _ = open::that("ms-settings:defaultapps?registeredAppUser=Werkbank");
}

/// Icon-Datei (.ico) für ein Programm im Datenordner; wird aus dem eingebauten PNG erzeugt.
pub fn icon_file(app: &App) -> Option<PathBuf> {
    let name = app.icon.as_deref().unwrap_or(&app.id);
    let bytes = crate::icons::ICONS.iter().find(|(n, _)| *n == name).map(|(_, b)| *b)?;
    let dir = crate::core::data_dir().join("icons");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{}.ico", app.id));
    if !path.exists() {
        let img = image::load_from_memory(bytes).ok()?.resize(256, 256, image::imageops::FilterType::Lanczos3);
        img.save_with_format(&path, image::ImageFormat::Ico).ok()?;
    }
    Some(path)
}

#[cfg(windows)]
mod windows {
    use super::*;
    use winreg::enums::*;
    use winreg::RegKey;

    fn progid(id: &str) -> String {
        format!("Werkbank.{id}")
    }

    pub fn apply(manifest: &Manifest, previous: &Choice, wanted: &Choice) -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let classes = hkcu.create_subkey("Software\\Classes")?.0;
        let exe = std::env::current_exe()?;
        let exe = exe.display().to_string();

        // Entfernen, was nicht mehr gewählt ist.
        for (id, exts) in previous {
            let keep = wanted.get(id).cloned().unwrap_or_default();
            for ext in exts {
                if !keep.contains(ext) {
                    if let Ok(k) = classes.open_subkey_with_flags(format!("{ext}\\OpenWithProgIds"), KEY_SET_VALUE) {
                        let _ = k.delete_value(progid(id));
                    }
                }
            }
            if keep.is_empty() {
                let _ = classes.delete_subkey_all(progid(id));
            }
        }

        // Eintragen, was gewählt ist.
        let caps = hkcu.create_subkey("Software\\Werkbank\\Capabilities")?.0;
        caps.set_value("ApplicationName", &"Werkbank")?;
        caps.set_value("ApplicationDescription", &"Werkbank: Kreativ-Programme unter einem Dach")?;
        let assoc = caps.create_subkey("FileAssociations")?.0;
        for v in assoc.enum_values().flatten().map(|(n, _)| n).collect::<Vec<_>>() {
            let _ = assoc.delete_value(v);
        }
        for app in &manifest.app {
            let Some(exts) = wanted.get(&app.id) else { continue };
            if exts.is_empty() {
                continue;
            }
            let pid = progid(&app.id);
            let k = classes.create_subkey(&pid)?.0;
            k.set_value("", &format!("{}-Datei", app.name))?;
            k.set_value("FriendlyTypeName", &format!("{}-Datei", app.name))?;
            if let Some(icon) = icon_file(app) {
                k.create_subkey("DefaultIcon")?.0.set_value("", &format!("{},0", icon.display()))?;
            }
            let cmd = k.create_subkey("shell\\open\\command")?.0;
            cmd.set_value("", &format!("\"{exe}\" --open {} \"%1\"", app.id))?;
            k.create_subkey("shell\\open")?.0.set_value("", &format!("Mit {} öffnen", app.name))?;
            for ext in exts {
                classes.create_subkey(format!("{ext}\\OpenWithProgIds"))?.0.set_value(&pid, &"")?;
                assoc.set_value(ext, &pid)?;
            }
        }
        hkcu.create_subkey("Software\\RegisteredApplications")?
            .0
            .set_value("Werkbank", &"Software\\Werkbank\\Capabilities")?;
        notify();
        Ok(())
    }

    /// Sagt Explorer und Co., dass sich Zuordnungen geändert haben.
    fn notify() {
        #[link(name = "shell32")]
        extern "system" {
            fn SHChangeNotify(event: i32, flags: u32, item1: *const std::ffi::c_void, item2: *const std::ffi::c_void);
        }
        const SHCNE_ASSOCCHANGED: i32 = 0x0800_0000;
        // Sicherheit: reine Benachrichtigung ohne Zeiger, wie von Microsoft für diesen Fall beschrieben.
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, 0, std::ptr::null(), std::ptr::null()) };
    }
}
