// Release-Build: werk-prebuild.sh legt ffmpeg nach vendor/ und setzt WERK_FFMPEG.
// Dann wird es in die Programmdatei eingebettet; ohne die Variable baut Diawerk wie bisher
// und sucht ffmpeg neben der .exe oder auf dem PATH.
fn main() {
    werk_icon();
    println!("cargo::rustc-check-cfg=cfg(embedded_ffmpeg)");
    println!("cargo::rerun-if-env-changed=WERK_FFMPEG");
    if let Ok(path) = std::env::var("WERK_FFMPEG") {
        let path = std::path::PathBuf::from(path);
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        if path.is_file() {
            println!("cargo::rerun-if-changed={}", path.display());
            println!("cargo::rustc-cfg=embedded_ffmpeg");
            println!("cargo::rustc-env=WERK_FFMPEG_PATH={}", path.display());
        } else {
            println!("cargo::warning=WERK_FFMPEG zeigt auf keine Datei: {}", path.display());
        }
    }
}

// Logo der Programmdatei (Explorer, Verknüpfungen) unter Windows.
#[cfg(windows)]
fn werk_icon() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo::rerun-if-changed=icon.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icon.ico");
        if let Err(e) = res.compile() {
            println!("cargo::warning=Programm-Logo nicht eingebettet: {e}");
        }
    }
}

#[cfg(not(windows))]
fn werk_icon() {}
