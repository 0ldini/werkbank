//! «Zwei Sprecher angleichen (Klang)»: erkennt in einer Aufnahme mit zwei Stimmen die dumpfere
//! und berechnet, wo sie spricht und wie stark ihre Präsenz (um 2.5 kHz) und Höhen (ab 4.5 kHz)
//! angehoben werden. convert.rs rendert die Datei dann zweimal (ohne und mit Klangausgleich) und
//! blendet sekundenweise weich zwischen beiden über.
//!
//! Ablauf auf 16 kHz Mono, in Fenstern von 1 Sekunde:
//! 1. Je Sekunde Pegel (dB) und der Anteil von vier Bändern (80–300, 300–1000, 1000–3000,
//!    3000–8000 Hz) an der Energie 80–8000 Hz in dB (Hann-Fenster, eigene FFT).
//! 2. Sprache = Sekunden mit Pegel über −40 dB. Die vier Bandanteile werden normiert
//!    (z-Wert) und mit k-means (k = 2, 50 Runden) in zwei Gruppen geteilt. Start: die dumpfere
//!    und die hellere Hälfte nach Präsenz + Höhen – so ist das Ergebnis immer gleich.
//! 3. Stimme B = Gruppe mit weniger Präsenz + Höhen. Unterscheiden sich die Gruppen kaum
//!    (Präsenz- plus Höhenunterschied unter 3 dB) oder hat eine Gruppe unter 10 % der
//!    Sprechzeit, gibt es keinen Ausgleich.
//! 4. Maske je Sekunde (1 = B); Pausen gehören zur Stimme davor; Median über 3 Sekunden.
//!    `Weich` liefert daraus je Abtastwert einen über 0.4 s gemittelten Wert (weiche Übergänge).
//!
//! Nur Standardbibliothek – testbar mit `rustc --edition 2021 --test src/sprecher.rs`.

#![allow(dead_code)]

/// Abtastrate der Analyse.
pub const RATE: usize = 16_000;
/// FFT-Länge: eine Sekunde (16 000 Werte) mit Nullen aufgefüllt.
const FFT_N: usize = 16_384;
/// Bänder für die Klang-Merkmale (Hz): Tiefen, Mitten, Präsenz, Höhen.
pub const BAENDER: [(f64, f64); 4] = [(80.0, 300.0), (300.0, 1000.0), (1000.0, 3000.0), (3000.0, 8000.0)];
/// Sekunden darüber zählen als Sprache.
pub const SPRACHE_AB_DB: f32 = -40.0;
/// Mindestens so viele Sekunden Sprache braucht die Erkennung.
const MIN_SPRACHE_S: usize = 20;
/// Mindestanteil jeder Stimme an der Sprechzeit.
const MIN_ANTEIL: f64 = 0.10;
/// Mindestunterschied Präsenz + Höhen (dB) zwischen den Stimmen.
const MIN_UNTERSCHIED_DB: f32 = 3.0;

/// Merkmale einer Sekunde.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Merkmal {
    /// Pegel (RMS) in dBFS.
    pub pegel_db: f32,
    /// Anteil der vier Bänder an der Energie 80–8000 Hz, in dB (≤ 0).
    pub baender: [f32; 4],
}

// ---------------- FFT (Radix 2, iterativ) ----------------

struct Fft {
    n: usize,
    cos: Vec<f64>,
    sin: Vec<f64>,
    rev: Vec<usize>,
}

impl Fft {
    fn new(n: usize) -> Fft {
        let bits = n.trailing_zeros();
        let rev = (0..n).map(|i| i.reverse_bits() >> (usize::BITS - bits)).collect();
        let (cos, sin) = (0..n / 2)
            .map(|k| {
                let w = -2.0 * std::f64::consts::PI * k as f64 / n as f64;
                (w.cos(), w.sin())
            })
            .unzip();
        Fft { n, cos, sin, rev }
    }

    /// Vorwärts-FFT an Ort und Stelle.
    fn run(&self, re: &mut [f64], im: &mut [f64]) {
        let n = self.n;
        for i in 0..n {
            let j = self.rev[i];
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let half = len / 2;
            let step = n / len;
            for start in (0..n).step_by(len) {
                for k in 0..half {
                    let (c, s) = (self.cos[k * step], self.sin[k * step]);
                    let (a, b) = (start + k, start + k + half);
                    let tr = re[b] * c - im[b] * s;
                    let ti = re[b] * s + im[b] * c;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len *= 2;
        }
    }
}

// ---------------- Merkmale sammeln ----------------

/// Nimmt 16-kHz-Mono-Werte in beliebigen Stücken entgegen und berechnet je volle Sekunde ein
/// `Merkmal`. Eine angefangene letzte Sekunde zählt nicht.
pub struct Sammler {
    puffer: Vec<f64>,
    hann: Vec<f64>,
    fft: Fft,
    re: Vec<f64>,
    im: Vec<f64>,
    /// Bereich der FFT-Stellen je Band und für 80–8000 Hz.
    bins: [(usize, usize); 5],
    merkmale: Vec<Merkmal>,
    werte: u64,
}

impl Default for Sammler {
    fn default() -> Self {
        Sammler::new()
    }
}

impl Sammler {
    pub fn new() -> Sammler {
        let hann = (0..RATE)
            .map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (RATE - 1) as f64).cos())
            .collect();
        // Stelle k entspricht k·16000/16384 Hz; Band [a, b) wie im Prototyp.
        let bin = |f: f64| (f * FFT_N as f64 / RATE as f64).ceil() as usize;
        let mut bins = [(0, 0); 5];
        for (i, (a, b)) in BAENDER.iter().enumerate() {
            bins[i] = (bin(*a), bin(*b));
        }
        bins[4] = (bin(80.0), bin(8000.0));
        Sammler {
            puffer: Vec::with_capacity(RATE),
            hann,
            fft: Fft::new(FFT_N),
            re: vec![0.0; FFT_N],
            im: vec![0.0; FFT_N],
            bins,
            merkmale: Vec::new(),
            werte: 0,
        }
    }

    pub fn push_i16(&mut self, s: &[i16]) {
        for &v in s {
            self.push_wert(v as f64 / 32768.0);
        }
    }

    pub fn push_f32(&mut self, s: &[f32]) {
        for &v in s {
            self.push_wert(v as f64);
        }
    }

    fn push_wert(&mut self, v: f64) {
        self.werte += 1;
        self.puffer.push(if v.is_finite() { v } else { 0.0 });
        if self.puffer.len() == RATE {
            self.sekunde();
            self.puffer.clear();
        }
    }

    /// Anzahl der bisher gelesenen Werte (für den Fortschritt).
    pub fn werte(&self) -> u64 {
        self.werte
    }

    fn sekunde(&mut self) {
        let x = &self.puffer;
        let rms = (x.iter().map(|v| v * v).sum::<f64>() / x.len() as f64).sqrt();
        let pegel_db = (20.0 * (rms + 1e-9).log10()) as f32;
        for i in 0..FFT_N {
            self.re[i] = if i < RATE { x[i] * self.hann[i] } else { 0.0 };
            self.im[i] = 0.0;
        }
        self.fft.run(&mut self.re, &mut self.im);
        let energie = |(a, b): (usize, usize), re: &[f64], im: &[f64]| -> f64 {
            (a..b.min(FFT_N / 2 + 1)).map(|k| re[k] * re[k] + im[k] * im[k]).sum::<f64>() + 1e-12
        };
        let total = energie(self.bins[4], &self.re, &self.im);
        let mut baender = [0f32; 4];
        for (i, b) in baender.iter_mut().enumerate() {
            *b = (10.0 * (energie(self.bins[i], &self.re, &self.im) / total).log10()) as f32;
        }
        self.merkmale.push(Merkmal { pegel_db, baender });
    }

    pub fn fertig(self) -> Vec<Merkmal> {
        self.merkmale
    }
}

// ---------------- Gruppen bilden ----------------

/// Teilt Punkte mit k-means (k = 2) in zwei Gruppen. `start` sind die Anfangsmitten.
/// Ergebnis: Gruppe 0 oder 1 je Punkt.
pub fn kmeans2(punkte: &[[f64; 4]], start: [[f64; 4]; 2], runden: usize) -> Vec<u8> {
    let dist = |p: &[f64; 4], c: &[f64; 4]| p.iter().zip(c).map(|(a, b)| (a - b) * (a - b)).sum::<f64>();
    let mut mitte = start;
    let mut gruppe = vec![0u8; punkte.len()];
    for _ in 0..runden {
        for (g, p) in gruppe.iter_mut().zip(punkte) {
            *g = if dist(p, &mitte[1]) < dist(p, &mitte[0]) { 1 } else { 0 };
        }
        for (k, m) in mitte.iter_mut().enumerate() {
            let mut summe = [0f64; 4];
            let mut n = 0usize;
            for (p, _) in punkte.iter().zip(&gruppe).filter(|(_, g)| **g as usize == k) {
                for d in 0..4 {
                    summe[d] += p[d];
                }
                n += 1;
            }
            // Leere Gruppe: alte Mitte behalten.
            if n > 0 {
                for d in 0..4 {
                    m[d] = summe[d] / n as f64;
                }
            }
        }
    }
    gruppe
}

/// Klangausgleich für Stimme B in dB (für ffmpeg «equalizer»/«highshelf»).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Ausgleich {
    /// Absenkung um 220 Hz (≤ 0; 0 = keine).
    pub tief_db: f32,
    /// Anhebung um 2500 Hz (Präsenz).
    pub praesenz_db: f32,
    /// Anhebung ab 4500 Hz (Höhen).
    pub hoehen_db: f32,
}

impl Ausgleich {
    /// Aus den gemessenen Unterschieden (A − B bei Präsenz/Höhen, B − A bei den Tiefen).
    pub fn aus(dp: f32, dh: f32, dl: f32) -> Ausgleich {
        Ausgleich {
            tief_db: if dl > 1.0 { -(dl * 0.8).clamp(0.0, 4.0) } else { 0.0 },
            praesenz_db: (dp * 0.9).clamp(0.0, 8.0),
            hoehen_db: dh.clamp(0.0, 9.0),
        }
    }
}

/// Ergebnis der Stimmen-Erkennung.
#[derive(Clone, Debug, PartialEq)]
pub struct Ergebnis {
    /// Sekunden mit Sprache.
    pub sprache_s: usize,
    pub a_s: usize,
    pub b_s: usize,
    /// Median-Pegel der Stimmen (dBFS).
    pub pegel_a: f32,
    pub pegel_b: f32,
    /// Mittlere Bandanteile (dB) je Stimme: Tiefen, Mitten, Präsenz, Höhen.
    pub baender_a: [f32; 4],
    pub baender_b: [f32; 4],
    /// Präsenz A − B, Höhen A − B, Tiefen B − A (dB).
    pub dp: f32,
    pub dh: f32,
    pub dl: f32,
    /// None = keine zweite, dumpfere Stimme erkannt.
    pub ausgleich: Option<Ausgleich>,
    /// Je Sekunde (eine mehr als volle Sekunden): 1 = Stimme B, 0 = Stimme A.
    pub maske: Vec<f32>,
}

fn median(v: &mut [f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

fn mittel(v: &[[f64; 4]]) -> [f32; 4] {
    let mut s = [0f64; 4];
    for p in v {
        for d in 0..4 {
            s[d] += p[d];
        }
    }
    let n = v.len().max(1) as f64;
    [(s[0] / n) as f32, (s[1] / n) as f32, (s[2] / n) as f32, (s[3] / n) as f32]
}

/// Erkennt die zwei Stimmen und berechnet Ausgleich und Maske.
pub fn analyse(merkmale: &[Merkmal]) -> Ergebnis {
    let sprache: Vec<usize> = (0..merkmale.len()).filter(|&i| merkmale[i].pegel_db > SPRACHE_AB_DB).collect();
    let roh: Vec<[f64; 4]> = sprache
        .iter()
        .map(|&i| {
            let b = merkmale[i].baender;
            [b[0] as f64, b[1] as f64, b[2] as f64, b[3] as f64]
        })
        .collect();
    let leer = Ergebnis {
        sprache_s: sprache.len(),
        a_s: sprache.len(),
        b_s: 0,
        pegel_a: 0.0,
        pegel_b: 0.0,
        baender_a: mittel(&roh),
        baender_b: [0.0; 4],
        dp: 0.0,
        dh: 0.0,
        dl: 0.0,
        ausgleich: None,
        maske: vec![0.0; merkmale.len() + 1],
    };
    if sprache.len() < MIN_SPRACHE_S {
        return leer;
    }

    // z-Werte
    let n = roh.len() as f64;
    let mut mu = [0f64; 4];
    let mut sd = [0f64; 4];
    for d in 0..4 {
        mu[d] = roh.iter().map(|p| p[d]).sum::<f64>() / n;
        sd[d] = (roh.iter().map(|p| (p[d] - mu[d]).powi(2)).sum::<f64>() / n).sqrt() + 1e-9;
    }
    let z: Vec<[f64; 4]> = roh
        .iter()
        .map(|p| [(p[0] - mu[0]) / sd[0], (p[1] - mu[1]) / sd[1], (p[2] - mu[2]) / sd[2], (p[3] - mu[3]) / sd[3]])
        .collect();

    // Start: dumpfere und hellere Hälfte nach Präsenz + Höhen.
    let mut ord: Vec<usize> = (0..z.len()).collect();
    ord.sort_by(|&a, &b| (z[a][2] + z[a][3]).total_cmp(&(z[b][2] + z[b][3])));
    let half = ord.len() / 2;
    let haelfte = |idx: &[usize]| {
        let v: Vec<[f64; 4]> = idx.iter().map(|&i| z[i]).collect();
        let m = mittel(&v);
        [m[0] as f64, m[1] as f64, m[2] as f64, m[3] as f64]
    };
    let start = [haelfte(&ord[..half]), haelfte(&ord[half..])];
    let gruppe = kmeans2(&z, start, 50);

    let teil = |k: u8| -> Vec<[f64; 4]> { roh.iter().zip(&gruppe).filter(|(_, g)| **g == k).map(|(p, _)| *p).collect() };
    let (g0, g1) = (teil(0), teil(1));
    let (m0, m1) = (mittel(&g0), mittel(&g1));
    // B = Gruppe mit weniger Präsenz + Höhen.
    let b_gruppe: u8 = if m1[2] + m1[3] < m0[2] + m0[3] { 1 } else { 0 };
    let (ba, bb) = if b_gruppe == 1 { (m0, m1) } else { (m1, m0) };
    let pegel = |k: u8| {
        let mut v: Vec<f32> = sprache.iter().zip(&gruppe).filter(|(_, g)| **g == k).map(|(&i, _)| merkmale[i].pegel_db).collect();
        median(&mut v)
    };
    let b_s = gruppe.iter().filter(|g| **g == b_gruppe).count();
    let a_s = gruppe.len() - b_s;
    let dp = ba[2] - bb[2];
    let dh = ba[3] - bb[3];
    let dl = bb[0] - ba[0];

    let mut erg = Ergebnis {
        sprache_s: sprache.len(),
        a_s,
        b_s,
        pegel_a: pegel(1 - b_gruppe),
        pegel_b: pegel(b_gruppe),
        baender_a: ba,
        baender_b: bb,
        dp,
        dh,
        dl,
        ausgleich: None,
        maske: vec![0.0; merkmale.len() + 1],
    };
    let anteil_min = a_s.min(b_s) as f64 / sprache.len() as f64;
    if dp + dh < MIN_UNTERSCHIED_DB || anteil_min < MIN_ANTEIL {
        return erg;
    }
    erg.ausgleich = Some(Ausgleich::aus(dp, dh, dl));

    // Maske je Sekunde: Pausen erben die Stimme davor, dann Median über 3 s.
    let mut roh_maske: Vec<Option<f32>> = vec![None; merkmale.len() + 1];
    for (&i, &g) in sprache.iter().zip(&gruppe) {
        roh_maske[i] = Some(if g == b_gruppe { 1.0 } else { 0.0 });
    }
    let mut letzte = 0.0;
    let gefuellt: Vec<f32> = roh_maske
        .iter()
        .map(|m| {
            if let Some(v) = m {
                letzte = *v;
            }
            letzte
        })
        .collect();
    erg.maske = median3(&gefuellt);
    erg
}

/// Median über 3 Werte, am Rand wird der Randwert wiederholt.
pub fn median3(v: &[f32]) -> Vec<f32> {
    let n = v.len();
    (0..n)
        .map(|i| {
            let mut w = [v[i.saturating_sub(1)], v[i], v[(i + 1).min(n - 1)]];
            w.sort_by(|a, b| a.total_cmp(b));
            w[1]
        })
        .collect()
}

// ---------------- Weiche Übergänge ----------------

/// Maske je Abtastwert: die Sekunden-Maske, über `breite_s` gleitend gemittelt (zentriert,
/// am Rand mit dem Randwert fortgesetzt). Rechnet mit Teilsummen, daher O(1) je Wert.
pub struct Weich {
    maske: Vec<f64>,
    praefix: Vec<f64>,
    rate: f64,
    breite: f64,
}

impl Weich {
    pub fn new(maske: &[f32], rate: u32, breite_s: f64) -> Weich {
        let maske: Vec<f64> = maske.iter().map(|v| *v as f64).collect();
        let mut praefix = Vec::with_capacity(maske.len() + 1);
        let mut s = 0.0;
        praefix.push(0.0);
        for v in &maske {
            s += v;
            praefix.push(s);
        }
        let rate = rate.max(1) as f64;
        Weich { maske, praefix, rate, breite: (breite_s * rate).round().max(1.0) }
    }

    /// Integral der Stufenfunktion von 0 bis x (in Abtastwerten).
    fn integral(&self, x: f64) -> f64 {
        let n = self.maske.len();
        let (erst, letzt) = (self.maske[0], self.maske[n - 1]);
        let ende = n as f64 * self.rate;
        if x <= 0.0 {
            return erst * x;
        }
        if x >= ende {
            return self.praefix[n] * self.rate + letzt * (x - ende);
        }
        let k = (x / self.rate).floor() as usize;
        let k = k.min(n - 1);
        self.praefix[k] * self.rate + self.maske[k] * (x - k as f64 * self.rate)
    }

    /// Maskenwert (0..1) für Abtastwert `i`.
    pub fn wert(&self, i: u64) -> f32 {
        if self.maske.is_empty() {
            return 0.0;
        }
        let a = i as f64 - (self.breite / 2.0).floor();
        let b = a + self.breite;
        (((self.integral(b) - self.integral(a)) / self.breite) as f32).clamp(0.0, 1.0)
    }
}

/// Mischt einen Block: out = a·(1−m) + b·m, `start` = Index des ersten Werts.
pub fn mische(a: &[f32], b: &[f32], weich: &Weich, start: u64, out: &mut Vec<f32>) {
    out.clear();
    for (j, (x, y)) in a.iter().zip(b).enumerate() {
        let m = weich.wert(start + j as u64);
        out.push(x * (1.0 - m) + y * m);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Einfacher Zufall (xorshift), damit die Tests ohne Fremdcrates laufen.
    struct Zufall(u64);
    impl Zufall {
        fn f(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
        }
    }

    /// Eine Sekunde «Stimme»: Grundton mit Obertönen, `hell` = Stärke der Obertöne über 1 kHz.
    fn stimme(z: &mut Zufall, f0: f64, pegel: f64, hell: f64) -> Vec<f32> {
        (0..RATE)
            .map(|i| {
                let t = i as f64 / RATE as f64;
                let mut v = 0.0;
                for h in 1..=30 {
                    let f = f0 * h as f64;
                    if f >= 7500.0 {
                        break;
                    }
                    let g = if f > 1000.0 { hell } else { 1.0 } / h as f64;
                    v += g * (2.0 * std::f64::consts::PI * f * t).sin();
                }
                (pegel * (v * 0.3 + 0.01 * z.f())) as f32
            })
            .collect()
    }

    fn merkmale_von(sekunden: &[Vec<f32>]) -> Vec<Merkmal> {
        let mut s = Sammler::new();
        for sek in sekunden {
            // in ungeraden Stücken einspeisen
            for st in sek.chunks(777) {
                s.push_f32(st);
            }
        }
        s.fertig()
    }

    #[test]
    fn fft_sinus() {
        let f = Fft::new(64);
        let mut re: Vec<f64> = (0..64).map(|i| (2.0 * std::f64::consts::PI * 5.0 * i as f64 / 64.0).cos()).collect();
        let mut im = vec![0.0; 64];
        f.run(&mut re, &mut im);
        assert!((re[5] - 32.0).abs() < 1e-9 && (re[59] - 32.0).abs() < 1e-9);
        assert!(re[6].abs() < 1e-9 && im[5].abs() < 1e-9);
    }

    #[test]
    fn baender_eines_tons() {
        // 2 kHz liegt fast ganz im Präsenzband.
        let x: Vec<i16> = (0..RATE * 2).map(|i| (8000.0 * (2.0 * std::f64::consts::PI * 2000.0 * i as f64 / RATE as f64).sin()) as i16).collect();
        let mut s = Sammler::new();
        s.push_i16(&x[..5000]);
        s.push_i16(&x[5000..]);
        s.push_i16(&[0; 100]); // angefangene Sekunde zählt nicht
        let m = s.fertig();
        assert_eq!(m.len(), 2);
        assert!(m[0].baender[2] > -0.1, "{:?}", m[0]);
        assert!(m[0].baender[0] < -40.0 && m[0].baender[3] < -40.0, "{:?}", m[0]);
        // Sinus mit Amplitude 8000/32768: RMS = 0.244/√2 → −15.3 dB
        assert!((m[0].pegel_db + 15.25).abs() < 0.2, "{}", m[0].pegel_db);
    }

    /// Zwei Stimmen abwechselnd: A hell und laut, B dumpf und 4.5 dB leiser, mit Pausen.
    fn gespraech() -> (Vec<Vec<f32>>, Vec<u8>) {
        let mut z = Zufall(0x1234_5678_9abc);
        let mut sek = Vec::new();
        let mut wer = Vec::new(); // 0 = A, 1 = B, 2 = Pause
        let plan: &[(u8, usize)] = &[(0, 12), (2, 1), (1, 9), (0, 7), (2, 2), (1, 14), (0, 10), (1, 8), (2, 1), (0, 6)];
        for &(w, n) in plan {
            for _ in 0..n {
                sek.push(match w {
                    0 => stimme(&mut z, 140.0, 0.5, 1.0),
                    1 => stimme(&mut z, 190.0, 0.3, 0.35),
                    _ => (0..RATE).map(|_| (0.001 * z.f()) as f32).collect(),
                });
                wer.push(w);
            }
        }
        (sek, wer)
    }

    #[test]
    fn zwei_stimmen_erkennen() {
        let (sek, wer) = gespraech();
        let m = merkmale_von(&sek);
        assert_eq!(m.len(), sek.len());
        let e = analyse(&m);
        assert_eq!(e.sprache_s, wer.iter().filter(|w| **w != 2).count());
        assert_eq!(e.b_s, wer.iter().filter(|w| **w == 1).count());
        assert!((e.pegel_a - e.pegel_b - 4.4).abs() < 1.0, "{} {}", e.pegel_a, e.pegel_b);
        assert!(e.dp > 3.0 && e.dh > 3.0, "{e:?}");
        let a = e.ausgleich.expect("Ausgleich");
        assert!(a.praesenz_db > 2.0 && a.praesenz_db <= 8.0 && a.hoehen_db <= 9.0, "{a:?}");
        // Maske: B-Sekunden 1, A-Sekunden 0 (bis auf Ränder nach dem Median), Pausen erben.
        let mut falsch = 0;
        let mut vorher = 0u8;
        for (i, w) in wer.iter().enumerate() {
            let soll = match w {
                2 => vorher,
                w => {
                    vorher = *w;
                    *w
                }
            };
            if e.maske[i] != soll as f32 {
                falsch += 1;
            }
        }
        assert!(falsch <= 2, "falsch: {falsch} {:?}", e.maske);
        assert_eq!(e.maske.len(), m.len() + 1);
    }

    #[test]
    fn eine_stimme_kein_ausgleich() {
        let mut z = Zufall(42);
        let sek: Vec<Vec<f32>> = (0..60).map(|i| stimme(&mut z, 140.0 + (i % 5) as f64 * 3.0, 0.4 + (i % 3) as f64 * 0.1, 1.0)).collect();
        let e = analyse(&merkmale_von(&sek));
        assert_eq!(e.ausgleich, None, "{e:?}");
        assert!(e.maske.iter().all(|v| *v == 0.0));
        // Zu wenig Sprache oder Stille: auch kein Ausgleich, kein Absturz.
        assert_eq!(analyse(&[]).ausgleich, None);
        let still = vec![Merkmal { pegel_db: -90.0, baender: [0.0; 4] }; 30];
        assert_eq!(analyse(&still).ausgleich, None);
    }

    #[test]
    fn kleine_zweite_stimme_zaehlt_nicht() {
        let (sek, wer) = gespraech();
        // Nur 3 Sekunden B unter vielen A: unter 10 %.
        let mut z = Zufall(7);
        let mut s2: Vec<Vec<f32>> = sek.into_iter().zip(&wer).filter(|(_, w)| **w == 0).map(|(s, _)| s).collect();
        for _ in 0..3 {
            s2.push(stimme(&mut z, 190.0, 0.3, 0.35));
        }
        assert_eq!(analyse(&merkmale_von(&s2)).ausgleich, None);
    }

    #[test]
    fn ausgleich_grenzen() {
        let a = Ausgleich::aus(6.5, 7.0, 2.0);
        assert!((a.praesenz_db - 5.85).abs() < 1e-4 && (a.hoehen_db - 7.0).abs() < 1e-4);
        assert!((a.tief_db + 1.6).abs() < 1e-4);
        let b = Ausgleich::aus(20.0, 20.0, 0.5);
        assert_eq!((b.praesenz_db, b.hoehen_db, b.tief_db), (8.0, 9.0, 0.0));
        assert_eq!(Ausgleich::aus(-2.0, -1.0, 9.0).tief_db, -4.0);
    }

    #[test]
    fn median_und_weich() {
        assert_eq!(median3(&[0.0, 1.0, 0.0, 0.0, 1.0, 1.0]), vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0]);
        // Stufe bei Sekunde 2 (Rate 10, Breite 0.4 s = 4 Werte)
        let w = Weich::new(&[0.0, 0.0, 1.0, 1.0], 10, 0.4);
        assert_eq!(w.wert(0), 0.0);
        assert_eq!(w.wert(17), 0.0);
        assert_eq!(w.wert(18), 0.0);
        assert!((w.wert(19) - 0.25).abs() < 1e-6);
        assert!((w.wert(20) - 0.5).abs() < 1e-6);
        assert!((w.wert(21) - 0.75).abs() < 1e-6);
        assert_eq!(w.wert(22), 1.0);
        assert_eq!(w.wert(1000), 1.0); // nach dem Ende: Randwert
        let mut out = Vec::new();
        mische(&[1.0, 1.0, 1.0], &[3.0, 3.0, 3.0], &w, 19, &mut out);
        assert_eq!(out, vec![1.5, 2.0, 2.5]);
        assert_eq!(Weich::new(&[], 48000, 0.4).wert(5), 0.0);
    }
}
