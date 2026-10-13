// Windows: Programm-Icon in die .exe einbetten (Explorer, Taskleiste).
fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icons/werkbank.ico");
        if let Err(e) = res.compile() {
            println!("cargo::warning=Icon nicht eingebettet: {e}");
        }
    }
    println!("cargo::rerun-if-changed=icons/werkbank.ico");
}
