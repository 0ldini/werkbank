//! Datei speichern – nativ über einen Dialog, im Browser als Download.

#[cfg(not(target_arch = "wasm32"))]
pub fn save_bytes(name: &str, bytes: &[u8]) {
    if let Some(path) = rfd::FileDialog::new().set_file_name(name).save_file() {
        let _ = std::fs::write(path, bytes);
    }
}

#[cfg(target_arch = "wasm32")]
pub fn save_bytes(name: &str, bytes: &[u8]) {
    use wasm_bindgen::JsCast;
    let array = js_sys::Uint8Array::from(bytes);
    let parts = js_sys::Array::new();
    parts.push(&array.buffer());
    let blob = match web_sys::Blob::new_with_u8_array_sequence(&parts) {
        Ok(b) => b,
        Err(_) => return,
    };
    let url = match web_sys::Url::create_object_url_with_blob(&blob) {
        Ok(u) => u,
        Err(_) => return,
    };
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        if let Ok(el) = doc.create_element("a") {
            if let Ok(a) = el.dyn_into::<web_sys::HtmlAnchorElement>() {
                a.set_href(&url);
                a.set_download(name);
                a.click();
            }
        }
    }
    let _ = web_sys::Url::revoke_object_url(&url);
}

/// Datei zum Anhängen wählen: (Name, Inhalt). Nur Desktop; im Browser kommt `None`.
#[cfg(not(target_arch = "wasm32"))]
pub fn pick_files() -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    if let Some(paths) = rfd::FileDialog::new().pick_files() {
        for p in paths {
            if let Ok(data) = std::fs::read(&p) {
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "datei".into());
                out.push((name, data));
            }
        }
    }
    out
}

#[cfg(target_arch = "wasm32")]
pub fn pick_files() -> Vec<(String, Vec<u8>)> {
    Vec::new()
}
