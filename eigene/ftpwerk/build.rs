// Bettet unter Windows das Logo in die Programmdatei ein.
fn main() {
    werk_icon();
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
