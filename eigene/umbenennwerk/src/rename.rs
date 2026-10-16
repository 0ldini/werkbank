//! Kern: aus einem alten Dateinamen nach den Regeln einen neuen bilden.
//! Plattformunabhängig und ohne externe Bibliotheken – von der Oberfläche
//! und von der Kommandozeile genutzt, und damit gut testbar.

#[derive(Clone, Copy, PartialEq)]
pub enum Pos {
    Front,
    Back,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Case {
    Original,
    Upper,
    Lower,
    Title,
}

#[derive(Clone)]
pub struct Rules {
    pub replace_name: bool, // true: Basisname durch new_name ersetzen
    pub new_name: String,
    pub find: String, // Suchen & Ersetzen (nur wenn !replace_name)
    pub repl: String,
    pub prefix: String,
    pub suffix: String,
    pub case: Case,
    pub spaces_us: bool,     // Leerzeichen -> Unterstrich
    pub strip_special: bool, // Sonderzeichen entfernen
    pub add_date: bool,
    pub date_pos: Pos,
    pub numbering: bool,
    pub num_start: i64,
    pub num_step: i64,
    pub num_pad: usize,
    pub num_pos: Pos,
    pub num_sep: String,
    pub ext_lower: bool,
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            replace_name: false,
            new_name: String::new(),
            find: String::new(),
            repl: String::new(),
            prefix: String::new(),
            suffix: String::new(),
            case: Case::Original,
            spaces_us: false,
            strip_special: false,
            add_date: false,
            date_pos: Pos::Front,
            numbering: false,
            num_start: 1,
            num_step: 1,
            num_pad: 3,
            num_pos: Pos::Back,
            num_sep: "_".into(),
            ext_lower: false,
        }
    }
}

/// Zerlegt einen Dateinamen in (Stamm, Erweiterung-ohne-Punkt). Ein führender
/// Punkt (versteckte Datei ohne Endung) gilt nicht als Trenner.
pub fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i + 1..]),
        _ => (name, ""),
    }
}

fn apply_case(s: &str, c: Case) -> String {
    match c {
        Case::Original => s.to_string(),
        Case::Upper => s.to_uppercase(),
        Case::Lower => s.to_lowercase(),
        Case::Title => {
            let mut out = String::with_capacity(s.len());
            let mut start = true;
            for ch in s.chars() {
                if ch.is_alphanumeric() {
                    if start {
                        out.extend(ch.to_uppercase());
                    } else {
                        out.extend(ch.to_lowercase());
                    }
                    start = false;
                } else {
                    out.push(ch);
                    start = true;
                }
            }
            out
        }
    }
}

fn clean(s: &str, spaces_us: bool, strip_special: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch == ' ' && spaces_us {
            out.push('_');
        } else if strip_special
            && !(ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.' | ' '))
        {
            // Sonderzeichen entfernen (Buchstaben/Ziffern/._- bleiben)
        } else {
            out.push(ch);
        }
    }
    out
}

fn num_str(idx: usize, r: &Rules) -> String {
    let n = r.num_start + (idx as i64) * r.num_step;
    if n < 0 {
        return n.to_string();
    }
    let digits = n.to_string();
    if digits.len() >= r.num_pad {
        digits
    } else {
        format!("{}{}", "0".repeat(r.num_pad - digits.len()), digits)
    }
}

/// Baut den neuen vollständigen Dateinamen. `idx` ist der 0-basierte Rang in
/// der Liste; `date` ist der bereits formatierte Datumstext (oder None).
pub fn build(name: &str, idx: usize, date: Option<&str>, r: &Rules) -> String {
    let (stem, ext) = split_ext(name);

    // 1. Basis
    let mut s = if r.replace_name {
        r.new_name.clone()
    } else if !r.find.is_empty() {
        stem.replace(&r.find, &r.repl)
    } else {
        stem.to_string()
    };

    // 2. Gross/Klein, 3. bereinigen
    s = apply_case(&s, r.case);
    s = clean(&s, r.spaces_us, r.strip_special);

    // 4. Präfix / Suffix
    if !r.prefix.is_empty() || !r.suffix.is_empty() {
        s = format!("{}{}{}", r.prefix, s, r.suffix);
    }

    // 5. Datum
    if r.add_date {
        if let Some(d) = date {
            if !d.is_empty() {
                s = match r.date_pos {
                    Pos::Front => format!("{}{}{}", d, sep_if(&s), s),
                    Pos::Back => format!("{}{}{}", s, sep_if(&s), d),
                };
            }
        }
    }

    // 6. Nummerierung
    if r.numbering {
        let n = num_str(idx, r);
        s = match r.num_pos {
            Pos::Front => format!("{}{}{}", n, r.num_sep, s),
            Pos::Back => format!("{}{}{}", s, r.num_sep, n),
        };
    }

    // 7. Erweiterung
    let ext_out = if r.ext_lower { ext.to_lowercase() } else { ext.to_string() };
    if ext_out.is_empty() {
        if s.is_empty() {
            name.to_string() // niemals leer zurückgeben
        } else {
            s
        }
    } else {
        format!("{}.{}", s, ext_out)
    }
}

// Trenner "_" nur einfügen, wenn der bestehende Text nicht schon mit einem
// Trennzeichen endet (vermeidet doppelte Unterstriche).
fn sep_if(s: &str) -> &'static str {
    match s.chars().last() {
        Some('_') | Some('-') | Some(' ') => "",
        _ => "_",
    }
}

/// Markiert Zeilen mit leeren oder doppelten Zielnamen (Konflikte).
/// Rückgabe: für jede Position true, wenn sie ein Konflikt ist.
pub fn conflicts(new_names: &[String]) -> Vec<bool> {
    use std::collections::HashMap;
    let mut counts: HashMap<&str, u32> = HashMap::new();
    for n in new_names {
        *counts.entry(n.as_str()).or_insert(0) += 1;
    }
    new_names
        .iter()
        .map(|n| n.is_empty() || counts.get(n.as_str()).copied().unwrap_or(0) > 1)
        .collect()
}
