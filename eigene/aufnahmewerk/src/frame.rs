//! Bildoperationen auf rohen BGRA8-Puffern (4 Bytes je Pixel: B, G, R, A).
//! Arbeitet mit einer frei angebbaren Zeilenlänge (stride) in Bytes, damit
//! Puffer mit Zeilen-Auffüllung (wie von Windows.Graphics.Capture) passen.
//! Keine Plattform-APIs, keine Oberfläche – nur Standardbibliothek, testbar.

/// Schneidet aus `src` das Rechteck (x, y, w, h) heraus und gibt einen dicht
/// gepackten Puffer (Zeilenlänge w*4) zurück. Clippt an den Quellgrenzen.
pub fn crop(
    src: &[u8],
    src_stride: usize,
    src_w: u32,
    src_h: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) -> Vec<u8> {
    let (sw, sh) = (src_w as usize, src_h as usize);
    let (x, y) = (x as usize, y as usize);
    // tatsächlich verfügbares Rechteck
    let w = (w as usize).min(sw.saturating_sub(x));
    let h = (h as usize).min(sh.saturating_sub(y));
    let dst_stride = w * 4;
    let mut out = vec![0u8; dst_stride * h];
    for row in 0..h {
        let s = (y + row) * src_stride + x * 4;
        let d = row * dst_stride;
        out[d..d + dst_stride].copy_from_slice(&src[s..s + dst_stride]);
    }
    out
}

/// Kopiert das undurchsichtige Overlay `ov` (dicht gepackt, Zeilenlänge ov_w*4)
/// an die Position (px, py) in `dst`. Clippt sauber an allen Rändern, auch bei
/// negativen Positionen.
pub fn composite(
    dst: &mut [u8],
    dst_stride: usize,
    dst_w: u32,
    dst_h: u32,
    ov: &[u8],
    ov_w: u32,
    ov_h: u32,
    px: i32,
    py: i32,
) {
    let ov_stride = ov_w as usize * 4;
    for row in 0..ov_h as i32 {
        let dy = py + row;
        if dy < 0 || dy >= dst_h as i32 {
            continue;
        }
        for col in 0..ov_w as i32 {
            let dx = px + col;
            if dx < 0 || dx >= dst_w as i32 {
                continue;
            }
            let s = row as usize * ov_stride + col as usize * 4;
            let d = dy as usize * dst_stride + dx as usize * 4;
            dst[d..d + 4].copy_from_slice(&ov[s..s + 4]);
        }
    }
}

/// Zeichnet einen Rahmen (Rechteck-Umriss) der Dicke `t` in `color` (BGRA).
pub fn draw_border(
    dst: &mut [u8],
    dst_stride: usize,
    dst_w: u32,
    dst_h: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    t: u32,
    color: [u8; 4],
) {
    let t = t as i32;
    for dy in y..y + h as i32 {
        if dy < 0 || dy >= dst_h as i32 {
            continue;
        }
        for dx in x..x + w as i32 {
            if dx < 0 || dx >= dst_w as i32 {
                continue;
            }
            let on_edge = dx < x + t || dx >= x + w as i32 - t || dy < y + t || dy >= y + h as i32 - t;
            if on_edge {
                let d = dy as usize * dst_stride + dx as usize * 4;
                dst[d..d + 4].copy_from_slice(&color);
            }
        }
    }
}

#[inline]
fn blend_px(px: &mut [u8], color: [u8; 4], a: u16) {
    // a: 0..=255 Deckkraft der Farbe
    for k in 0..3 {
        let bg = px[k] as u16;
        let fg = color[k] as u16;
        px[k] = ((bg * (255 - a) + fg * a) / 255) as u8;
    }
    px[3] = 255;
}

/// Klick-/Zeiger-Markierung: gefüllte, halbdurchsichtige Scheibe mit kräftigerem
/// Ring am Rand. Mittelpunkt (cx, cy), Radius `r`. `color` in BGRA.
pub fn draw_click_marker(
    dst: &mut [u8],
    dst_stride: usize,
    dst_w: u32,
    dst_h: u32,
    cx: i32,
    cy: i32,
    r: i32,
    color: [u8; 4],
) {
    let r2 = r * r;
    let inner = ((r - r / 4).max(0)) * (r - r / 4).max(0); // Ringbeginn
    for dy in -r..=r {
        let yy = cy + dy;
        if yy < 0 || yy >= dst_h as i32 {
            continue;
        }
        for dx in -r..=r {
            let xx = cx + dx;
            if xx < 0 || xx >= dst_w as i32 {
                continue;
            }
            let d2 = dx * dx + dy * dy;
            if d2 > r2 {
                continue;
            }
            let a: u16 = if d2 >= inner { 230 } else { 90 }; // Ring kräftig, Füllung zart
            let off = yy as usize * dst_stride + xx as usize * 4;
            blend_px(&mut dst[off..off + 4], color, a);
        }
    }
}

/// Skaliert ein BGRA-Bild per Nearest-Neighbor auf (dw, dh). Für das
/// Webcam-Overlay ausreichend und ohne Fremdbibliothek.
pub fn scale_nn(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    dw: u32,
    dh: u32,
) -> Vec<u8> {
    let (sw, sh) = (src_w as usize, src_h as usize);
    let src_stride = sw * 4;
    let dst_stride = dw as usize * 4;
    let mut out = vec![0u8; dst_stride * dh as usize];
    if dw == 0 || dh == 0 {
        return out;
    }
    for dy in 0..dh as usize {
        let sy = dy * sh / dh as usize;
        for dx in 0..dw as usize {
            let sx = dx * sw / dw as usize;
            let s = sy * src_stride + sx * 4;
            let d = dy * dst_stride + dx * 4;
            out[d..d + 4].copy_from_slice(&src[s..s + 4]);
        }
    }
    out
}

/// Bestimmt die obere linke Ecke des Overlays in einem (dst_w × dst_h)-Bild
/// für die gewählte Ecke, mit Rand `margin`.
pub fn corner_pos(
    corner: crate::model::Corner,
    dst_w: u32,
    dst_h: u32,
    ov_w: u32,
    ov_h: u32,
    margin: i32,
) -> (i32, i32) {
    use crate::model::Corner::*;
    let right = dst_w as i32 - ov_w as i32 - margin;
    let bottom = dst_h as i32 - ov_h as i32 - margin;
    match corner {
        ObenLinks => (margin, margin),
        ObenRechts => (right, margin),
        UntenLinks => (margin, bottom),
        UntenRechts => (right, bottom),
    }
}
