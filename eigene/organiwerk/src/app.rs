//! Oberfläche von Organiwerk (egui): Werkzeugleiste, Zeichenfläche, Eigenschaften rechts (bzw. unten).

use std::sync::mpsc::{channel, Receiver, Sender};

use egui::{pos2, vec2, Color32, FontId, Key, Modifiers, Pos2, Rect, RichText, Sense, Stroke};

use crate::model::{self, Align, Doc, Kind, P, PALETTE};
use crate::save;
use crate::sprache::{t, tf};
use crate::thema;

/// Programmfarbe auf der (immer hellen) Zeichenfläche: Auswahl, Griffe, Verbindungslinie.
const ACCENT: Color32 = Color32::from_rgb(0x2F, 0x5F, 0xB3);
/// Zeichenfläche («Papier»): bleibt in beiden Designs hell, wie das Diagramm im SVG.
const PAPER: Color32 = Color32::from_rgb(0xFA, 0xFB, 0xFC);

/// Programmfarbe für Text in der Oberfläche (im dunklen Design heller, damit lesbar).
fn accent_text() -> Color32 {
    thema::pick(ACCENT, Color32::from_rgb(0x7F, 0xA8, 0xF0))
}
const INK: Color32 = Color32::from_rgb(0x3A, 0x46, 0x50);
const STORAGE_KEY: &str = "organiwerk_v1";
const GRID: f32 = 10.0;

fn col(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Select,
    Connect,
}

#[derive(Clone)]
enum Drag {
    None,
    Pan,
    Move { start: P, orig: Vec<(u64, f32, f32)> },
    Resize { id: u64, corner: u8, orig: (f32, f32, f32, f32) },
    Rubber { start: P },
    Link { from: u64 },
}

/// Umrechnung zwischen Dokument- und Bildschirmkoordinaten.
#[derive(Clone, Copy)]
struct View {
    origin: Pos2,
    pan: P,
    zoom: f32,
}

impl View {
    fn s(&self, p: P) -> Pos2 {
        pos2(self.origin.x + (p.0 - self.pan.0) * self.zoom, self.origin.y + (p.1 - self.pan.1) * self.zoom)
    }
    fn w(&self, q: Pos2) -> P {
        ((q.x - self.origin.x) / self.zoom + self.pan.0, (q.y - self.origin.y) / self.zoom + self.pan.1)
    }
    fn rect(&self, s: &model::Shape) -> Rect {
        Rect::from_min_max(self.s((s.x, s.y)), self.s((s.right(), s.bottom())))
    }
}

pub struct App {
    doc: Doc,
    base: Doc,
    undo: Vec<Doc>,
    redo: Vec<Doc>,
    gesture_open: bool,
    sel: Vec<u64>,
    sel_edge: Option<u64>,
    pan: P,
    zoom: f32,
    fitted: bool,
    canvas: Rect,
    tool: Tool,
    drag: Drag,
    connect_from: Option<u64>,
    snap: bool,
    show_grid: bool,
    focus_text: bool,
    typing: bool,
    spawn: usize,
    status: String,
    open_tx: Sender<(String, Vec<u8>)>,
    open_rx: Receiver<(String, Vec<u8>)>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Hell/Dunkel wie im Werkbank-Launcher gewählt (nicht nach dem System).
        cc.egui_ctx.set_theme(if thema::is_dark() { egui::Theme::Dark } else { egui::Theme::Light });
        thema::apply(&cc.egui_ctx);
        cc.egui_ctx.style_mut(|s| {
            s.visuals.selection.bg_fill = thema::pick(Color32::from_rgb(0xC7, 0xD7, 0xF2), Color32::from_rgb(0x2D, 0x4A, 0x7A));
            s.visuals.selection.stroke = Stroke::new(1.0, accent_text());
            s.visuals.hyperlink_color = accent_text();
        });
        let doc = cc
            .storage
            .and_then(|s| s.get_string(STORAGE_KEY))
            .and_then(|t| Doc::from_json_str(&t).ok())
            .unwrap_or_else(Doc::example);
        let (open_tx, open_rx) = channel();
        App {
            base: doc.clone(),
            doc,
            undo: Vec::new(),
            redo: Vec::new(),
            gesture_open: false,
            sel: Vec::new(),
            sel_edge: None,
            pan: (0.0, 0.0),
            zoom: 1.0,
            fitted: false,
            canvas: Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 600.0)),
            tool: Tool::Select,
            drag: Drag::None,
            connect_from: None,
            snap: true,
            show_grid: true,
            focus_text: false,
            typing: false,
            spawn: 0,
            status: String::new(),
            open_tx,
            open_rx,
        }
    }

    /// Vorgabetext einer neuen Form in der gewählten Sprache (eingegebene Texte bleiben, wie sie sind).
    fn localize_new(&mut self, id: u64) {
        if let Some(s) = self.doc.shape_mut(id) {
            let de: Option<&'static str> = match s.text.as_str() {
                "Name\nFunktion" => Some("Name\nFunktion"),
                "Entscheid?" => Some("Entscheid?"),
                "Abteilung" => Some("Abteilung"),
                "Text" => Some("Text"),
                _ => None,
            };
            if let Some(de) = de {
                s.text = t(de).to_string();
            }
        }
    }

    fn view(&self, rect: Rect) -> View {
        View { origin: rect.min, pan: self.pan, zoom: self.zoom }
    }

    fn snap_v(&self, x: f32) -> f32 {
        if self.snap {
            (x / GRID).round() * GRID
        } else {
            x
        }
    }

    // ------------------------------------------------------------ Befehle

    fn do_undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.doc, prev));
            self.base = self.doc.clone();
            self.fix_selection();
        }
    }

    fn do_redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.doc, next));
            self.base = self.doc.clone();
            self.fix_selection();
        }
    }

    fn fix_selection(&mut self) {
        let doc = &self.doc;
        self.sel.retain(|id| doc.shape(*id).is_some());
        if self.sel_edge.map_or(false, |e| doc.edge(e).is_none()) {
            self.sel_edge = None;
        }
    }

    fn fit(&mut self, rect: Rect) {
        match self.doc.bounds(&[]) {
            Some((x0, y0, x1, y1)) => {
                let (bw, bh) = ((x1 - x0).max(50.0), (y1 - y0).max(50.0));
                self.zoom = ((rect.width() / bw).min(rect.height() / bh) * 0.9).clamp(0.2, 1.2);
                let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
                self.pan = (cx - rect.width() / 2.0 / self.zoom, cy - rect.height() / 2.0 / self.zoom);
            }
            None => {
                self.zoom = 1.0;
                self.pan = (-rect.width() / 2.0, -rect.height() / 2.0);
            }
        }
    }

    fn add_at_center(&mut self, kind: Kind) {
        let v = self.view(self.canvas);
        let c = v.w(self.canvas.center());
        let off = (self.spawn % 6) as f32 * 24.0;
        self.spawn += 1;
        let id = self.doc.add_shape(kind, self.snap_v(c.0 + off), self.snap_v(c.1 + off));
        self.localize_new(id);
        self.sel = vec![id];
        self.sel_edge = None;
        self.focus_text = true;
    }

    fn delete_selection(&mut self) {
        if let Some(e) = self.sel_edge.take() {
            self.doc.remove_edge(e);
        }
        let ids = std::mem::take(&mut self.sel);
        self.doc.remove_shapes(&ids);
    }

    fn add_child(&mut self) {
        if let Some(id) = self.sel.first().copied() {
            if let Some(n) = self.doc.add_child(id) {
                self.localize_new(n);
                self.sel = vec![n];
                self.focus_text = true;
            }
        }
    }

    fn add_sibling(&mut self) {
        if let Some(id) = self.sel.first().copied() {
            if let Some(n) = self.doc.add_sibling(id) {
                self.localize_new(n);
                self.sel = vec![n];
                self.focus_text = true;
            }
        }
    }

    fn duplicate(&mut self) {
        let ids = self.sel.clone();
        if !ids.is_empty() {
            self.sel = self.doc.duplicate(&ids);
        }
    }

    fn arrange(&mut self) {
        let sel = self.sel.clone();
        self.doc.layout_tree(&sel, 30.0, 60.0);
    }

    fn open_file(&mut self, ctx: &egui::Context) {
        save::open_file(self.open_tx.clone(), ctx.clone());
    }

    fn save_file(&mut self) {
        save::save_bytes("organigramm.organi", self.doc.to_json_string().as_bytes());
    }

    fn export_svg(&mut self) {
        save::save_bytes("organigramm.svg", self.doc.to_svg().as_bytes());
    }

    fn zoom_by(&mut self, f: f32) {
        let c = self.canvas.center();
        let v = self.view(self.canvas);
        let before = v.w(c);
        self.zoom = (self.zoom * f).clamp(0.15, 3.0);
        self.pan = (before.0 - (c.x - self.canvas.min.x) / self.zoom, before.1 - (c.y - self.canvas.min.y) / self.zoom);
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.typing {
            return;
        }
        let k = |ctx: &egui::Context, m: Modifiers, key: Key| ctx.input_mut(|i| i.consume_key(m, key));
        if k(ctx, Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || k(ctx, Modifiers::COMMAND, Key::Y) {
            self.do_redo();
        }
        if k(ctx, Modifiers::COMMAND, Key::Z) {
            self.do_undo();
        }
        if k(ctx, Modifiers::COMMAND, Key::D) {
            self.duplicate();
        }
        if k(ctx, Modifiers::COMMAND, Key::S) {
            self.save_file();
        }
        if k(ctx, Modifiers::COMMAND, Key::A) {
            self.sel = self.doc.shapes.iter().map(|s| s.id).collect();
        }
        if k(ctx, Modifiers::COMMAND | Modifiers::SHIFT, Key::Enter) {
            self.add_sibling();
        } else if k(ctx, Modifiers::COMMAND, Key::Enter) {
            self.add_child();
        }
        if k(ctx, Modifiers::NONE, Key::Delete) || k(ctx, Modifiers::NONE, Key::Backspace) {
            self.delete_selection();
        }
        if k(ctx, Modifiers::NONE, Key::Escape) {
            self.sel.clear();
            self.sel_edge = None;
            self.connect_from = None;
            self.tool = Tool::Select;
        }
        for (key, dx, dy) in [(Key::ArrowLeft, -1.0, 0.0), (Key::ArrowRight, 1.0, 0.0), (Key::ArrowUp, 0.0, -1.0), (Key::ArrowDown, 0.0, 1.0)] {
            let step = if k(ctx, Modifiers::SHIFT, key) {
                10.0
            } else if k(ctx, Modifiers::NONE, key) {
                1.0
            } else {
                continue;
            };
            let sel = self.sel.clone();
            for id in sel {
                if let Some(s) = self.doc.shape_mut(id) {
                    s.x += dx * step;
                    s.y += dy * step;
                }
            }
        }
    }
}

// ================================================================ Zeichnen

fn wrap_text(painter: &egui::Painter, text: &str, font: &FontId, max_w: f32) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let cand = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if line.is_empty() || painter.layout_no_wrap(cand.clone(), font.clone(), Color32::WHITE).size().x <= max_w {
                line = cand;
            } else {
                out.push(std::mem::take(&mut line));
                line = word.to_string();
            }
        }
        out.push(line);
    }
    out
}

fn paint_text(painter: &egui::Painter, s: &model::Shape, r: Rect, zoom: f32) {
    if s.text.trim().is_empty() || s.font * zoom < 4.0 {
        return;
    }
    let color = col(if matches!(s.kind, Kind::Text | Kind::Group) { 0x151B1F } else { model::text_color(s.fill) });
    if s.kind == Kind::Group {
        let g = painter.layout_no_wrap(s.text.replace('\n', " "), FontId::proportional(s.font * zoom), color);
        painter.galley(pos2(r.min.x + 12.0 * zoom, r.min.y + 8.0 * zoom), g, color);
        return;
    }
    let factor = if matches!(s.kind, Kind::Ellipse | Kind::Diamond) { 0.68 } else { 1.0 };
    let max_w = ((r.width() - 16.0 * zoom) * factor).max(10.0);
    let mut galleys = Vec::new();
    if s.kind == Kind::Org {
        let (name, role) = match s.text.split_once('\n') {
            Some((a, b)) => (a, b),
            None => (s.text.as_str(), ""),
        };
        let f1 = FontId::proportional(s.font * 1.08 * zoom);
        for l in wrap_text(painter, name, &f1, max_w) {
            galleys.push(painter.layout_no_wrap(l, f1.clone(), color));
        }
        let f2 = FontId::proportional(s.font * 0.88 * zoom);
        let weak = color.gamma_multiply(0.8);
        if !role.is_empty() {
            for l in wrap_text(painter, role, &f2, max_w) {
                galleys.push(painter.layout_no_wrap(l, f2.clone(), weak));
            }
        }
    } else {
        let f = FontId::proportional(s.font * zoom);
        for l in wrap_text(painter, &s.text, &f, max_w) {
            galleys.push(painter.layout_no_wrap(l, f.clone(), color));
        }
    }
    let total: f32 = galleys.iter().map(|g| g.size().y).sum();
    let mut y = r.center().y - total / 2.0;
    let clip = painter.with_clip_rect(r.expand(2.0));
    for g in galleys {
        let (w, h) = (g.size().x, g.size().y);
        clip.galley(pos2(r.center().x - w / 2.0, y), g, color);
        y += h;
    }
}

fn paint_shape(painter: &egui::Painter, s: &model::Shape, v: View, selected: bool) {
    let r = v.rect(s);
    let fill = col(s.fill);
    let stroke = Stroke::new((1.5 * v.zoom).max(1.0), if s.kind == Kind::Group { Color32::from_rgb(0x8E, 0x9A, 0xA3) } else { INK });
    match s.kind {
        Kind::Rect => {
            painter.rect(r, 0.0, fill, stroke);
        }
        Kind::Rounded | Kind::Org => {
            painter.rect(r, 10.0 * v.zoom, fill, stroke);
        }
        Kind::Group => {
            painter.rect(r, 8.0 * v.zoom, fill.gamma_multiply(0.6), stroke);
        }
        Kind::Ellipse => {
            let (c, rx, ry) = (r.center(), r.width() / 2.0, r.height() / 2.0);
            let pts: Vec<Pos2> = (0..56).map(|i| {
                let a = i as f32 / 56.0 * std::f32::consts::TAU;
                pos2(c.x + rx * a.cos(), c.y + ry * a.sin())
            }).collect();
            painter.add(egui::Shape::convex_polygon(pts, fill, stroke));
        }
        Kind::Diamond => {
            let c = r.center();
            painter.add(egui::Shape::convex_polygon(vec![pos2(c.x, r.min.y), pos2(r.max.x, c.y), pos2(c.x, r.max.y), pos2(r.min.x, c.y)], fill, stroke));
        }
        Kind::Text => {}
    }
    paint_text(painter, s, r, v.zoom);
    if selected {
        painter.rect_stroke(r.expand(3.0), 4.0, Stroke::new(2.0, ACCENT));
    }
}

fn paint_arrow(painter: &egui::Painter, from: Pos2, tip: Pos2, color: Color32, size: f32) {
    let d = (tip - from).normalized();
    if !d.x.is_finite() || !d.y.is_finite() {
        return;
    }
    let n = vec2(-d.y, d.x);
    let base = tip - d * size;
    painter.add(egui::Shape::convex_polygon(vec![tip, base + n * size * 0.45, base - n * size * 0.45], color, Stroke::NONE));
}

impl App {
    fn paint_doc(&self, painter: &egui::Painter, v: View, rect: Rect) {
        painter.rect_filled(rect, 0.0, PAPER);
        if self.show_grid {
            let step = GRID * 2.0 * v.zoom;
            if step >= 8.0 {
                let w0 = v.w(rect.min);
                let (gx0, gy0) = ((w0.0 / (GRID * 2.0)).floor() * GRID * 2.0, (w0.1 / (GRID * 2.0)).floor() * GRID * 2.0);
                let nx = (rect.width() / step) as i32 + 2;
                let ny = (rect.height() / step) as i32 + 2;
                for ix in 0..nx {
                    for iy in 0..ny {
                        let p = v.s((gx0 + ix as f32 * GRID * 2.0, gy0 + iy as f32 * GRID * 2.0));
                        painter.circle_filled(p, 1.0, Color32::from_rgb(0xD5, 0xDB, 0xE0));
                    }
                }
            }
        }
        for s in self.doc.shapes.iter().filter(|s| s.kind == Kind::Group) {
            paint_shape(painter, s, v, self.sel.contains(&s.id));
        }
        for e in &self.doc.edges {
            let Some(pts) = self.doc.edge_points(e) else { continue };
            let selected = self.sel_edge == Some(e.id);
            let color = if selected { ACCENT } else { INK };
            let sp: Vec<Pos2> = pts.iter().map(|p| v.s(*p)).collect();
            painter.add(egui::Shape::line(sp.clone(), Stroke::new(if selected { 2.5 } else { (1.5 * v.zoom).max(1.0) }, color)));
            if e.arrow && sp.len() >= 2 {
                paint_arrow(painter, sp[sp.len() - 2], sp[sp.len() - 1], color, (11.0 * v.zoom).max(7.0));
            }
        }
        for s in self.doc.shapes.iter().filter(|s| s.kind != Kind::Group) {
            paint_shape(painter, s, v, self.sel.contains(&s.id));
        }
        for e in &self.doc.edges {
            if e.label.is_empty() {
                continue;
            }
            if let Some(pts) = self.doc.edge_points(e) {
                let m = v.s(model::polyline_mid(&pts));
                let g = painter.layout_no_wrap(e.label.clone(), FontId::proportional((12.0 * v.zoom).max(8.0)), INK);
                let r = Rect::from_center_size(m, g.size() + vec2(8.0, 4.0));
                painter.rect_filled(r, 3.0, PAPER);
                painter.galley(pos2(m.x - g.size().x / 2.0, m.y - g.size().y / 2.0), g, INK);
            }
        }
        // Griffe zum Vergrössern
        if self.sel.len() == 1 {
            if let Some(s) = self.doc.shape(self.sel[0]) {
                for c in self.corners(v, s) {
                    painter.rect(Rect::from_center_size(c, vec2(9.0, 9.0)), 2.0, Color32::WHITE, Stroke::new(1.5, ACCENT));
                }
            }
        }
        // Rubber-Band und Verbindungslinie
        let pointer = painter.ctx().input(|i| i.pointer.hover_pos());
        match (&self.drag, pointer) {
            (Drag::Rubber { start }, Some(p)) => {
                let r = Rect::from_two_pos(v.s(*start), p);
                painter.rect(r, 0.0, ACCENT.gamma_multiply(0.12), Stroke::new(1.0, ACCENT));
            }
            (Drag::Link { from }, Some(p)) => {
                if let Some(s) = self.doc.shape(*from) {
                    painter.line_segment([v.s(s.center()), p], Stroke::new(2.0, ACCENT));
                }
            }
            _ => {}
        }
        if let (Some(f), Some(p)) = (self.connect_from, pointer) {
            if let Some(s) = self.doc.shape(f) {
                painter.line_segment([v.s(s.center()), p], Stroke::new(2.0, ACCENT));
            }
        }
        if self.doc.shapes.is_empty() {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, t("Doppelklick auf die Fläche legt eine Karte an.\nOder oben bei «Einfügen» eine Form wählen."), FontId::proportional(15.0), Color32::from_rgb(0x5B, 0x61, 0x6E));
        }
    }

    fn corners(&self, v: View, s: &model::Shape) -> [Pos2; 4] {
        let r = v.rect(s);
        [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()]
    }

    fn handle_at(&self, v: View, pos: Pos2) -> Option<(u64, u8)> {
        if self.sel.len() != 1 {
            return None;
        }
        let s = self.doc.shape(self.sel[0])?;
        self.corners(v, s).iter().position(|c| (c.x - pos.x).abs() <= 8.0 && (c.y - pos.y).abs() <= 8.0).map(|i| (s.id, i as u8))
    }

    // ------------------------------------------------------------ Eingaben auf der Fläche

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = resp.rect;
        self.canvas = rect;
        if !self.fitted {
            self.fitted = true;
            self.fit(rect);
        }

        // Zoomen (Ctrl+Rad, Pinch) und Verschieben (Rad, Zweifinger)
        if resp.hovered() {
            let (zd, sc, hover) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta, i.pointer.hover_pos()));
            if (zd - 1.0).abs() > 0.001 {
                if let Some(h) = hover {
                    let before = self.view(rect).w(h);
                    self.zoom = (self.zoom * zd).clamp(0.15, 3.0);
                    self.pan = (before.0 - (h.x - rect.min.x) / self.zoom, before.1 - (h.y - rect.min.y) / self.zoom);
                }
            }
            if sc != vec2(0.0, 0.0) {
                self.pan.0 -= sc.x / self.zoom;
                self.pan.1 -= sc.y / self.zoom;
            }
        }
        let v = self.view(rect);
        let (shift, middle, secondary, space) = ui.input(|i| (i.modifiers.shift, i.pointer.middle_down(), i.pointer.secondary_down(), i.key_down(Key::Space)));

        if resp.drag_started() {
            if let Some(pos) = ui.input(|i| i.pointer.press_origin()) {
                let wp = v.w(pos);
                self.drag = if middle || secondary || space {
                    Drag::Pan
                } else if let Some((id, corner)) = self.handle_at(v, pos) {
                    let s = self.doc.shape(id).unwrap();
                    Drag::Resize { id, corner, orig: (s.x, s.y, s.w, s.h) }
                } else if self.tool == Tool::Connect {
                    match self.doc.hit_shape(wp) {
                        Some(id) => Drag::Link { from: id },
                        None => Drag::Rubber { start: wp },
                    }
                } else if let Some(id) = self.doc.hit_shape(wp) {
                    if !self.sel.contains(&id) {
                        if !shift {
                            self.sel.clear();
                        }
                        self.sel.push(id);
                    }
                    self.sel_edge = None;
                    // Gruppen nehmen ihren Inhalt mit
                    let mut moving: Vec<u64> = self.sel.clone();
                    for g in self.sel.clone() {
                        if let Some(gs) = self.doc.shape(g).filter(|s| s.kind == Kind::Group).cloned() {
                            for s in &self.doc.shapes {
                                if s.id != g && s.x >= gs.x && s.y >= gs.y && s.right() <= gs.right() && s.bottom() <= gs.bottom() && !moving.contains(&s.id) {
                                    moving.push(s.id);
                                }
                            }
                        }
                    }
                    let orig = moving.iter().filter_map(|i| self.doc.shape(*i)).map(|s| (s.id, s.x, s.y)).collect();
                    Drag::Move { start: wp, orig }
                } else {
                    if !shift {
                        self.sel.clear();
                        self.sel_edge = None;
                    }
                    Drag::Rubber { start: wp }
                };
            }
        }

        if resp.dragged() {
            let cur = resp.interact_pointer_pos().map(|p| v.w(p));
            match self.drag.clone() {
                Drag::Pan => {
                    let d = resp.drag_delta();
                    self.pan.0 -= d.x / self.zoom;
                    self.pan.1 -= d.y / self.zoom;
                }
                Drag::Move { start, orig } => {
                    if let (Some(c), Some(first)) = (cur, orig.first()) {
                        let dx = self.snap_v(first.1 + c.0 - start.0) - first.1;
                        let dy = self.snap_v(first.2 + c.1 - start.1) - first.2;
                        for (id, x, y) in orig {
                            if let Some(s) = self.doc.shape_mut(id) {
                                s.x = x + dx;
                                s.y = y + dy;
                            }
                        }
                    }
                }
                Drag::Resize { id, corner, orig } => {
                    if let Some(c) = cur {
                        let (x, y, w, h) = orig;
                        let (cx, cy) = (self.snap_v(c.0), self.snap_v(c.1));
                        let (ax, ay) = match corner {
                            0 => (x + w, y + h),
                            1 => (x, y + h),
                            2 => (x, y),
                            _ => (x + w, y),
                        };
                        let (nx0, nx1) = (cx.min(ax), cx.max(ax));
                        let (ny0, ny1) = (cy.min(ay), cy.max(ay));
                        if let Some(s) = self.doc.shape_mut(id) {
                            if nx1 - nx0 >= 24.0 && ny1 - ny0 >= 20.0 {
                                s.x = nx0;
                                s.y = ny0;
                                s.w = nx1 - nx0;
                                s.h = ny1 - ny0;
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if resp.drag_stopped() {
            let end = ui.input(|i| i.pointer.hover_pos()).map(|p| v.w(p));
            match std::mem::replace(&mut self.drag, Drag::None) {
                Drag::Rubber { start } => {
                    if let Some(e) = end {
                        let found = self.doc.shapes_in_rect((start.0, start.1, e.0, e.1));
                        for id in found {
                            if !self.sel.contains(&id) {
                                self.sel.push(id);
                            }
                        }
                    }
                }
                Drag::Link { from } => {
                    if let Some(to) = end.and_then(|e| self.doc.hit_shape(e)) {
                        self.doc.connect(from, to, true, true);
                    }
                }
                _ => {}
            }
        }

        if resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let wp = v.w(pos);
                let hit = self.doc.hit_shape(wp);
                if self.tool == Tool::Connect {
                    if let Some(id) = hit {
                        match self.connect_from.take() {
                            Some(from) => {
                                self.doc.connect(from, id, true, true);
                            }
                            None => self.connect_from = Some(id),
                        }
                    } else {
                        self.connect_from = None;
                    }
                } else if let Some(id) = hit {
                    self.sel_edge = None;
                    if shift {
                        if let Some(i) = self.sel.iter().position(|x| *x == id) {
                            self.sel.remove(i);
                        } else {
                            self.sel.push(id);
                        }
                    } else {
                        self.sel = vec![id];
                    }
                } else if let Some(e) = self.doc.hit_edge(wp, 6.0 / self.zoom) {
                    self.sel.clear();
                    self.sel_edge = Some(e);
                } else {
                    self.sel.clear();
                    self.sel_edge = None;
                }
            }
        }

        if resp.double_clicked() && self.tool == Tool::Select {
            if let Some(pos) = resp.interact_pointer_pos() {
                let wp = v.w(pos);
                match self.doc.hit_shape(wp) {
                    Some(id) => {
                        self.sel = vec![id];
                        self.focus_text = true;
                    }
                    None => {
                        let id = self.doc.add_shape(Kind::Org, self.snap_v(wp.0), self.snap_v(wp.1));
                        self.localize_new(id);
                        self.sel = vec![id];
                        self.sel_edge = None;
                        self.focus_text = true;
                    }
                }
            }
        }

        let v = self.view(rect);
        self.paint_doc(&painter, v, rect);
    }
}

// ================================================================ Leisten

impl App {
    fn ui_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Organiwerk").strong().color(accent_text()));
            if ui.button(t("Neu")).clicked() {
                self.doc = Doc::default();
                self.sel.clear();
                self.sel_edge = None;
                self.fitted = false;
            }
            if ui.button(t("Öffnen")).clicked() {
                self.open_file(&ui.ctx().clone());
            }
            if ui.button(t("Speichern")).clicked() {
                self.save_file();
            }
            if ui.button(t("Als SVG")).clicked() {
                self.export_svg();
            }
            ui.separator();
            if ui.add_enabled(!self.undo.is_empty(), egui::Button::new(t("Rückgängig"))).clicked() {
                self.do_undo();
            }
            if ui.add_enabled(!self.redo.is_empty(), egui::Button::new(t("Wiederholen"))).clicked() {
                self.do_redo();
            }
            ui.separator();
            ui.selectable_value(&mut self.tool, Tool::Select, t("Auswählen"));
            ui.selectable_value(&mut self.tool, Tool::Connect, t("Verbinden"));
            ui.separator();
            if ui.button("−").clicked() {
                self.zoom_by(1.0 / 1.2);
            }
            ui.label(format!("{:.0} %", self.zoom * 100.0));
            if ui.button("+").clicked() {
                self.zoom_by(1.2);
            }
            if ui.button(t("Einpassen")).clicked() {
                self.fitted = false;
            }
            ui.checkbox(&mut self.snap, t("Einrasten"));
            ui.checkbox(&mut self.show_grid, t("Raster"));
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(t("Einfügen:"));
            for k in Kind::ALL {
                if ui.button(t(k.label())).clicked() {
                    self.add_at_center(k);
                }
            }
            ui.separator();
            if ui.button(t("Organigramm anordnen")).on_hover_text(t("Ordnet die verbundenen Karten als Baum an (nur die Auswahl, wenn etwas gewählt ist).")).clicked() {
                self.arrange();
            }
            if ui.button(t("Beispiel")).clicked() {
                self.doc = Doc::example();
                self.sel.clear();
                self.fitted = false;
            }
        });
        if !self.status.is_empty() {
            ui.label(RichText::new(&self.status).small());
        }
        ui.add_space(4.0);
    }

    fn swatches(&mut self, ui: &mut egui::Ui, ids: &[u64]) {
        ui.horizontal_wrapped(|ui| {
            for c in PALETTE {
                let (r, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
                ui.painter().rect(r, 5.0, col(c), Stroke::new(1.0, Color32::from_rgb(0x8E, 0x9A, 0xA3)));
                if resp.clicked() {
                    for id in ids {
                        if let Some(s) = self.doc.shape_mut(*id) {
                            s.fill = c;
                        }
                    }
                }
            }
        });
    }

    fn ui_props(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(6.0);
            let sel = self.sel.clone();
            if let Some(eid) = self.sel_edge {
                ui.label(RichText::new(t("Verbinder")).strong());
                if let Some(e) = self.doc.edge_mut(eid) {
                    ui.label(t("Beschriftung"));
                    let r = ui.add(egui::TextEdit::singleline(&mut e.label).desired_width(f32::INFINITY));
                    self.typing |= r.has_focus();
                    ui.checkbox(&mut e.arrow, t("Pfeilspitze"));
                    ui.radio_value(&mut e.elbow, true, t("Rechtwinklig"));
                    ui.radio_value(&mut e.elbow, false, t("Gerade"));
                    if ui.button(t("Richtung umkehren")).clicked() {
                        std::mem::swap(&mut e.a, &mut e.b);
                    }
                }
                if ui.button(t("Löschen")).clicked() {
                    self.delete_selection();
                }
            } else if sel.len() == 1 && self.doc.shape(sel[0]).is_some() {
                let id = sel[0];
                ui.label(RichText::new(t("Form")).strong());
                let want_focus = std::mem::take(&mut self.focus_text);
                if let Some(s) = self.doc.shape_mut(id) {
                    ui.label(if s.kind == Kind::Org { t("Name, Funktion (zweite Zeile)") } else { t("Text") });
                    let r = ui.add(egui::TextEdit::multiline(&mut s.text).desired_rows(3).desired_width(f32::INFINITY));
                    if want_focus {
                        r.request_focus();
                    }
                    self.typing |= r.has_focus();
                    ui.add_space(4.0);
                    egui::ComboBox::from_id_salt("kind").selected_text(t(s.kind.label())).show_ui(ui, |ui| {
                        for k in Kind::ALL {
                            ui.selectable_value(&mut s.kind, k, t(k.label()));
                        }
                    });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(t("Breite"));
                        ui.add(egui::DragValue::new(&mut s.w).range(24.0..=2000.0).speed(1.0));
                        ui.label(t("Höhe"));
                        ui.add(egui::DragValue::new(&mut s.h).range(20.0..=2000.0).speed(1.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label(t("Schrift"));
                        ui.add(egui::DragValue::new(&mut s.font).range(8.0..=60.0).speed(0.2));
                    });
                }
                ui.add_space(4.0);
                ui.label(t("Farbe"));
                self.swatches(ui, &[id]);
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    if ui.button(t("Untergeordnet hinzufügen")).on_hover_text(t("Strg+Enter")).clicked() {
                        self.add_child();
                    }
                    if ui.button(t("Gleiche Ebene hinzufügen")).on_hover_text(t("Strg+Umschalt+Enter")).clicked() {
                        self.add_sibling();
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button(t("Duplizieren")).clicked() {
                        self.duplicate();
                    }
                    if ui.button(t("Löschen")).clicked() {
                        self.delete_selection();
                    }
                });
            } else if sel.len() > 1 {
                ui.label(RichText::new(tf("{n} Formen gewählt", &[("n", &sel.len())])).strong());
                ui.add_space(4.0);
                ui.label(t("Ausrichten"));
                ui.horizontal_wrapped(|ui| {
                    for (label, how) in [("Links", Align::Left), ("Mitte", Align::HCenter), ("Rechts", Align::Right), ("Oben", Align::Top), ("Mitte ↕", Align::VCenter), ("Unten", Align::Bottom), ("Gleichmässig ↔", Align::DistributeH), ("Gleichmässig ↕", Align::DistributeV)] {
                        if ui.button(t(label)).clicked() {
                            self.doc.align(&sel, how);
                        }
                    }
                });
                ui.add_space(4.0);
                ui.label(t("Farbe"));
                self.swatches(ui, &sel);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(t("Duplizieren")).clicked() {
                        self.duplicate();
                    }
                    if ui.button(t("Löschen")).clicked() {
                        self.delete_selection();
                    }
                });
            } else {
                ui.label(RichText::new(t("Nichts gewählt")).strong());
                ui.add_space(4.0);
                ui.label(tf("{s} Formen, {e} Verbinder", &[("s", &self.doc.shapes.len()), ("e", &self.doc.edges.len())]));
                ui.add_space(8.0);
                ui.label(RichText::new(t("So geht's")).strong());
                for h in [
                    "Doppelklick auf die Fläche: neue Karte",
                    "Doppelklick auf eine Karte: Text ändern",
                    "Verbinden: Werkzeug «Verbinden» wählen und von Karte zu Karte ziehen (oder nacheinander antippen)",
                    "Karte wählen, dann «Untergeordnet hinzufügen»: Karte samt Verbinder",
                    "«Organigramm anordnen» richtet alles als Baum aus",
                    "Rechteck aufziehen: mehrere wählen; Umschalt: dazuwählen",
                    "Fläche verschieben: Mausrad, Leertaste+Ziehen oder rechte Maustaste. Zoom: Strg+Mausrad",
                    "Strg+Z / Strg+Y: Rückgängig / Wiederholen, Strg+D: Duplizieren, Entf: Löschen",
                ] {
                    ui.label(RichText::new(format!("• {}", t(h))).small());
                }
            }
        });
    }
}

// ================================================================ eframe

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(STORAGE_KEY, self.doc.to_json_string());
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Geöffnete Dateien einlesen
        while let Ok((name, data)) = self.open_rx.try_recv() {
            match String::from_utf8(data).map_err(|_| t("Die Datei ist kein Text.").to_string()).and_then(|t| Doc::from_json_str(&t)) {
                Ok(d) => {
                    self.doc = d;
                    self.sel.clear();
                    self.sel_edge = None;
                    self.fitted = false;
                    self.status = tf("«{name}» geöffnet.", &[("name", &name)]);
                }
                Err(e) => {
                    // Bekannte Meldung aus model.rs übersetzen (model.rs bleibt ohne sprache.rs testbar).
                    let e = if e == "Das ist keine Organiwerk-Datei." { t("Das ist keine Organiwerk-Datei.").to_string() } else { e };
                    self.status = tf("«{name}» konnte nicht geöffnet werden: {e}", &[("name", &name), ("e", &e)]);
                }
            }
        }

        self.shortcuts(ctx);
        self.typing = false;

        let narrow = ctx.screen_rect().width() < 760.0;
        egui::TopBottomPanel::top("leiste").show(ctx, |ui| self.ui_toolbar(ui));
        if narrow {
            egui::TopBottomPanel::bottom("eigenschaften").resizable(true).default_height(180.0).show(ctx, |ui| self.ui_props(ui));
        } else {
            egui::SidePanel::right("eigenschaften").resizable(true).default_width(260.0).width_range(200.0..=420.0).show(ctx, |ui| self.ui_props(ui));
        }
        egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| self.canvas(ui));

        // Rückgängig-Schritte: eine zusammenhängende Handlung (Ziehen, Tippen) ist ein Schritt.
        let busy = ctx.input(|i| i.pointer.any_down()) || self.typing;
        if self.doc != self.base {
            if !(busy && self.gesture_open) {
                self.undo.push(self.base.clone());
                if self.undo.len() > 80 {
                    self.undo.remove(0);
                }
                self.redo.clear();
            }
            self.gesture_open = busy;
            self.base = self.doc.clone();
        } else if !busy {
            self.gesture_open = false;
        }
    }
}
