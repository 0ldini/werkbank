//! Bildrechnerei für Diashow-Frames: Ausschnitt mit Unterpixel-Genauigkeit
//! (Schwenk, sanfter Zoom), Übergänge und Ein-/Ausblenden. Alles auf rohen
//! RGB-Puffern (3 Byte je Pixel), nur Standardbibliothek, auf mehrere Kerne verteilt.

use std::sync::OnceLock;

fn threads() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2)
            .clamp(1, 12)
    })
}

/// Ruft `f(zeilennummer, zeile)` für jede Zeile auf, verteilt auf mehrere Threads.
fn par_rows<F>(buf: &mut [u8], row_bytes: usize, f: F)
where
    F: Fn(usize, &mut [u8]) + Sync,
{
    let rows = buf.len() / row_bytes.max(1);
    if rows == 0 {
        return;
    }
    let n = threads().min(rows);
    if n <= 1 {
        for (r, row) in buf.chunks_mut(row_bytes).enumerate() {
            f(r, row);
        }
        return;
    }
    let per = (rows + n - 1) / n;
    std::thread::scope(|sc| {
        for (ci, chunk) in buf.chunks_mut(per * row_bytes).enumerate() {
            let f = &f;
            sc.spawn(move || {
                for (r, row) in chunk.chunks_mut(row_bytes).enumerate() {
                    f(ci * per + r, row);
                }
            });
        }
    });
}

/// Weiche Kurve 0..1 (langsam los, langsam aus).
pub fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Für Schwenk und Zoom: überwiegend gleichmässig, nur Anfang und Ende weich.
pub fn ease_move(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t + (smooth(t) - t) * 0.6
}

/// Schneidet aus `src` (sw × sh) das Rechteck (x0, y0, ww, hh) heraus und skaliert es
/// bilinear auf `dst` (dw × dh). Koordinaten dürfen Bruchteile von Pixeln sein, daher
/// läuft ein Schwenk ohne Ruckeln.
pub fn sample(dst: &mut [u8], dw: usize, dh: usize, src: &[u8], sw: usize, sh: usize, x0: f32, y0: f32, ww: f32, hh: f32) {
    debug_assert_eq!(dst.len(), dw * dh * 3);
    debug_assert_eq!(src.len(), sw * sh * 3);
    let maxx = (sw - 1) as f32;
    let maxy = (sh - 1) as f32;
    // Spalten vorab: (links, rechts, Gewicht rechts 0..256)
    let xs: Vec<(usize, usize, u32)> = (0..dw)
        .map(|i| {
            let fx = (x0 + (i as f32 + 0.5) * ww / dw as f32 - 0.5).clamp(0.0, maxx);
            let a = fx.floor();
            let b = (a + 1.0).min(maxx);
            (a as usize, b as usize, ((fx - a) * 256.0) as u32)
        })
        .collect();
    par_rows(dst, dw * 3, |j, row| {
        let fy = (y0 + (j as f32 + 0.5) * hh / dh as f32 - 0.5).clamp(0.0, maxy);
        let a = fy.floor();
        let ya = a as usize;
        let yb = ((a + 1.0).min(maxy)) as usize;
        let wy = ((fy - a) * 256.0) as u32;
        let r0 = &src[ya * sw * 3..(ya + 1) * sw * 3];
        let r1 = &src[yb * sw * 3..(yb + 1) * sw * 3];
        for (i, px) in row.chunks_exact_mut(3).enumerate() {
            let (xa, xb, wx) = xs[i];
            for c in 0..3 {
                let p00 = r0[xa * 3 + c] as u32;
                let p01 = r0[xb * 3 + c] as u32;
                let p10 = r1[xa * 3 + c] as u32;
                let p11 = r1[xb * 3 + c] as u32;
                let top = p00 * (256 - wx) + p01 * wx;
                let bot = p10 * (256 - wx) + p11 * wx;
                px[c] = ((top * (256 - wy) + bot * wy) >> 16) as u8;
            }
        }
    });
}

/// Überblendung: out = a·(1−t) + b·t
pub fn blend(out: &mut [u8], w: usize, a: &[u8], b: &[u8], t: f32) {
    let wb = (t.clamp(0.0, 1.0) * 256.0).round() as u32;
    let wa = 256 - wb;
    par_rows(out, w * 3, |j, row| {
        let o = j * w * 3;
        let (ra, rb) = (&a[o..o + row.len()], &b[o..o + row.len()]);
        for i in 0..row.len() {
            row[i] = ((ra[i] as u32 * wa + rb[i] as u32 * wb + 128) >> 8) as u8;
        }
    });
}

/// Schieben: A fährt nach links hinaus, B kommt von rechts herein.
pub fn push(out: &mut [u8], w: usize, a: &[u8], b: &[u8], t: f32) {
    let off = ((t.clamp(0.0, 1.0)) * w as f32).round() as usize;
    let off = off.min(w);
    par_rows(out, w * 3, |j, row| {
        let o = j * w * 3;
        let keep = (w - off) * 3;
        row[..keep].copy_from_slice(&a[o + off * 3..o + w * 3]);
        row[keep..].copy_from_slice(&b[o..o + off * 3]);
    });
}

/// Wischen von links nach rechts mit weicher Kante.
pub fn wipe(out: &mut [u8], w: usize, a: &[u8], b: &[u8], t: f32) {
    let feather = (w as f32 * 0.14).max(2.0);
    let edge = t.clamp(0.0, 1.0) * (w as f32 + feather);
    let wts: Vec<u32> = (0..w)
        .map(|x| (smooth((edge - x as f32) / feather) * 256.0).round() as u32)
        .collect();
    par_rows(out, w * 3, |j, row| {
        let o = j * w * 3;
        for (x, px) in row.chunks_exact_mut(3).enumerate() {
            let wb = wts[x];
            let wa = 256 - wb;
            for c in 0..3 {
                px[c] = ((a[o + x * 3 + c] as u32 * wa + b[o + x * 3 + c] as u32 * wb + 128) >> 8) as u8;
            }
        }
    });
}

/// Zoom-Überblendung: A wird beim Ausblenden etwas grösser, B setzt grösser ein
/// und rastet auf Normalgrösse ein. `ta` und `tb` sind Zwischenpuffer gleicher Grösse.
pub fn zoom(out: &mut [u8], w: usize, h: usize, a: &[u8], b: &[u8], t: f32, ta: &mut [u8], tb: &mut [u8]) {
    let t = t.clamp(0.0, 1.0);
    let (sa, sb) = (1.0 + 0.10 * t, 1.10 - 0.10 * t);
    let (wf, hf) = (w as f32, h as f32);
    let win = |s: f32| (wf / s, hf / s, (wf - wf / s) / 2.0, (hf - hf / s) / 2.0);
    let (aw, ah, ax, ay) = win(sa);
    sample(ta, w, h, a, w, h, ax, ay, aw, ah);
    let (bw, bh, bx, by) = win(sb);
    sample(tb, w, h, b, w, h, bx, by, bw, bh);
    blend(out, w, ta, tb, t);
}

/// Mit Schwarz abblenden: f = 1 unverändert, f = 0 ganz schwarz.
pub fn fade(buf: &mut [u8], w: usize, f: f32) {
    let m = (f.clamp(0.0, 1.0) * 256.0).round() as u32;
    if m >= 256 {
        return;
    }
    par_rows(buf, w * 3, |_, row| {
        for v in row.iter_mut() {
            *v = ((*v as u32 * m) >> 8) as u8;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn farbverlauf(w: usize, h: usize) -> Vec<u8> {
        let mut v = vec![0u8; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                let o = (y * w + x) * 3;
                v[o] = (x * 255 / (w - 1)) as u8;
                v[o + 1] = (y * 255 / (h - 1)) as u8;
                v[o + 2] = 77;
            }
        }
        v
    }

    #[test]
    fn ausschnitt_eins_zu_eins() {
        let src = farbverlauf(40, 20);
        let mut dst = vec![0u8; 40 * 20 * 3];
        sample(&mut dst, 40, 20, &src, 40, 20, 0.0, 0.0, 40.0, 20.0);
        assert_eq!(dst, src);
    }

    #[test]
    fn schwenk_ist_ruhig_und_laeuft_nach_rechts() {
        // 100 Pixel breites Bild, 40 Pixel Fenster: Rot-Kanal wächst mit x
        let src = farbverlauf(100, 10);
        let mut a = vec![0u8; 40 * 10 * 3];
        let mut b = vec![0u8; 40 * 10 * 3];
        sample(&mut a, 40, 10, &src, 100, 10, 10.0, 0.0, 40.0, 10.0);
        sample(&mut b, 40, 10, &src, 100, 10, 10.5, 0.0, 40.0, 10.0);
        assert!(b[0] > a[0]); // Fenster weiter rechts = hellere linke Kante
        assert!((b[0] as i32 - a[0] as i32) <= 2); // halber Pixel = winziger Schritt
    }

    #[test]
    fn uebergaenge_enden_stimmen() {
        let (w, h) = (16usize, 4usize);
        let a = vec![200u8; w * h * 3];
        let b = vec![40u8; w * h * 3];
        let mut o = vec![0u8; w * h * 3];
        blend(&mut o, w, &a, &b, 0.0);
        assert_eq!(o, a);
        blend(&mut o, w, &a, &b, 1.0);
        assert_eq!(o, b);
        push(&mut o, w, &a, &b, 0.0);
        assert_eq!(o, a);
        push(&mut o, w, &a, &b, 1.0);
        assert_eq!(o, b);
        wipe(&mut o, w, &a, &b, 0.0);
        assert_eq!(o, a);
        wipe(&mut o, w, &a, &b, 1.0);
        assert_eq!(o, b);
        let (mut ta, mut tb) = (vec![0u8; w * h * 3], vec![0u8; w * h * 3]);
        zoom(&mut o, w, h, &a, &b, 0.0, &mut ta, &mut tb);
        assert_eq!(o, a);
        zoom(&mut o, w, h, &a, &b, 1.0, &mut ta, &mut tb);
        assert_eq!(o, b);
    }

    #[test]
    fn kurven() {
        assert_eq!(smooth(0.0), 0.0);
        assert_eq!(smooth(1.0), 1.0);
        assert!((ease_move(0.5) - 0.5).abs() < 1e-6);
        assert!(ease_move(0.1) < 0.1); // Anfang weich
    }
}
