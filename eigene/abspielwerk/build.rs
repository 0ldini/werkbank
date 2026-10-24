// Windows: Logo in die Programmdatei. macOS: libVLC liegt im Programm unter
// Contents/MacOS/lib; der Suchpfad @executable_path/lib lässt dessen @rpath-Verweise finden.
fn main() {
    werk_icon();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo::rustc-link-arg-bins=-Wl,-rpath,@executable_path/lib");
        println!("cargo::rustc-link-arg-bins=-Wl,-rpath,@executable_path");
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
