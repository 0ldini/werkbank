//! Kern: einen Ordner rekursiv durchgehen und einen Baum mit aggregierten
//! Grössen bauen; dazu Auswertungen (grösste Dateien, Dateitypen, CSV) und die
//! Grössen-Formatierung. Plattformunabhängig, nur Standardbibliothek – damit
//! gut testbar. Symbolischen Verknüpfungen wird nicht gefolgt (keine Schleifen).

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub struct Node {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub children: Vec<Node>,
}

/// Durchläuft `root`. Zählt Dateien/Ordner/Bytes mit (für die Fortschrittsanzeige)
/// und bricht ab, sobald `cancel` gesetzt ist (Rückgabe None).
pub fn scan(
    root: &Path,
    files: &AtomicU64,
    dirs: &AtomicU64,
    bytes: &AtomicU64,
    cancel: &AtomicBool,
) -> Option<Node> {
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    let name = root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());
    let meta = std::fs::symlink_metadata(root).ok()?;
    let ft = meta.file_type();

    if ft.is_symlink() {
        // Verknüpfung nicht verfolgen; mit eigener (kleiner) Grösse zählen.
        let sz = meta.len();
        files.fetch_add(1, Ordering::Relaxed);
        bytes.fetch_add(sz, Ordering::Relaxed);
        return Some(Node { name, is_dir: false, size: sz, children: vec![] });
    }

    if ft.is_dir() {
        dirs.fetch_add(1, Ordering::Relaxed);
        let mut children = Vec::new();
        let mut total = 0u64;
        if let Ok(rd) = std::fs::read_dir(root) {
            for e in rd.flatten() {
                if cancel.load(Ordering::Relaxed) {
                    return None;
                }
                if let Some(ch) = scan(&e.path(), files, dirs, bytes, cancel) {
                    total += ch.size;
                    children.push(ch);
                }
            }
        }
        children.sort_by(|a, b| b.size.cmp(&a.size));
        Some(Node { name, is_dir: true, size: total, children })
    } else {
        let sz = meta.len();
        files.fetch_add(1, Ordering::Relaxed);
        bytes.fetch_add(sz, Ordering::Relaxed);
        Some(Node { name, is_dir: false, size: sz, children: vec![] })
    }
}

/// Grösse wie im Explorer (1024er-Einheiten, Komma als Dezimaltrennzeichen).
pub fn fmt_size(b: u64) -> String {
    if b < 1024 {
        return format!("{} B", b);
    }
    let units = ["KB", "MB", "GB", "TB", "PB"];
    let mut v = b as f64 / 1024.0;
    let mut u = 0;
    while v >= 1000.0 && u < units.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    let s = if v >= 99.95 {
        format!("{:.0}", v)
    } else {
        let t = format!("{:.1}", v);
        t.strip_suffix(".0").map(str::to_string).unwrap_or(t)
    };
    format!("{} {}", s.replace('.', ","), units[u])
}

/// Tausender mit Apostroph (Schweizer Schreibweise), z. B. 184'302.
pub fn group_thousands(n: u64) -> String {
    let s = n.to_string();
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in b.iter().enumerate() {
        if i > 0 && (b.len() - i) % 3 == 0 {
            out.push('\u{2019}');
        }
        out.push(*c as char);
    }
    out
}

fn ext_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => name[i + 1..].to_lowercase(),
        _ => String::from("(ohne)"),
    }
}

fn walk_files<'a>(node: &'a Node, prefix: &str, f: &mut dyn FnMut(&str, &'a Node)) {
    for ch in &node.children {
        let path = if prefix.is_empty() {
            ch.name.clone()
        } else {
            format!("{}/{}", prefix, ch.name)
        };
        if ch.is_dir {
            walk_files(ch, &path, f);
        } else {
            f(&path, ch);
        }
    }
    // auch die Wurzel selbst, falls sie eine Datei ist
    if prefix.is_empty() && !node.is_dir {
        f(&node.name, node);
    }
}

/// Die n grössten Dateien (Pfad relativ zur Wurzel, Grösse).
pub fn largest_files(root: &Node, n: usize) -> Vec<(String, u64)> {
    let mut v: Vec<(String, u64)> = Vec::new();
    walk_files(root, "", &mut |path, node| v.push((path.to_string(), node.size)));
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v.truncate(n);
    v
}

/// Grösse je Dateiendung, absteigend.
pub fn ext_sizes(root: &Node) -> Vec<(String, u64, u64)> {
    let mut map: HashMap<String, (u64, u64)> = HashMap::new(); // ext -> (bytes, count)
    walk_files(root, "", &mut |_p, node| {
        let e = map.entry(ext_of(&node.name)).or_insert((0, 0));
        e.0 += node.size;
        e.1 += 1;
    });
    let mut v: Vec<(String, u64, u64)> = map.into_iter().map(|(k, (b, c))| (k, b, c)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v
}

/// CSV (Pfad;Bytes;Typ) des ganzen Baums.
pub fn to_csv(root: &Node) -> String {
    let mut out = String::from("Pfad;Bytes;Typ\n");
    fn rec(node: &Node, prefix: &str, out: &mut String) {
        let path = if prefix.is_empty() {
            node.name.clone()
        } else {
            format!("{}/{}", prefix, node.name)
        };
        let typ = if node.is_dir { "Ordner" } else { "Datei" };
        out.push_str(&format!("{};{};{}\n", path.replace(';', ","), node.size, typ));
        for ch in &node.children {
            rec(ch, &path, out);
        }
    }
    rec(root, "", &mut out);
    out
}
