//! Reine Bildschirmfoto-Logik («Schnappwerk»-Teil) ohne Oberfläche und ohne
//! Plattform-APIs: RGBA-Bildpuffer, Rechteck-Normalisierung, Zuschneiden,
//! Schwärzen, Verpixeln (zerstört die Pixel wirklich), Zeichnen von Pfeil,
//! Rechteck, Stift und Leuchtstift, Einblenden von Text-Deckung, Dateinamen,
//! Rückgängig/Wiederholen und Monitor-/Fenster-Zuordnung.
//!
//! Nur Standardbibliothek – eigenständig prüfbar:
//! `rustc --edition 2021 --test src/schnapp.rs -o /tmp/schnapp && /tmp/schnapp`

#![allow(dead_code)]

/// RGBA8-Bild, dicht gepackt (Zeilenlänge w*4).
#[derive(Clone, Debug, PartialEq)]
pub struct Img {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

/// Rechteck in Bildpixeln: (x, y, breite, höhe).
pub type Rechteck = (u32, u32, u32, u32);

/// Mischart beim Zeichnen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Blend {
    /// Farbe deckt (mit ihrer Deckkraft).
    Normal,
    /// Wie ein Leuchtstift: multipliziert, dunkle Schrift bleibt lesbar.
    Multiply,
}

impl Img {
    pub fn new(w: u32, h: u32, fill: [u8; 4]) -> Img {
        let n = w as usize * h as usize;
        let mut px = Vec::with_capacity(n * 4);
        for _ in 0..n {
            px.extend_from_slice(&fill);
        }
        Img { w, h, px }
    }

    /// Aus einem dicht gepackten BGRA-Puffer (wie von der Bildschirmaufnahme).
    /// Alpha wird auf 255 gesetzt. `None`, wenn der Puffer zu kurz ist.
    pub fn from_bgra(w: u32, h: u32, bgra: &[u8]) -> Option<Img> {
        let n = w as usize * h as usize * 4;
        if w == 0 || h == 0 || bgra.len() < n {
            return None;
        }
        let mut px = vec![0u8; n];
        for (d, s) in px.chunks_exact_mut(4).zip(bgra[..n].chunks_exact(4)) {
            d[0] = s[2];
            d[1] = s[1];
            d[2] = s[0];
            d[3] = 255;
        }
        Some(Img { w, h, px })
    }

    #[inline]
    fn idx(&self, x: u32, y: u32) -> usize {
        (y as usize * self.w as usize + x as usize) * 4
    }

    pub fn get(&self, x: u32, y: u32) -> [u8; 4] {
        let i = self.idx(x, y);
        [self.px[i], self.px[i + 1], self.px[i + 2], self.px[i + 3]]
    }

    pub fn set(&mut self, x: u32, y: u32, c: [u8; 4]) {
        let i = self.idx(x, y);
        self.px[i..i + 4].copy_from_slice(&c);
    }
}

// ---------- Rechtecke ----------

/// Macht aus zwei beliebigen Eckpunkten (Reihenfolge egal, auch ausserhalb
/// des Bildes) ein Rechteck, das ganz im Bild (w × h) liegt. Bruchteile werden
/// nach aussen gerundet. `None`, wenn nichts übrig bleibt.
pub fn norm_rect(ax: f32, ay: f32, bx: f32, by: f32, w: u32, h: u32) -> Option<Rechteck> {
    if !(ax.is_finite() && ay.is_finite() && bx.is_finite() && by.is_finite()) {
        return None;
    }
    let x0 = ax.min(bx).floor().max(0.0);
    let y0 = ay.min(by).floor().max(0.0);
    let x1 = ax.max(bx).ceil().min(w as f32);
    let y1 = ay.max(by).ceil().min(h as f32);
    if x1 - x0 < 1.0 || y1 - y0 < 1.0 {
        return None;
    }
    Some((x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32))
}

/// Beschneidet ein Rechteck auf das Bild.
pub fn clip(r: Rechteck, w: u32, h: u32) -> Option<Rechteck> {
    let (x, y, rw, rh) = r;
    if x >= w || y >= h {
        return None;
    }
    let rw = rw.min(w - x);
    let rh = rh.min(h - y);
    if rw == 0 || rh == 0 {
        return None;
    }
    Some((x, y, rw, rh))
}

/// Schneidet das Rechteck heraus (neues Bild). `None`, wenn es leer ist.
pub fn crop(img: &Img, r: Rechteck) -> Option<Img> {
    let (x, y, w, h) = clip(r, img.w, img.h)?;
    let row = w as usize * 4;
    let mut px = Vec::with_capacity(row * h as usize);
    for yy in y..y + h {
        let s = img.idx(x, yy);
        px.extend_from_slice(&img.px[s..s + row]);
    }
    Some(Img { w, h, px })
}

/// Füllt das Rechteck deckend (Schwärzen).
pub fn fill_rect(img: &mut Img, r: Rechteck, c: [u8; 4]) {
    let Some((x, y, w, h)) = clip(r, img.w, img.h) else { return };
    for yy in y..y + h {
        for xx in x..x + w {
            img.set(xx, yy, c);
        }
    }
}

/// Sinnvolle Blockgrösse fürs Verpixeln: grob genug, dass Schrift nicht mehr
/// lesbar oder rekonstruierbar ist.
pub fn pixel_block(rw: u32, rh: u32) -> u32 {
    (rw.min(rh) / 6).clamp(10, 48)
}

/// Verpixelt das Rechteck: jeder Block (ab der linken oberen Ecke des
/// Rechtecks) wird durch seinen Mittelwert ersetzt. Die ursprünglichen Pixel
/// sind danach im Puffer nicht mehr vorhanden.
pub fn pixelate(img: &mut Img, r: Rechteck, block: u32) {
    let Some((x, y, w, h)) = clip(r, img.w, img.h) else { return };
    let b = block.max(1);
    let mut by = y;
    while by < y + h {
        let bh = b.min(y + h - by);
        let mut bx = x;
        while bx < x + w {
            let bw = b.min(x + w - bx);
            let mut sum = [0u64; 4];
            for yy in by..by + bh {
                for xx in bx..bx + bw {
                    let p = img.get(xx, yy);
                    for k in 0..4 {
                        sum[k] += p[k] as u64;
                    }
                }
            }
            let n = (bw as u64 * bh as u64).max(1);
            let avg = [
                ((sum[0] + n / 2) / n) as u8,
                ((sum[1] + n / 2) / n) as u8,
                ((sum[2] + n / 2) / n) as u8,
                ((sum[3] + n / 2) / n) as u8,
            ];
            for yy in by..by + bh {
                for xx in bx..bx + bw {
                    img.set(xx, yy, avg);
                }
            }
            bx += b;
        }
        by += b;
    }
}

// ---------- Zeichnen ----------

#[inline]
fn blend_px(px: &mut [u8], c: [u8; 4], cov: u8, mode: Blend) {
    let a = c[3] as u32 * cov as u32 / 255;
    if a == 0 {
        return;
    }
    for k in 0..3 {
        let src = px[k] as u32;
        let target = match mode {
            Blend::Normal => c[k] as u32,
            Blend::Multiply => src * c[k] as u32 / 255,
        };
        px[k] = ((src * (255 - a) + target * a + 127) / 255) as u8;
    }
    let da = px[3] as u32;
    px[3] = (da + a * (255 - da) / 255) as u8;
}

/// Deckungsmaske über einem Ausschnitt des Bildes. Mehrere Striche werden
/// erst gesammelt (Maximum) und dann einmal eingemischt – so wird ein
/// halbtransparenter Strich an Überlappungen nicht dunkler.
struct Mask {
    x0: i64,
    y0: i64,
    w: i64,
    h: i64,
    cov: Vec<u8>,
}

impl Mask {
    fn new(img: &Img, minx: f32, miny: f32, maxx: f32, maxy: f32) -> Option<Mask> {
        if !(minx.is_finite() && miny.is_finite() && maxx.is_finite() && maxy.is_finite()) {
            return None;
        }
        let x0 = (minx.floor() as i64).max(0);
        let y0 = (miny.floor() as i64).max(0);
        let x1 = (maxx.ceil() as i64).min(img.w as i64);
        let y1 = (maxy.ceil() as i64).min(img.h as i64);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let (w, h) = (x1 - x0, y1 - y0);
        Some(Mask { x0, y0, w, h, cov: vec![0u8; (w * h) as usize] })
    }

    /// Range der Maskenpixel (absolut) für einen Bereich, beschnitten.
    fn span(&self, lo: f32, hi: f32, o: i64, n: i64) -> (i64, i64) {
        let a = (lo.floor() as i64).max(o);
        let b = (hi.ceil() as i64).min(o + n);
        (a, b)
    }

    #[inline]
    fn put(&mut self, x: i64, y: i64, c: u8) {
        let i = ((y - self.y0) * self.w + (x - self.x0)) as usize;
        if let Some(v) = self.cov.get_mut(i) {
            if c > *v {
                *v = c;
            }
        }
    }

    /// Dicker Strich (Kapsel) von a nach b mit Radius r, weich gerandet.
    fn segment(&mut self, a: (f32, f32), b: (f32, f32), r: f32) {
        let (xa, xb) = self.span(a.0.min(b.0) - r - 1.0, a.0.max(b.0) + r + 1.0, self.x0, self.w);
        let (ya, yb) = self.span(a.1.min(b.1) - r - 1.0, a.1.max(b.1) + r + 1.0, self.y0, self.h);
        for y in ya..yb {
            for x in xa..xb {
                let d = dist_seg((x as f32 + 0.5, y as f32 + 0.5), a, b);
                let c = (r + 0.5 - d).clamp(0.0, 1.0);
                if c > 0.0 {
                    self.put(x, y, (c * 255.0).round() as u8);
                }
            }
        }
    }

    /// Gefülltes Dreieck.
    fn triangle(&mut self, p: [(f32, f32); 3]) {
        let minx = p[0].0.min(p[1].0).min(p[2].0);
        let maxx = p[0].0.max(p[1].0).max(p[2].0);
        let miny = p[0].1.min(p[1].1).min(p[2].1);
        let maxy = p[0].1.max(p[1].1).max(p[2].1);
        let (xa, xb) = self.span(minx, maxx, self.x0, self.w);
        let (ya, yb) = self.span(miny, maxy, self.y0, self.h);
        let edge = |a: (f32, f32), b: (f32, f32), q: (f32, f32)| (b.0 - a.0) * (q.1 - a.1) - (b.1 - a.1) * (q.0 - a.0);
        for y in ya..yb {
            for x in xa..xb {
                let q = (x as f32 + 0.5, y as f32 + 0.5);
                let e0 = edge(p[0], p[1], q);
                let e1 = edge(p[1], p[2], q);
                let e2 = edge(p[2], p[0], q);
                let inside = (e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0) || (e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0);
                if inside {
                    self.put(x, y, 255);
                }
            }
        }
    }

    fn apply(&self, img: &mut Img, c: [u8; 4], mode: Blend) {
        for my in 0..self.h {
            for mx in 0..self.w {
                let cov = self.cov[(my * self.w + mx) as usize];
                if cov == 0 {
                    continue;
                }
                let i = img.idx((self.x0 + mx) as u32, (self.y0 + my) as u32);
                blend_px(&mut img.px[i..i + 4], c, cov, mode);
            }
        }
    }
}

/// Abstand des Punktes q zur Strecke a–b.
fn dist_seg(q: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let (wx, wy) = (q.0 - a.0, q.1 - a.1);
    let l2 = vx * vx + vy * vy;
    let t = if l2 <= f32::EPSILON { 0.0 } else { ((wx * vx + wy * vy) / l2).clamp(0.0, 1.0) };
    let (px, py) = (a.0 + t * vx - q.0, a.1 + t * vy - q.1);
    (px * px + py * py).sqrt()
}

fn bbox(pts: &[(f32, f32)], pad: f32) -> (f32, f32, f32, f32) {
    let mut b = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in pts {
        b.0 = b.0.min(p.0);
        b.1 = b.1.min(p.1);
        b.2 = b.2.max(p.0);
        b.3 = b.3.max(p.1);
    }
    (b.0 - pad, b.1 - pad, b.2 + pad, b.3 + pad)
}

/// Linienzug (Stift, Leuchtstift). Ein einzelner Punkt wird ein Tupfen.
pub fn draw_polyline(img: &mut Img, pts: &[(f32, f32)], width: f32, c: [u8; 4], mode: Blend) {
    if pts.is_empty() {
        return;
    }
    let r = width.max(1.0) / 2.0;
    let b = bbox(pts, r + 2.0);
    let Some(mut m) = Mask::new(img, b.0, b.1, b.2, b.3) else { return };
    if pts.len() == 1 {
        m.segment(pts[0], pts[0], r);
    } else {
        for w in pts.windows(2) {
            m.segment(w[0], w[1], r);
        }
    }
    m.apply(img, c, mode);
}

/// Rechteck-Umriss zwischen zwei Eckpunkten.
pub fn draw_rect_outline(img: &mut Img, a: (f32, f32), b: (f32, f32), width: f32, c: [u8; 4]) {
    let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
    let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
    let pts = [(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)];
    draw_polyline(img, &pts, width, c, Blend::Normal);
}

/// Pfeil von a nach b (Spitze bei b).
pub fn draw_arrow(img: &mut Img, a: (f32, f32), b: (f32, f32), width: f32, c: [u8; 4]) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    if !(len > 1.0) {
        draw_polyline(img, &[a], width, c, Blend::Normal);
        return;
    }
    let r = width.max(1.0) / 2.0;
    let (ux, uy) = (dx / len, dy / len);
    let head = (width * 4.0).max(14.0).min(len);
    let half = head * 0.55;
    let base = (b.0 - ux * head, b.1 - uy * head);
    let left = (base.0 - uy * half, base.1 + ux * half);
    let right = (base.0 + uy * half, base.1 - ux * half);
    let shaft_end = (b.0 - ux * head * 0.5, b.1 - uy * head * 0.5);
    let bb = bbox(&[a, b, left, right], r + 2.0);
    let Some(mut m) = Mask::new(img, bb.0, bb.1, bb.2, bb.3) else { return };
    m.segment(a, shaft_end, r);
    m.triangle([b, left, right]);
    m.apply(img, c, Blend::Normal);
}

/// Mischt eine Deckungskarte (w × h, 0..=255 je Pixel, z. B. eine gerasterte
/// Schrift) an die Stelle (x0, y0) ein. Clippt an allen Rändern.
pub fn blend_coverage(img: &mut Img, x0: i32, y0: i32, w: u32, h: u32, cov: &[u8], c: [u8; 4]) {
    for y in 0..h as i64 {
        let iy = y0 as i64 + y;
        if iy < 0 || iy >= img.h as i64 {
            continue;
        }
        for x in 0..w as i64 {
            let ix = x0 as i64 + x;
            if ix < 0 || ix >= img.w as i64 {
                continue;
            }
            let Some(&cv) = cov.get((y * w as i64 + x) as usize) else { continue };
            if cv == 0 {
                continue;
            }
            let i = img.idx(ix as u32, iy as u32);
            blend_px(&mut img.px[i..i + 4], c, cv, Blend::Normal);
        }
    }
}

// ---------- Anzeige ----------

/// Massstab, damit ein Bild (w × h) in die Fläche (aw × ah) passt, höchstens `max`.
pub fn fit_scale(w: u32, h: u32, aw: f32, ah: f32, max: f32) -> f32 {
    if w == 0 || h == 0 || !(aw > 0.0) || !(ah > 0.0) {
        return 1.0;
    }
    (aw / w as f32).min(ah / h as f32).min(max).max(0.01)
}

// ---------- Dateinamen ----------

/// Dateiname ohne Endung, Windows-tauglich (keine Doppelpunkte).
/// Beispiel: "Bildschirmfoto 2026-10-18 14-22-05".
pub fn shot_stem(y: i32, mon: u32, d: u32, hh: u32, mm: u32, ss: u32) -> String {
    format!("Bildschirmfoto {:04}-{:02}-{:02} {:02}-{:02}-{:02}", y, mon, d, hh, mm, ss)
}

/// Erster freier Name: "stem.ext", sonst "stem (2).ext", "stem (3).ext" …
pub fn unique_name(stem: &str, ext: &str, exists: impl Fn(&str) -> bool) -> String {
    let first = format!("{stem}.{ext}");
    if !exists(&first) {
        return first;
    }
    for n in 2..10_000 {
        let c = format!("{stem} ({n}).{ext}");
        if !exists(&c) {
            return c;
        }
    }
    format!("{stem} (neu).{ext}")
}

// ---------- Rückgängig / Wiederholen ----------

/// Liste von Bearbeitungsschritten mit Rückgängig/Wiederholen.
#[derive(Clone, Debug)]
pub struct History<T> {
    done: Vec<T>,
    undone: Vec<T>,
}

impl<T> Default for History<T> {
    fn default() -> Self {
        History { done: Vec::new(), undone: Vec::new() }
    }
}

impl<T> History<T> {
    /// Neuer Schritt; was zum Wiederholen bereitlag, verfällt.
    pub fn push(&mut self, op: T) {
        self.done.push(op);
        self.undone.clear();
    }
    pub fn undo(&mut self) -> bool {
        match self.done.pop() {
            Some(op) => {
                self.undone.push(op);
                true
            }
            None => false,
        }
    }
    pub fn redo(&mut self) -> bool {
        match self.undone.pop() {
            Some(op) => {
                self.done.push(op);
                true
            }
            None => false,
        }
    }
    pub fn items(&self) -> &[T] {
        &self.done
    }
    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }
    pub fn is_empty(&self) -> bool {
        self.done.is_empty()
    }
}

// ---------- Monitore & Fenster ----------

/// Index des Monitors (x, y, w, h), der den Punkt enthält; sonst der mit dem
/// nächstgelegenen Mittelpunkt. `None` bei leerer Liste.
pub fn pick_monitor(rects: &[(i32, i32, u32, u32)], px: i32, py: i32) -> Option<usize> {
    let (px, py) = (px as i64, py as i64);
    for (i, r) in rects.iter().enumerate() {
        let (x, y) = (r.0 as i64, r.1 as i64);
        if px >= x && py >= y && px < x + r.2 as i64 && py < y + r.3 as i64 {
            return Some(i);
        }
    }
    rects
        .iter()
        .enumerate()
        .min_by_key(|(_, r)| {
            let cx = r.0 as i64 + r.2 as i64 / 2;
            let cy = r.1 as i64 + r.3 as i64 / 2;
            (cx - px).pow(2) + (cy - py).pow(2)
        })
        .map(|(i, _)| i)
}

/// Fensterrechteck (links, oben, rechts, unten; Bildschirmkoordinaten) als
/// Ausschnitt im Monitorbild (img_w × img_h) des Monitors `mon` (x, y, w, h).
/// Berücksichtigt, falls das Bild eine andere Grösse als der Monitor hat.
pub fn window_crop(win: (i32, i32, i32, i32), mon: (i32, i32, u32, u32), img_w: u32, img_h: u32) -> Option<Rechteck> {
    if mon.2 == 0 || mon.3 == 0 {
        return None;
    }
    let sx = img_w as f64 / mon.2 as f64;
    let sy = img_h as f64 / mon.3 as f64;
    let l = (win.0 as i64 - mon.0 as i64) as f64 * sx;
    let t = (win.1 as i64 - mon.1 as i64) as f64 * sy;
    let r = (win.2 as i64 - mon.0 as i64) as f64 * sx;
    let b = (win.3 as i64 - mon.1 as i64) as f64 * sy;
    norm_rect(l as f32, t as f32, r as f32, b as f32, img_w, img_h)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const BLACK: [u8; 4] = [0, 0, 0, 255];

    /// Bild mit lauter verschiedenen Pixeln (Verlauf).
    fn ramp(w: u32, h: u32) -> Img {
        let mut img = Img::new(w, h, BLACK);
        for y in 0..h {
            for x in 0..w {
                img.set(x, y, [(x * 7 % 256) as u8, (y * 11 % 256) as u8, ((x + y) * 3 % 256) as u8, 255]);
            }
        }
        img
    }

    #[test]
    fn rechteck_normalisieren() {
        assert_eq!(norm_rect(10.0, 20.0, 30.0, 25.0, 100, 100), Some((10, 20, 20, 5)));
        // umgekehrt aufgezogen
        assert_eq!(norm_rect(30.0, 25.0, 10.0, 20.0, 100, 100), Some((10, 20, 20, 5)));
        // ausserhalb wird beschnitten
        assert_eq!(norm_rect(-5.0, -5.0, 150.0, 8.0, 100, 50), Some((0, 0, 100, 8)));
        // Bruchteile nach aussen
        assert_eq!(norm_rect(1.5, 1.5, 3.2, 2.1, 10, 10), Some((1, 1, 3, 2)));
        // leer
        assert_eq!(norm_rect(5.0, 5.0, 5.0, 9.0, 10, 10), None);
        assert_eq!(norm_rect(200.0, 5.0, 300.0, 9.0, 100, 10), None);
        assert_eq!(norm_rect(f32::NAN, 0.0, 3.0, 3.0, 10, 10), None);
    }

    #[test]
    fn beschneiden() {
        assert_eq!(clip((90, 0, 50, 10), 100, 100), Some((90, 0, 10, 10)));
        assert_eq!(clip((100, 0, 5, 5), 100, 100), None);
        assert_eq!(clip((0, 0, 0, 5), 100, 100), None);
    }

    #[test]
    fn bgra_wandeln() {
        let img = Img::from_bgra(2, 1, &[1, 2, 3, 0, 4, 5, 6, 9]).unwrap();
        assert_eq!(img.get(0, 0), [3, 2, 1, 255]);
        assert_eq!(img.get(1, 0), [6, 5, 4, 255]);
        assert!(Img::from_bgra(2, 2, &[0; 8]).is_none());
        assert!(Img::from_bgra(0, 2, &[]).is_none());
    }

    #[test]
    fn zuschneiden() {
        let img = ramp(20, 10);
        let c = crop(&img, (5, 2, 4, 3)).unwrap();
        assert_eq!((c.w, c.h), (4, 3));
        assert_eq!(c.px.len(), 4 * 3 * 4);
        assert_eq!(c.get(0, 0), img.get(5, 2));
        assert_eq!(c.get(3, 2), img.get(8, 4));
        // über den Rand hinaus -> beschnitten
        let c = crop(&img, (18, 8, 10, 10)).unwrap();
        assert_eq!((c.w, c.h), (2, 2));
        assert_eq!(c.get(1, 1), img.get(19, 9));
        assert!(crop(&img, (20, 0, 1, 1)).is_none());
    }

    #[test]
    fn schwaerzen() {
        let mut img = ramp(10, 10);
        let before = img.get(0, 0);
        fill_rect(&mut img, (2, 2, 3, 3), BLACK);
        for y in 2..5 {
            for x in 2..5 {
                assert_eq!(img.get(x, y), BLACK);
            }
        }
        assert_eq!(img.get(0, 0), before);
        assert_eq!(img.get(5, 5), ramp(10, 10).get(5, 5));
    }

    #[test]
    fn verpixeln_zerstoert_pixel() {
        let orig = ramp(40, 40);
        let mut img = orig.clone();
        pixelate(&mut img, (8, 8, 20, 20), 10);
        // Jeder 10er-Block ist einfarbig.
        for (bx, by) in [(8, 8), (18, 8), (8, 18), (18, 18)] {
            let c = img.get(bx, by);
            for y in by..by + 10 {
                for x in bx..bx + 10 {
                    assert_eq!(img.get(x, y), c, "Block {bx},{by} nicht einheitlich");
                }
            }
        }
        // Die Originalwerte im Bereich sind weg (der Verlauf hatte verschiedene Pixel).
        let mut changed = 0;
        for y in 8..28 {
            for x in 8..28 {
                if img.get(x, y) != orig.get(x, y) {
                    changed += 1;
                }
            }
        }
        assert!(changed > 380, "nur {changed} Pixel verändert");
        // Ausserhalb unverändert.
        assert_eq!(img.get(7, 7), orig.get(7, 7));
        assert_eq!(img.get(28, 28), orig.get(28, 28));
        assert_eq!(img.get(30, 10), orig.get(30, 10));
    }

    #[test]
    fn verpixeln_mittelwert_und_randbloecke() {
        let mut img = Img::new(5, 1, BLACK);
        img.set(0, 0, [100, 0, 0, 255]);
        img.set(1, 0, [200, 0, 0, 255]);
        // Block 2: (0,1) -> 150; (2,3) -> 0; (4) Restblock -> 0
        pixelate(&mut img, (0, 0, 5, 1), 2);
        assert_eq!(img.get(0, 0), [150, 0, 0, 255]);
        assert_eq!(img.get(1, 0), [150, 0, 0, 255]);
        assert_eq!(img.get(2, 0), BLACK);
        // ausserhalb des Bildes: kein Absturz
        pixelate(&mut img, (10, 10, 5, 5), 2);
        pixelate(&mut img, (3, 0, 50, 50), 0);
    }

    #[test]
    fn blockgroesse() {
        assert_eq!(pixel_block(30, 20), 10);
        assert_eq!(pixel_block(600, 300), 48);
        assert_eq!(pixel_block(200, 120), 20);
    }

    #[test]
    fn linie_zeichnen() {
        let mut img = Img::new(50, 50, WHITE);
        draw_polyline(&mut img, &[(5.0, 25.0), (45.0, 25.0)], 4.0, [255, 0, 0, 255], Blend::Normal);
        assert_eq!(img.get(25, 25), [255, 0, 0, 255]);
        assert_eq!(img.get(25, 24), [255, 0, 0, 255]);
        assert_eq!(img.get(25, 10), WHITE);
        assert_eq!(img.get(25, 40), WHITE);
        // Tupfen
        let mut img = Img::new(20, 20, WHITE);
        draw_polyline(&mut img, &[(10.0, 10.0)], 6.0, BLACK, Blend::Normal);
        assert_eq!(img.get(10, 10), BLACK);
        assert_eq!(img.get(0, 0), WHITE);
        // ganz ausserhalb: kein Absturz, nichts verändert
        let mut img = Img::new(10, 10, WHITE);
        draw_polyline(&mut img, &[(-50.0, -50.0), (-40.0, -45.0)], 3.0, BLACK, Blend::Normal);
        assert_eq!(img, Img::new(10, 10, WHITE));
        draw_polyline(&mut img, &[(f32::NAN, 1.0)], 3.0, BLACK, Blend::Normal);
    }

    #[test]
    fn halbtransparent_nicht_doppelt() {
        // Zwei Segmente, die sich kreuzen: Kreuzungspunkt gleich wie einfacher Strich.
        let mut img = Img::new(40, 40, WHITE);
        let c = [255, 255, 0, 120];
        draw_polyline(&mut img, &[(5.0, 20.0), (35.0, 20.0), (20.0, 5.0), (20.0, 35.0)], 6.0, c, Blend::Normal);
        let cross = img.get(20, 20);
        let single = img.get(10, 20);
        assert_eq!(cross, single);
        assert!(single[2] < 255 && single[2] > 0, "halbtransparent erwartet: {single:?}");
    }

    #[test]
    fn leuchtstift_multipliziert() {
        let mut img = Img::new(20, 10, WHITE);
        img.set(10, 5, BLACK);
        draw_polyline(&mut img, &[(2.0, 5.0), (18.0, 5.0)], 6.0, [255, 230, 0, 255], Blend::Multiply);
        // Weiss wird gelb, Schwarz bleibt schwarz (Schrift lesbar).
        assert_eq!(img.get(5, 5), [255, 230, 0, 255]);
        assert_eq!(img.get(10, 5), BLACK);
    }

    #[test]
    fn rechteck_und_pfeil() {
        let mut img = Img::new(60, 60, WHITE);
        draw_rect_outline(&mut img, (40.0, 40.0), (10.0, 10.0), 2.0, BLACK);
        assert_eq!(img.get(25, 10), BLACK);
        assert_eq!(img.get(40, 25), BLACK);
        assert_eq!(img.get(25, 25), WHITE); // innen leer

        let mut img = Img::new(100, 60, WHITE);
        draw_arrow(&mut img, (10.0, 30.0), (90.0, 30.0), 3.0, BLACK);
        assert_eq!(img.get(30, 30), BLACK); // Schaft
        assert_eq!(img.get(85, 30), BLACK); // Spitze
        assert_eq!(img.get(80, 34), BLACK); // breiter Kopf
        assert_eq!(img.get(30, 36), WHITE); // Schaft ist schmal
        assert_eq!(img.get(95, 30), WHITE); // nach der Spitze
        // Nullpfeil: kein Absturz
        draw_arrow(&mut img, (5.0, 5.0), (5.0, 5.0), 3.0, BLACK);
        assert_eq!(img.get(5, 5), BLACK);
    }

    #[test]
    fn deckung_einmischen() {
        let mut img = Img::new(4, 4, WHITE);
        let cov = [255, 0, 128, 255];
        // teils ausserhalb (negativ)
        blend_coverage(&mut img, -1, -1, 2, 2, &cov, BLACK);
        assert_eq!(img.get(0, 0), BLACK); // cov[3]
        assert_eq!(img.get(1, 1), WHITE);
        blend_coverage(&mut img, 2, 2, 2, 2, &cov, BLACK);
        assert_eq!(img.get(2, 2), BLACK);
        assert_eq!(img.get(3, 2), WHITE);
        let g = img.get(2, 3);
        assert!(g[0] > 100 && g[0] < 160, "{g:?}");
        // zu kurze Karte: kein Absturz
        blend_coverage(&mut img, 0, 0, 4, 4, &[255], BLACK);
    }

    #[test]
    fn massstab() {
        assert_eq!(fit_scale(2000, 1000, 1000.0, 1000.0, 1.0), 0.5);
        assert_eq!(fit_scale(100, 100, 1000.0, 500.0, 1.0), 1.0);
        assert_eq!(fit_scale(100, 100, 1000.0, 500.0, 2.0), 2.0);
        assert_eq!(fit_scale(0, 100, 10.0, 10.0, 1.0), 1.0);
    }

    #[test]
    fn dateinamen() {
        assert_eq!(shot_stem(2026, 10, 8, 9, 5, 3), "Bildschirmfoto 2026-10-08 09-05-03");
        let taken = ["a.png", "a (2).png"];
        assert_eq!(unique_name("a", "png", |n| taken.contains(&n)), "a (3).png");
        assert_eq!(unique_name("a", "jpg", |n| taken.contains(&n)), "a.jpg");
        assert!(!shot_stem(2026, 1, 1, 1, 1, 1).contains(':'));
    }

    #[test]
    fn verlauf() {
        let mut h: History<u32> = History::default();
        assert!(!h.can_undo() && !h.can_redo() && h.is_empty());
        h.push(1);
        h.push(2);
        assert!(h.undo());
        assert_eq!(h.items(), &[1]);
        assert!(h.can_redo());
        assert!(h.redo());
        assert_eq!(h.items(), &[1, 2]);
        assert!(!h.redo());
        h.undo();
        h.push(3); // verwirft «2» zum Wiederholen
        assert!(!h.can_redo());
        assert_eq!(h.items(), &[1, 3]);
        h.undo();
        h.undo();
        assert!(!h.undo());
    }

    #[test]
    fn monitor_waehlen() {
        let mons = [(0, 0, 1920, 1080), (1920, 0, 2560, 1440), (-1280, 0, 1280, 1024)];
        assert_eq!(pick_monitor(&mons, 100, 100), Some(0));
        assert_eq!(pick_monitor(&mons, 2000, 1300), Some(1));
        assert_eq!(pick_monitor(&mons, -10, 10), Some(2));
        assert_eq!(pick_monitor(&mons, 5000, 100), Some(1)); // nächstgelegener
        assert_eq!(pick_monitor(&[], 0, 0), None);
    }

    #[test]
    fn fenster_ausschnitt() {
        // Fenster auf dem zweiten Monitor
        let r = window_crop((2020, 100, 2420, 400), (1920, 0, 2560, 1440), 2560, 1440);
        assert_eq!(r, Some((100, 100, 400, 300)));
        // teilweise ausserhalb -> beschnitten
        let r = window_crop((-20, -10, 300, 200), (0, 0, 1920, 1080), 1920, 1080);
        assert_eq!(r, Some((0, 0, 300, 200)));
        // Bild halb so gross wie der Monitor (Skalierung)
        let r = window_crop((200, 100, 600, 500), (0, 0, 2000, 1000), 1000, 500);
        assert_eq!(r, Some((100, 50, 200, 200)));
        // ganz woanders
        assert_eq!(window_crop((5000, 0, 5100, 100), (0, 0, 1920, 1080), 1920, 1080), None);
        assert_eq!(window_crop((0, 0, 10, 10), (0, 0, 0, 0), 10, 10), None);
    }
}
