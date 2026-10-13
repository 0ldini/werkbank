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
