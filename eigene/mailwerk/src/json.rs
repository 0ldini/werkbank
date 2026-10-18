//! Kleines JSON (Lesen und Schreiben) ohne Abhängigkeiten. Nur Standardbibliothek.
//! Dient für JMAP und für die lokale Speicherung.

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

static NULL: Json = Json::Null;

// ---------------------------------------------------------------- Bauen

pub fn obj(items: Vec<(&str, Json)>) -> Json {
    Json::Obj(items.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}
pub fn arr(items: Vec<Json>) -> Json {
    Json::Arr(items)
}
pub fn s(x: &str) -> Json {
    Json::Str(x.to_string())
}
pub fn n(x: i64) -> Json {
    Json::Num(x as f64)
}
pub fn b(x: bool) -> Json {
    Json::Bool(x)
}

// ---------------------------------------------------------------- Lesen

impl Json {
    /// Feld eines Objekts; fehlt es (oder ist es kein Objekt), kommt `Null`.
    pub fn get(&self, key: &str) -> &Json {
        match self {
            Json::Obj(v) => v.iter().find(|(k, _)| k == key).map(|(_, v)| v).unwrap_or(&NULL),
            _ => &NULL,
        }
    }

    pub fn at(&self, i: usize) -> &Json {
        match self {
            Json::Arr(v) => v.get(i).unwrap_or(&NULL),
            _ => &NULL,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Text oder leer.
    pub fn str_or_empty(&self) -> String {
        self.as_str().unwrap_or("").to_string()
    }

    pub fn as_bool(&self) -> bool {
        matches!(self, Json::Bool(true))
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Num(n) if n.is_finite() => Some(*n as i64),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> &[Json] {
        match self {
            Json::Arr(v) => v,
            _ => &[],
        }
    }

    pub fn entries(&self) -> &[(String, Json)] {
        match self {
            Json::Obj(v) => v,
            _ => &[],
        }
    }

    pub fn parse(src: &str) -> Result<Json, String> {
        let mut p = P { b: src.as_bytes(), i: 0, src };
        p.ws();
        let v = p.value(0)?;
        p.ws();
        if p.i != p.b.len() {
            return Err(format!("JSON: Überzähliger Text bei Position {}", p.i));
        }
        Ok(v)
    }

    pub fn to_string(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    out.push_str(&format!("{}", *n as i64));
                } else if n.is_finite() {
                    out.push_str(&format!("{n}"));
                } else {
                    out.push_str("null");
                }
            }
            Json::Str(s) => write_str(s, out),
            Json::Arr(v) => {
                out.push('[');
                for (i, x) in v.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    x.write(out);
                }
                out.push(']');
            }
            Json::Obj(v) => {
                out.push('{');
                for (i, (k, x)) in v.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_str(k, out);
                    out.push(':');
                    x.write(out);
                }
                out.push('}');
            }
        }
    }
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
    src: &'a str,
}

impl<'a> P<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn err<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("JSON: {what} bei Position {}", self.i))
    }

    fn lit(&mut self, word: &str, v: Json) -> Result<Json, String> {
        if self.src[self.i..].starts_with(word) {
            self.i += word.len();
            Ok(v)
        } else {
            self.err("Unbekanntes Wort")
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > 64 {
            return self.err("Zu tief verschachtelt");
        }
        match self.b.get(self.i) {
            None => self.err("Unerwartetes Ende"),
            Some(b'{') => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Json::Obj(items));
                }
                loop {
                    self.ws();
                    if self.b.get(self.i) != Some(&b'"') {
                        return self.err("Schlüssel erwartet");
                    }
                    let k = self.string()?;
                    self.ws();
                    if self.b.get(self.i) != Some(&b':') {
                        return self.err("«:» erwartet");
                    }
                    self.i += 1;
                    self.ws();
                    let v = self.value(depth + 1)?;
                    items.push((k, v));
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Json::Obj(items));
                        }
                        _ => return self.err("«,» oder «}» erwartet"),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Json::Arr(items));
                }
                loop {
                    self.ws();
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Json::Arr(items));
                        }
                        _ => return self.err("«,» oder «]» erwartet"),
                    }
                }
            }
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.lit("true", Json::Bool(true)),
            Some(b'f') => self.lit("false", Json::Bool(false)),
            Some(b'n') => self.lit("null", Json::Null),
            Some(_) => {
                let start = self.i;
                while self.i < self.b.len() && matches!(self.b[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.i += 1;
                }
                if start == self.i {
                    return self.err("Unerwartetes Zeichen");
                }
                match self.src[start..self.i].parse::<f64>() {
                    Ok(n) => Ok(Json::Num(n)),
                    Err(_) => self.err("Ungültige Zahl"),
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        if self.i + 4 > self.b.len() {
            return self.err("Ungültiges \\u");
        }
        let h = &self.src[self.i..self.i + 4];
        self.i += 4;
        u32::from_str_radix(h, 16).or_else(|_| self.err("Ungültiges \\u"))
    }

    fn string(&mut self) -> Result<String, String> {
        self.i += 1; // «"»
        let mut out = String::new();
        loop {
            let start = self.i;
            while self.i < self.b.len() && self.b[self.i] != b'"' && self.b[self.i] != b'\\' {
                self.i += 1;
            }
            out.push_str(&self.src[start..self.i]);
            match self.b.get(self.i) {
                None => return self.err("Text nicht beendet"),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(_) => {
                    self.i += 1;
                    let e = match self.b.get(self.i) {
                        Some(&c) => c,
                        None => return self.err("Text nicht beendet"),
                    };
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let cp = if (0xD800..0xDC00).contains(&hi) && self.src[self.i..].starts_with("\\u") {
                                self.i += 2;
                                let lo = self.hex4()?;
                                0x10000 + ((hi - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3ff)
                            } else {
                                hi
                            };
                            out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                        }
                        _ => return self.err("Ungültiges Escape"),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lesen() {
        let j = Json::parse(r#" {"a":[1,2.5,-3e2,true,false,null,"x\n\"y\" ä 😀"],"b":{"c":"d"},"e":[]} "#).unwrap();
        assert_eq!(j.get("a").at(0).as_i64(), Some(1));
        assert_eq!(j.get("a").at(1), &Json::Num(2.5));
        assert_eq!(j.get("a").at(2).as_i64(), Some(-300));
        assert!(j.get("a").at(3).as_bool() && !j.get("a").at(4).as_bool());
        assert!(j.get("a").at(5).is_null());
        assert_eq!(j.get("a").at(6).as_str(), Some("x\n\"y\" ä 😀"));
        assert_eq!(j.get("b").get("c").as_str(), Some("d"));
        assert!(j.get("e").as_arr().is_empty());
        assert!(j.get("fehlt").get("auch").is_null());
        assert_eq!(j.get("b").entries().len(), 1);
    }

    #[test]
    fn schreiben_und_rueckweg() {
        let j = obj(vec![
            ("text", s("Grüezi \"Welt\"\n\t\u{1}")),
            ("n", n(42)),
            ("f", Json::Num(1.5)),
            ("l", arr(vec![b(true), Json::Null, s("x")])),
            ("o", obj(vec![])),
        ]);
        let t = j.to_string();
        assert_eq!(t, "{\"text\":\"Grüezi \\\"Welt\\\"\\n\\t\\u0001\",\"n\":42,\"f\":1.5,\"l\":[true,null,\"x\"],\"o\":{}}");
        assert_eq!(Json::parse(&t).unwrap(), j);
    }

    #[test]
    fn fehler() {
        assert!(Json::parse("{").is_err());
        assert!(Json::parse("[1,]").is_err());
        assert!(Json::parse("{\"a\" 1}").is_err());
        assert!(Json::parse("\"offen").is_err());
        assert!(Json::parse("1 2").is_err());
        assert!(Json::parse("").is_err());
        assert!(Json::parse(&"[".repeat(100)).is_err());
    }
}
