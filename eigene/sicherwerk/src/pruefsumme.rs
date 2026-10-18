//! Prüfsummen: SHA-256 (Vergleich Quelle/Sicherung) und CRC-32 (ZIP). Nur std;
//! Test: `rustc --edition 2021 --test src/pruefsumme.rs`.

#![allow(dead_code)]

use std::io::{self, Read};

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
    0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
    0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
    0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
    0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
    0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
    0xc67178f2,
];

#[derive(Clone)]
pub struct Sha256 {
    h: [u32; 8],
    puffer: [u8; 64],
    fuellung: usize,
    laenge: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::neu()
    }
}

impl Sha256 {
    pub fn neu() -> Self {
        Sha256 {
            h: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
            ],
            puffer: [0; 64],
            fuellung: 0,
            laenge: 0,
        }
    }

    fn block(h: &mut [u32; 8], b: &[u8; 64]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut bb, mut c, mut d, mut e, mut f, mut g, mut hh] = *h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & bb) ^ (a & c) ^ (bb & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = bb;
            bb = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, bb, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }

    pub fn update(&mut self, mut daten: &[u8]) {
        self.laenge = self.laenge.wrapping_add(daten.len() as u64);
        if self.fuellung > 0 {
            let n = (64 - self.fuellung).min(daten.len());
            self.puffer[self.fuellung..self.fuellung + n].copy_from_slice(&daten[..n]);
            self.fuellung += n;
            daten = &daten[n..];
            if self.fuellung == 64 {
                let b = self.puffer;
                Self::block(&mut self.h, &b);
                self.fuellung = 0;
            }
        }
        while daten.len() >= 64 {
            let mut b = [0u8; 64];
            b.copy_from_slice(&daten[..64]);
            Self::block(&mut self.h, &b);
            daten = &daten[64..];
        }
        if !daten.is_empty() {
            self.puffer[..daten.len()].copy_from_slice(daten);
            self.fuellung = daten.len();
        }
    }

    pub fn fertig(mut self) -> [u8; 32] {
        let bits = self.laenge.wrapping_mul(8);
        let mut pad = vec![0x80u8];
        let rest = (self.laenge + 1) % 64;
        let nullen = if rest <= 56 { 56 - rest } else { 120 - rest };
        pad.extend(std::iter::repeat(0u8).take(nullen as usize));
        pad.extend_from_slice(&bits.to_be_bytes());
        let l = self.laenge;
        self.update(&pad);
        self.laenge = l;
        let mut out = [0u8; 32];
        for (i, v) in self.h.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&v.to_be_bytes());
        }
        out
    }
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// CRC-32 (ZIP, IEEE).
pub struct Crc32(u32);

fn crc_tabelle() -> &'static [u32; 256] {
    static T: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *e = c;
        }
        t
    })
}

impl Default for Crc32 {
    fn default() -> Self {
        Self::neu()
    }
}

impl Crc32 {
    pub fn neu() -> Self {
        Crc32(0xFFFF_FFFF)
    }
    pub fn update(&mut self, daten: &[u8]) {
        let t = crc_tabelle();
        let mut c = self.0;
        for &b in daten {
            c = t[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
        }
        self.0 = c;
    }
    pub fn wert(&self) -> u32 {
        self.0 ^ 0xFFFF_FFFF
    }
}

/// Liest alles aus `r`, liefert SHA-256, CRC-32 und Länge. `weiter` wird je Block mit der
/// Blockgrösse aufgerufen; liefert es `false`, bricht das Lesen mit `Interrupted` ab.
pub fn lesen<R: Read>(r: &mut R, mut weiter: impl FnMut(u64) -> bool) -> io::Result<([u8; 32], u32, u64)> {
    let mut sha = Sha256::neu();
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
        sha.update(teil);
        crc.update(teil);
        n_ges = n_ges.saturating_add(n as u64);
        if !weiter(n as u64) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "abgebrochen"));
        }
    }
    Ok((sha.fertig(), crc.wert(), n_ges))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(b: &[u8]) -> String {
        let mut s = Sha256::neu();
        s.update(b);
        hex(&s.fertig())
    }

    #[test]
    fn sha256() {
        assert_eq!(sha(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            sha(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let m = vec![b'a'; 1_000_000];
        assert_eq!(sha(&m), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
        // In ungleichen Stücken gefüttert = gleiches Ergebnis.
        let mut s = Sha256::neu();
        for teil in m.chunks(77) {
            s.update(teil);
        }
        assert_eq!(hex(&s.fertig()), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
        for n in [55usize, 56, 63, 64, 65, 119, 120] {
            let a = vec![7u8; n];
            let mut s1 = Sha256::neu();
            s1.update(&a);
            let mut s2 = Sha256::neu();
            s2.update(&a[..n / 2]);
            s2.update(&a[n / 2..]);
            assert_eq!(s1.fertig(), s2.fertig());
        }
    }

    #[test]
    fn crc() {
        let mut c = Crc32::neu();
        c.update(b"123456789");
        assert_eq!(c.wert(), 0xCBF4_3926);
        let mut c = Crc32::neu();
        c.update(b"");
        assert_eq!(c.wert(), 0);
    }

    #[test]
    fn lesen_ganz() {
        let daten = vec![1u8; 700_000];
        let (s, c, n) = lesen(&mut &daten[..], |_| true).unwrap();
        assert_eq!(n, 700_000);
        let mut s2 = Sha256::neu();
        s2.update(&daten);
        assert_eq!(s, s2.fertig());
        let mut c2 = Crc32::neu();
        c2.update(&daten);
        assert_eq!(c, c2.wert());
        assert!(lesen(&mut &daten[..], |_| false).is_err());
    }
}
