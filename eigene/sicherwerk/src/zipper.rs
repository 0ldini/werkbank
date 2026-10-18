//! ZIP schreiben und lesen (nur «gespeichert», ohne Kompression – schnell, und jede Datei
//! lässt sich mit jedem Entpacker öffnen). ZIP64 für Dateien/Archive über 4 GB und mehr als
//! 65535 Einträge. Namen in UTF-8. Nur std; Test: `rustc --edition 2021 --test src/zipper.rs`.

#![allow(dead_code)]

use std::io::{self, Read, Seek, SeekFrom, Write};

use crate::pruefsumme::Crc32;

// Für den Einzeltest mit rustc: dann ist diese Datei die Wurzel und braucht das Modul selbst.
#[cfg(test)]
#[allow(unused)]
#[path = "pruefsumme.rs"]
mod pruefsumme;

const SIG_LOKAL: u32 = 0x0403_4b50;
const SIG_ZENTRAL: u32 = 0x0201_4b50;
const SIG_ENDE: u32 = 0x0605_4b50;
const SIG_ENDE64: u32 = 0x0606_4b50;
const SIG_ZEIGER64: u32 = 0x0706_4b50;
/// Platzhalter im lokalen Kopf, wird bei grossen Dateien zum ZIP64-Feld (unbekannte
/// Kennungen überspringen Entpacker).
const ID_PLATZ: u16 = 0xD935;
const ID_ZIP64: u16 = 0x0001;
const ID_ZEIT: u16 = 0x5455;
const FLAG_UTF8: u16 = 0x0800;
const MAX32: u64 = 0xFFFF_FFFF;

fn fehler(text: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, text.to_string())
}

struct Zentral {
    name: Vec<u8>,
    crc: u32,
    groesse: u64,
    offset: u64,
    zeit: u16,
    datum: u16,
    unix: i64,
    ordner: bool,
}

pub struct ZipSchreiber<W: Write + Seek> {
    w: W,
    pos: u64,
    zentral: Vec<Zentral>,
    /// Ab dieser Grösse ZIP64 (Tests setzen sie klein).
    grenze: u64,
}

impl<W: Write + Seek> ZipSchreiber<W> {
    pub fn neu(mut w: W) -> io::Result<Self> {
        let pos = w.stream_position()?;
        Ok(ZipSchreiber { w, pos, zentral: Vec::new(), grenze: MAX32 })
    }

    #[cfg(test)]
    fn mit_grenze(mut self, g: u64) -> Self {
        self.grenze = g;
        self
    }

    pub fn anzahl(&self) -> usize {
        self.zentral.len()
    }

    fn name_bytes(name: &str) -> io::Result<Vec<u8>> {
        let b = name.as_bytes().to_vec();
        if b.is_empty() || b.len() > 0xFFFF {
            return Err(fehler("ungültiger Name im ZIP"));
        }
        Ok(b)
    }

    fn lokal_kopf(&mut self, name: &[u8], zeit: u16, datum: u16) -> io::Result<()> {
        let mut k = Vec::with_capacity(30 + name.len() + 20);
        k.extend_from_slice(&SIG_LOKAL.to_le_bytes());
        k.extend_from_slice(&20u16.to_le_bytes());
        k.extend_from_slice(&FLAG_UTF8.to_le_bytes());
        k.extend_from_slice(&0u16.to_le_bytes()); // gespeichert
        k.extend_from_slice(&zeit.to_le_bytes());
        k.extend_from_slice(&datum.to_le_bytes());
        k.extend_from_slice(&[0u8; 12]); // crc, grösse, grösse – später
        k.extend_from_slice(&(name.len() as u16).to_le_bytes());
        k.extend_from_slice(&20u16.to_le_bytes());
        k.extend_from_slice(name);
        k.extend_from_slice(&ID_PLATZ.to_le_bytes());
        k.extend_from_slice(&16u16.to_le_bytes());
        k.extend_from_slice(&[0u8; 16]);
        self.w.write_all(&k)?;
        self.pos = self.pos.saturating_add(k.len() as u64);
        Ok(())
    }

    /// Fügt eine Datei hinzu. `dos` = (Zeit, Datum) im DOS-Format, `unix` = Änderungszeit.
    /// `weiter(n)` je geschriebenem Block; `false` bricht ab. Bei einem Fehler wird der
    /// Eintrag verworfen (das Archiv bleibt brauchbar, wenn danach `fertig` folgt).
    pub fn datei<R: Read>(
        &mut self,
        name: &str,
        r: &mut R,
        dos: (u16, u16),
        unix: i64,
        mut weiter: impl FnMut(u64) -> bool,
    ) -> io::Result<(u64, u32)> {
        let nb = Self::name_bytes(name)?;
        let offset = self.pos;
        let ergebnis = (|| -> io::Result<(u64, u32)> {
            self.lokal_kopf(&nb, dos.0, dos.1)?;
            let mut crc = Crc32::neu();
            let mut buf = vec![0u8; 256 * 1024];
            let mut n_ges = 0u64;
            loop {
                let n = match r.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                };
                let teil = buf.get(..n).unwrap_or(&[]);
                crc.update(teil);
                self.w.write_all(teil)?;
                n_ges = n_ges.saturating_add(n as u64);
                self.pos = self.pos.saturating_add(n as u64);
                if !weiter(n as u64) {
                    return Err(io::Error::new(io::ErrorKind::Interrupted, "abgebrochen"));
                }
            }
            let crc = crc.wert();
            // Kopf nachtragen.
            let gross = n_ges >= self.grenze;
            self.w.seek(SeekFrom::Start(offset + 4))?;
            self.w.write_all(&(if gross { 45u16 } else { 20u16 }).to_le_bytes())?;
            self.w.seek(SeekFrom::Start(offset + 14))?;
            let g32 = if gross { MAX32 as u32 } else { n_ges as u32 };
            let mut f = Vec::with_capacity(12);
            f.extend_from_slice(&crc.to_le_bytes());
            f.extend_from_slice(&g32.to_le_bytes());
            f.extend_from_slice(&g32.to_le_bytes());
            self.w.write_all(&f)?;
            if gross {
                self.w.seek(SeekFrom::Start(offset + 30 + nb.len() as u64))?;
                let mut x = Vec::with_capacity(20);
                x.extend_from_slice(&ID_ZIP64.to_le_bytes());
                x.extend_from_slice(&16u16.to_le_bytes());
                x.extend_from_slice(&n_ges.to_le_bytes());
                x.extend_from_slice(&n_ges.to_le_bytes());
                self.w.write_all(&x)?;
            }
            self.w.seek(SeekFrom::Start(self.pos))?;
            Ok((n_ges, crc))
        })();
        match ergebnis {
            Ok((n, crc)) => {
                self.zentral.push(Zentral {
                    name: nb,
                    crc,
                    groesse: n,
                    offset,
                    zeit: dos.0,
                    datum: dos.1,
                    unix,
                    ordner: false,
                });
                Ok((n, crc))
            }
            Err(e) => {
                // Zurück an den Anfang dieses Eintrags; der Rest wird überschrieben.
                self.pos = offset;
                let _ = self.w.seek(SeekFrom::Start(offset));
                Err(e)
            }
        }
    }

    /// Leerer Ordner (Name ohne abschliessenden `/`).
    pub fn ordner(&mut self, name: &str, dos: (u16, u16), unix: i64) -> io::Result<()> {
        let n = format!("{}/", name.trim_end_matches('/'));
        let nb = Self::name_bytes(&n)?;
        let offset = self.pos;
        self.lokal_kopf(&nb, dos.0, dos.1)?;
        self.zentral.push(Zentral { name: nb, crc: 0, groesse: 0, offset, zeit: dos.0, datum: dos.1, unix, ordner: true });
        Ok(())
    }

    /// Schreibt das Inhaltsverzeichnis. Liefert den Schreiber und die Endposition (bei
    /// einer Datei danach `set_len(ende)`, falls ein verworfener Eintrag Reste hinterliess).
    pub fn fertig(mut self) -> io::Result<(W, u64)> {
        let cd_start = self.pos;
        let mut cd = Vec::new();
        for z in &self.zentral {
            let g_gross = z.groesse >= self.grenze;
            let o_gross = z.offset >= self.grenze;
            let mut extra = Vec::new();
            if g_gross || o_gross {
                let mut d = Vec::new();
                if g_gross {
                    d.extend_from_slice(&z.groesse.to_le_bytes());
                    d.extend_from_slice(&z.groesse.to_le_bytes());
                }
                if o_gross {
                    d.extend_from_slice(&z.offset.to_le_bytes());
                }
                extra.extend_from_slice(&ID_ZIP64.to_le_bytes());
                extra.extend_from_slice(&(d.len() as u16).to_le_bytes());
                extra.extend_from_slice(&d);
            }
            if let Ok(u) = i32::try_from(z.unix) {
                extra.extend_from_slice(&ID_ZEIT.to_le_bytes());
                extra.extend_from_slice(&5u16.to_le_bytes());
                extra.push(1);
                extra.extend_from_slice(&u.to_le_bytes());
            }
            let ver: u16 = if g_gross || o_gross { 45 } else { 20 };
            cd.extend_from_slice(&SIG_ZENTRAL.to_le_bytes());
            cd.extend_from_slice(&ver.to_le_bytes()); // erstellt mit (DOS/Windows)
            cd.extend_from_slice(&ver.to_le_bytes());
            cd.extend_from_slice(&FLAG_UTF8.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&z.zeit.to_le_bytes());
            cd.extend_from_slice(&z.datum.to_le_bytes());
            cd.extend_from_slice(&z.crc.to_le_bytes());
            let g32 = if g_gross { MAX32 as u32 } else { z.groesse as u32 };
            cd.extend_from_slice(&g32.to_le_bytes());
            cd.extend_from_slice(&g32.to_le_bytes());
            cd.extend_from_slice(&(z.name.len() as u16).to_le_bytes());
            cd.extend_from_slice(&(extra.len() as u16).to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes()); // Kommentar
            cd.extend_from_slice(&0u16.to_le_bytes()); // Disk
            cd.extend_from_slice(&0u16.to_le_bytes()); // intern
            cd.extend_from_slice(&(if z.ordner { 0x10u32 } else { 0x20u32 }).to_le_bytes());
            let o32 = if o_gross { MAX32 as u32 } else { z.offset as u32 };
            cd.extend_from_slice(&o32.to_le_bytes());
            cd.extend_from_slice(&z.name);
            cd.extend_from_slice(&extra);
        }
        self.w.write_all(&cd)?;
        let cd_size = cd.len() as u64;
        let mut ende_pos = cd_start.saturating_add(cd_size);
        let anzahl = self.zentral.len() as u64;
        let braucht64 = anzahl >= 0xFFFF || cd_start >= self.grenze || cd_size >= self.grenze;
        let mut e = Vec::new();
        if braucht64 {
            let e64_pos = ende_pos;
            e.extend_from_slice(&SIG_ENDE64.to_le_bytes());
            e.extend_from_slice(&44u64.to_le_bytes());
            e.extend_from_slice(&45u16.to_le_bytes());
            e.extend_from_slice(&45u16.to_le_bytes());
            e.extend_from_slice(&0u32.to_le_bytes());
            e.extend_from_slice(&0u32.to_le_bytes());
            e.extend_from_slice(&anzahl.to_le_bytes());
            e.extend_from_slice(&anzahl.to_le_bytes());
            e.extend_from_slice(&cd_size.to_le_bytes());
            e.extend_from_slice(&cd_start.to_le_bytes());
            e.extend_from_slice(&SIG_ZEIGER64.to_le_bytes());
            e.extend_from_slice(&0u32.to_le_bytes());
            e.extend_from_slice(&e64_pos.to_le_bytes());
            e.extend_from_slice(&1u32.to_le_bytes());
        }
        let a16 = if braucht64 { 0xFFFFu16 } else { anzahl as u16 };
        e.extend_from_slice(&SIG_ENDE.to_le_bytes());
        e.extend_from_slice(&0u16.to_le_bytes());
        e.extend_from_slice(&0u16.to_le_bytes());
        e.extend_from_slice(&a16.to_le_bytes());
        e.extend_from_slice(&a16.to_le_bytes());
        e.extend_from_slice(&(if braucht64 { MAX32 as u32 } else { cd_size as u32 }).to_le_bytes());
        e.extend_from_slice(&(if braucht64 { MAX32 as u32 } else { cd_start as u32 }).to_le_bytes());
        e.extend_from_slice(&0u16.to_le_bytes());
        self.w.write_all(&e)?;
        ende_pos = ende_pos.saturating_add(e.len() as u64);
        self.w.flush()?;
        Ok((self.w, ende_pos))
    }
}

// ------------------------------------------------------------------ Lesen

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZipEintrag {
    pub name: String,
    pub groesse: u64,
    pub gepackt: u64,
    pub crc: u32,
    pub offset: u64,
    pub methode: u16,
    pub dos: (u16, u16),
    pub unix: Option<i64>,
    pub ordner: bool,
}

fn u16_bei(b: &[u8], p: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(p)?, *b.get(p + 1)?]))
}
fn u32_bei(b: &[u8], p: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*b.get(p)?, *b.get(p + 1)?, *b.get(p + 2)?, *b.get(p + 3)?]))
}
fn u64_bei(b: &[u8], p: usize) -> Option<u64> {
    let s = b.get(p..p.checked_add(8)?)?;
    let mut a = [0u8; 8];
    a.copy_from_slice(s);
    Some(u64::from_le_bytes(a))
}

/// Liest das Inhaltsverzeichnis eines ZIP-Archivs.
pub fn verzeichnis<R: Read + Seek>(r: &mut R) -> io::Result<Vec<ZipEintrag>> {
    let laenge = r.seek(SeekFrom::End(0))?;
    let hinten = laenge.min(22 + 0xFFFF);
    let start = laenge - hinten;
    r.seek(SeekFrom::Start(start))?;
    let mut tail = vec![0u8; hinten as usize];
    r.read_exact(&mut tail)?;
    let mut ende = None;
    if tail.len() >= 22 {
        let mut i = tail.len() - 22;
        loop {
            if u32_bei(&tail, i) == Some(SIG_ENDE) {
                ende = Some(i);
                break;
            }
            if i == 0 {
                break;
            }
            i -= 1;
        }
    }
    let e = ende.ok_or_else(|| fehler("kein ZIP-Archiv (Ende nicht gefunden)"))?;
    let kaputt = || fehler("ZIP-Ende beschädigt");
    let mut anzahl = u16_bei(&tail, e + 10).ok_or_else(kaputt)? as u64;
    let mut cd_size = u32_bei(&tail, e + 12).ok_or_else(kaputt)? as u64;
    let mut cd_start = u32_bei(&tail, e + 16).ok_or_else(kaputt)? as u64;
    if anzahl == 0xFFFF || cd_size == MAX32 || cd_start == MAX32 {
        // ZIP64-Zeiger direkt vor dem Ende.
        let z = e.checked_sub(20).ok_or_else(|| fehler("ZIP64-Zeiger fehlt"))?;
        if u32_bei(&tail, z) != Some(SIG_ZEIGER64) {
            return Err(fehler("ZIP64-Zeiger fehlt"));
        }
        let e64 = u64_bei(&tail, z + 8).ok_or_else(kaputt)?;
        if e64.checked_add(56).is_none_or(|x| x > laenge) {
            return Err(kaputt());
        }
        r.seek(SeekFrom::Start(e64))?;
        let mut b = [0u8; 56];
        r.read_exact(&mut b)?;
        if u32_bei(&b, 0) != Some(SIG_ENDE64) {
            return Err(fehler("ZIP64-Ende beschädigt"));
        }
        anzahl = u64_bei(&b, 32).ok_or_else(kaputt)?;
        cd_size = u64_bei(&b, 40).ok_or_else(kaputt)?;
        cd_start = u64_bei(&b, 48).ok_or_else(kaputt)?;
    }
    if cd_start.checked_add(cd_size).is_none_or(|x| x > laenge) || cd_size > 1 << 30 {
        return Err(fehler("ZIP-Inhaltsverzeichnis ausserhalb der Datei"));
    }
    if anzahl > cd_size / 46 + 1 {
        return Err(fehler("ZIP-Inhaltsverzeichnis beschädigt"));
    }
    r.seek(SeekFrom::Start(cd_start))?;
    let mut cd = vec![0u8; cd_size as usize];
    r.read_exact(&mut cd)?;
    let mut out = Vec::with_capacity(anzahl as usize);
    let mut p = 0usize;
    for _ in 0..anzahl {
        if u32_bei(&cd, p) != Some(SIG_ZENTRAL) {
            return Err(fehler("ZIP-Eintrag beschädigt"));
        }
        let kaputt = || fehler("ZIP-Eintrag beschädigt");
        let flags = u16_bei(&cd, p + 8).ok_or_else(kaputt)?;
        let methode = u16_bei(&cd, p + 10).ok_or_else(kaputt)?;
        let zeit = u16_bei(&cd, p + 12).ok_or_else(kaputt)?;
        let datum = u16_bei(&cd, p + 14).ok_or_else(kaputt)?;
        let crc = u32_bei(&cd, p + 16).ok_or_else(kaputt)?;
        let mut gepackt = u32_bei(&cd, p + 20).ok_or_else(kaputt)? as u64;
        let mut groesse = u32_bei(&cd, p + 24).ok_or_else(kaputt)? as u64;
        let nl = u16_bei(&cd, p + 28).ok_or_else(kaputt)? as usize;
        let xl = u16_bei(&cd, p + 30).ok_or_else(kaputt)? as usize;
        let kl = u16_bei(&cd, p + 32).ok_or_else(kaputt)? as usize;
        let ext = u32_bei(&cd, p + 38).ok_or_else(kaputt)?;
        let mut offset = u32_bei(&cd, p + 42).ok_or_else(kaputt)? as u64;
        let name_b = cd.get(p + 46..p + 46 + nl).ok_or_else(kaputt)?;
        let extra = cd.get(p + 46 + nl..p + 46 + nl + xl).ok_or_else(kaputt)?;
        let _ = flags;
        let name = String::from_utf8_lossy(name_b).into_owned();
        let mut unix = None;
        let mut q = 0usize;
        while q + 4 <= extra.len() {
            let id = u16_bei(extra, q).ok_or_else(kaputt)?;
            let l = u16_bei(extra, q + 2).ok_or_else(kaputt)? as usize;
            let d = extra.get(q + 4..q + 4 + l).ok_or_else(kaputt)?;
            if id == ID_ZIP64 {
                let mut k = 0;
                if groesse == MAX32 {
                    groesse = u64_bei(d, k).ok_or_else(kaputt)?;
                    k += 8;
                }
                if gepackt == MAX32 {
                    gepackt = u64_bei(d, k).ok_or_else(kaputt)?;
                    k += 8;
                }
                if offset == MAX32 {
                    offset = u64_bei(d, k).ok_or_else(kaputt)?;
                }
            } else if id == ID_ZEIT && l >= 5 && d.first().is_some_and(|f| f & 1 != 0) {
                unix = u32_bei(d, 1).map(|v| v as i32 as i64);
            }
            q += 4 + l;
        }
        let ordner = name.ends_with('/') || ext & 0x10 != 0;
        out.push(ZipEintrag { name, groesse, gepackt, crc, offset, methode, dos: (zeit, datum), unix, ordner });
        p = p.checked_add(46 + nl + xl + kl).ok_or_else(kaputt)?;
    }
    Ok(out)
}

/// Leser für den Inhalt eines Eintrags (nur «gespeichert»).
pub fn eintrag_leser<'a, R: Read + Seek>(r: &'a mut R, e: &ZipEintrag) -> io::Result<io::Take<&'a mut R>> {
    if e.methode != 0 {
        return Err(fehler("ZIP-Eintrag ist komprimiert (nur ungepackte Einträge von Sicherwerk werden unterstützt)"));
    }
    r.seek(SeekFrom::Start(e.offset))?;
    let mut k = [0u8; 30];
    r.read_exact(&mut k)?;
    if u32_bei(&k, 0) != Some(SIG_LOKAL) {
        return Err(fehler("ZIP-Eintrag beschädigt"));
    }
    let nl = u16_bei(&k, 26).unwrap_or(0) as u64;
    let xl = u16_bei(&k, 28).unwrap_or(0) as u64;
    let daten = e.offset.saturating_add(30 + nl + xl);
    r.seek(SeekFrom::Start(daten))?;
    Ok(r.take(e.groesse))
}

// ------------------------------------------------------------------ DOS-Zeit

/// (Zeit, Datum) im DOS-Format aus Kalenderteilen; Jahre ausserhalb 1980–2107 werden begrenzt.
pub fn dos_aus_teilen(jahr: i32, monat: u32, tag: u32, h: u32, min: u32, s: u32) -> (u16, u16) {
    if jahr < 1980 {
        return (0, (1 << 5) | 1);
    }
    if jahr > 2107 {
        return ((23 << 11) | (59 << 5) | 29, ((127 << 9) | (12 << 5) | 31) as u16);
    }
    let zeit = ((h.min(23) << 11) | (min.min(59) << 5) | (s.min(59) / 2)) as u16;
    let datum = ((((jahr - 1980) as u32) << 9) | (monat.clamp(1, 12) << 5) | tag.clamp(1, 31)) as u16;
    (zeit, datum)
}

/// Tage seit 1970-01-01 → (Jahr, Monat, Tag) (Algorithmus von H. Hinnant).
pub fn datum_aus_tagen(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// DOS-Zeit aus Unix-Zeit, als UTC gerechnet (Ersatz, wenn keine Ortszeit bekannt ist).
pub fn dos_aus_unix_utc(unix: i64) -> (u16, u16) {
    let tage = unix.div_euclid(86_400);
    let sek = unix.rem_euclid(86_400) as u32;
    let (j, m, d) = datum_aus_tagen(tage);
    let j = j.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    dos_aus_teilen(j, m, d, sek / 3600, (sek / 60) % 60, sek % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn bauen(grenze: u64, dateien: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut z = ZipSchreiber::neu(Cursor::new(Vec::new())).unwrap().mit_grenze(grenze);
        for (n, d) in dateien {
            if let Some(o) = n.strip_suffix('/') {
                z.ordner(o, (0, 0x21), 0).unwrap();
            } else {
                z.datei(n, &mut &d[..], dos_aus_unix_utc(1_700_000_000), 1_700_000_000, |_| true).unwrap();
            }
        }
        let (c, ende) = z.fertig().unwrap();
        let v = c.into_inner();
        assert_eq!(ende, v.len() as u64);
        v
    }

    fn pruefen(v: Vec<u8>, dateien: &[(&str, Vec<u8>)]) {
        let mut c = Cursor::new(v);
        let inh = verzeichnis(&mut c).unwrap();
        assert_eq!(inh.len(), dateien.len());
        for (e, (n, d)) in inh.iter().zip(dateien) {
            assert_eq!(&e.name, n);
            if e.ordner {
                continue;
            }
            assert_eq!(e.groesse, d.len() as u64);
            assert_eq!(e.unix, Some(1_700_000_000));
            let mut r = eintrag_leser(&mut c, e).unwrap();
            let mut gelesen = Vec::new();
            r.read_to_end(&mut gelesen).unwrap();
            assert_eq!(&gelesen, d);
            let mut crc = Crc32::neu();
            crc.update(d);
            assert_eq!(e.crc, crc.wert());
        }
    }

    #[test]
    fn hin_und_zurueck() {
        let dateien: Vec<(&str, Vec<u8>)> = vec![
            ("Fotos/a.txt", b"Hallo Welt".to_vec()),
            ("Fotos/leer.txt", Vec::new()),
            ("Fotos/Ordner/", Vec::new()),
            ("Fotos/Grüsse ä ö ü.bin", (0..300_000u32).map(|i| (i % 251) as u8).collect()),
        ];
        pruefen(bauen(MAX32, &dateien), &dateien);
        // Mit kleiner Grenze: alles läuft über ZIP64-Felder.
        pruefen(bauen(5, &dateien), &dateien);
        pruefen(bauen(0, &dateien), &dateien);
    }

    #[test]
    fn verworfener_eintrag() {
        struct Kaputt(u32);
        impl Read for Kaputt {
            fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
                if self.0 == 0 {
                    return Err(io::Error::new(io::ErrorKind::Other, "Lesefehler"));
                }
                self.0 -= 1;
                let n = b.len().min(1000);
                b[..n].fill(9);
                Ok(n)
            }
        }
        let mut z = ZipSchreiber::neu(Cursor::new(Vec::new())).unwrap();
        z.datei("gut1", &mut &b"eins"[..], (0, 0x21), 0, |_| true).unwrap();
        assert!(z.datei("schlecht", &mut Kaputt(5), (0, 0x21), 0, |_| true).is_err());
        assert!(z.datei("abbruch", &mut &b"xxxx"[..], (0, 0x21), 0, |_| false).is_err());
        z.datei("gut2", &mut &b"zwei"[..], (0, 0x21), 0, |_| true).unwrap();
        let (c, ende) = z.fertig().unwrap();
        let mut v = c.into_inner();
        v.truncate(ende as usize);
        let mut c = Cursor::new(v);
        let inh = verzeichnis(&mut c).unwrap();
        assert_eq!(inh.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["gut1", "gut2"]);
        let mut s = String::new();
        eintrag_leser(&mut c, &inh[1]).unwrap().read_to_string(&mut s).unwrap();
        assert_eq!(s, "zwei");
    }

    #[test]
    fn kaputte_archive() {
        assert!(verzeichnis(&mut Cursor::new(Vec::new())).is_err());
        assert!(verzeichnis(&mut Cursor::new(vec![0u8; 100])).is_err());
        let gut = bauen(MAX32, &[("a", b"abc".to_vec())]);
        // Jede Kürzung und jedes veränderte Byte: Fehler oder Ergebnis, nie Absturz.
        for n in 0..gut.len() {
            let _ = verzeichnis(&mut Cursor::new(gut[..n].to_vec()));
        }
        for i in 0..gut.len() {
            for wert in [0u8, 0xFF, 0x7F] {
                let mut v = gut.clone();
                v[i] = wert;
                let mut c = Cursor::new(v);
                if let Ok(inh) = verzeichnis(&mut c) {
                    for e in &inh {
                        if let Ok(mut r) = eintrag_leser(&mut c, e) {
                            let mut s = Vec::new();
                            let _ = r.read_to_end(&mut s);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn dos_zeit() {
        assert_eq!(datum_aus_tagen(0), (1970, 1, 1));
        assert_eq!(datum_aus_tagen(19_000), (2022, 1, 08));
        assert_eq!(datum_aus_tagen(-1), (1969, 12, 31));
        // 2023-11-14 22:13:20 UTC
        let (z, d) = dos_aus_unix_utc(1_700_000_000);
        assert_eq!(d >> 9, 2023 - 1980);
        assert_eq!((d >> 5) & 0xF, 11);
        assert_eq!(d & 0x1F, 14);
        assert_eq!(z >> 11, 22);
        assert_eq!((z >> 5) & 0x3F, 13);
        assert_eq!((z & 0x1F) * 2, 20);
        assert_eq!(dos_aus_unix_utc(0), (0, 0x21));
        let _ = dos_aus_unix_utc(i64::MIN);
        let _ = dos_aus_unix_utc(i64::MAX);
    }
}
