//! Ausschlussregeln. Nur std; Test: `rustc --edition 2021 --test src/regeln.rs`.
//!
//! - Ein Muster ohne `/` gilt für den Namen jeder Datei und jedes Ordners, egal wie tief:
//!   `*.tmp`, `node_modules`, `Cache`, `~$*`. Ein ausgeschlossener Ordner fällt mit allem
//!   darin weg.
//! - Ein Muster mit `/` gilt für den Pfad ab dem Quellordner: `Projekte/*/build`,
//!   `Spiele/**/*.log`. `*` passt nicht über `/`, `**` schon.
//! - `?` steht für genau ein Zeichen. Gross-/Kleinschreibung zählt nicht.
//! - Leere Zeilen und Zeilen mit `#` am Anfang zählen nicht.

#![allow(dead_code)]

#[derive(Clone, Debug, Default)]
pub struct Regeln {
    namen: Vec<Vec<char>>,
    pfade: Vec<Vec<char>>,
}

impl Regeln {
    pub fn neu<S: AsRef<str>>(zeilen: &[S]) -> Regeln {
        let mut r = Regeln::default();
        for z in zeilen {
            let z = z.as_ref().trim();
            if z.is_empty() || z.starts_with('#') {
                continue;
            }
            let z = z.replace('\\', "/");
            let z = z.trim_matches('/');
            if z.is_empty() {
                continue;
            }
            let m: Vec<char> = z.to_lowercase().chars().collect();
            if z.contains('/') {
                r.pfade.push(m);
            } else {
                r.namen.push(m);
            }
        }
        r
    }

    pub fn ist_leer(&self) -> bool {
        self.namen.is_empty() && self.pfade.is_empty()
    }

    /// `rel` ist der Pfad ab dem Quellordner mit `/` getrennt (ohne führenden `/`).
    pub fn ausgeschlossen(&self, rel: &str) -> bool {
        let rel_l: Vec<char> = rel.to_lowercase().chars().collect();
        let name_start = rel_l.iter().rposition(|c| *c == '/').map(|p| p + 1).unwrap_or(0);
        let name = rel_l.get(name_start..).unwrap_or(&[]);
        self.namen.iter().any(|m| passt(m, name)) || self.pfade.iter().any(|m| passt(m, &rel_l))
    }
}

/// Glob-Vergleich (dynamische Programmierung, keine Rekursion):
/// `*` = beliebig viele Zeichen ausser `/`, `**` = beliebig viele Zeichen, `?` = ein Zeichen ausser `/`.
pub fn passt(muster: &[char], text: &[char]) -> bool {
    #[derive(Clone, Copy, PartialEq)]
    enum T {
        Z(char),
        Ein,
        Stern,
        /// `**` – beliebig viel, auch über `/`.
        Doppel,
        /// `**/` – nichts oder beliebig viel, das mit `/` endet (a/**/b passt auf a/b).
        DoppelOrdner,
    }
    let mut teile = Vec::with_capacity(muster.len());
    let mut i = 0;
    while let Some(&c) = muster.get(i) {
        match c {
            '*' if muster.get(i + 1) == Some(&'*') => {
                if muster.get(i + 2) == Some(&'/') {
                    teile.push(T::DoppelOrdner);
                    i += 3;
                } else {
                    teile.push(T::Doppel);
                    i += 2;
                }
            }
            '*' => {
                teile.push(T::Stern);
                i += 1;
            }
            '?' => {
                teile.push(T::Ein);
                i += 1;
            }
            c => {
                teile.push(T::Z(c));
                i += 1;
            }
        }
    }
    let n = text.len();
    // dp[j] = die ersten j Zeichen passen auf die bisherigen Teile.
    let mut dp = vec![false; n + 1];
    dp[0] = true;
    for t in &teile {
        let mut neu = vec![false; n + 1];
        match *t {
            T::Z(c) => {
                for j in 0..n {
                    if dp[j] && text[j] == c {
                        neu[j + 1] = true;
                    }
                }
            }
            T::Ein => {
                for j in 0..n {
                    if dp[j] && text[j] != '/' {
                        neu[j + 1] = true;
                    }
                }
            }
            T::Stern => {
                for j in 0..=n {
                    neu[j] = dp[j] || (j > 0 && neu[j - 1] && text[j - 1] != '/');
                }
            }
            T::Doppel => {
                let mut an = false;
                for j in 0..=n {
                    an = an || dp[j];
                    neu[j] = an;
                }
            }
            T::DoppelOrdner => {
                // an = irgendein dp[k] mit k < j
                let mut an = false;
                for j in 0..=n {
                    neu[j] = dp[j] || (an && j > 0 && text[j - 1] == '/');
                    an = an || dp[j];
                }
            }
        }
        dp = neu;
    }
    dp[n]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(z: &[&str]) -> Regeln {
        Regeln::neu(z)
    }

    #[test]
    fn namen() {
        let x = r(&["*.tmp", "node_modules", "Cache", "~$*", "# Kommentar", ""]);
        assert!(x.ausgeschlossen("a.tmp"));
        assert!(x.ausgeschlossen("Ordner/b.TMP"));
        assert!(x.ausgeschlossen("web/node_modules"));
        assert!(x.ausgeschlossen("App/cache"));
        assert!(x.ausgeschlossen("Briefe/~$brief.docx"));
        assert!(!x.ausgeschlossen("a.tmpx"));
        assert!(!x.ausgeschlossen("Cachex"));
        assert!(!x.ausgeschlossen("tmp/datei.txt"));
        assert!(!x.ausgeschlossen("# Kommentar"));
    }

    #[test]
    fn pfade() {
        let x = r(&["Projekte/*/build", "Spiele/**/*.log", "\\Temp\\"]);
        assert!(x.ausgeschlossen("Projekte/eins/build"));
        assert!(!x.ausgeschlossen("Projekte/eins/zwei/build"));
        assert!(!x.ausgeschlossen("Anderes/eins/build"));
        assert!(x.ausgeschlossen("Spiele/a/b/c.log"));
        assert!(x.ausgeschlossen("Spiele/c.log"));
        assert!(!x.ausgeschlossen("Spiele/c.txt"));
        // «\Temp\» ohne inneren Schrägstrich wird zum Namensmuster «temp».
        assert!(x.ausgeschlossen("x/Temp"));
    }

    #[test]
    fn glob() {
        let p = |m: &str, t: &str| passt(&m.chars().collect::<Vec<_>>(), &t.chars().collect::<Vec<_>>());
        assert!(p("*", ""));
        assert!(p("*", "abc"));
        assert!(!p("*", "a/b"));
        assert!(p("**", "a/b"));
        assert!(p("a?c", "abc"));
        assert!(!p("a?c", "a/c"));
        assert!(p("*.t*p", "x.tmp"));
        assert!(p("a/**/b", "a/b"));
        assert!(p("a/**/b", "a/x/y/b"));
        assert!(!p("a/**/b", "ab"));
        assert!(p("**/x", "x"));
        assert!(p("**/x", "q/x"));
        assert!(p("a/**/b/**/c", "a/b/x/c"));
        assert!(p("a/**/b/**/c", "a/q/b/c"));
        assert!(!p("a/**/b", "a/xb"));
        // Viele Sterne – darf nicht explodieren.
        let lang: String = "a".repeat(2000);
        assert!(!p("*a*a*a*a*a*a*a*a*b", &lang));
    }
}
