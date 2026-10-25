//! Editor für Bildschirmfotos (eigenes Fenster): Pfeil, Rechteck, Stift, Text,
//! Leuchtstift, Schwärzen, Verpixeln, Zuschneiden, Rückgängig/Wiederholen,
//! Farbe und Strichstärke; Speichern als PNG/JPG, Kopieren, Ordner öffnen.
//!
//! Das Originalbild bleibt im Speicher; jede Bearbeitung ist ein Schritt in
//! einer Liste. Das angezeigte und gespeicherte Bild wird aus Original +
//! Schritten neu berechnet (schnapp.rs). Schwärzen/Verpixeln verändern dabei die
//! Pixel selbst – in der gespeicherten Datei ist der Inhalt darunter weg.

use std::path::PathBuf;

use crate::schnapp::{self, Blend, History, Img, Rechteck};
use crate::shot;
use crate::sprache::{t, tf};
use crate::stil::{self, Sym};
use crate::thema;

/// Werkzeuge im Editor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Arrow,
    Rect,
    Pen,
    Text,
    Marker,
    Blacken,
    Pixelate,
    Crop,
}

impl Tool {
    const ALL: [Tool; 8] = [
        Tool::Arrow,
        Tool::Rect,
        Tool::Pen,
        Tool::Text,
        Tool::Marker,
        Tool::Blacken,
        Tool::Pixelate,
        Tool::Crop,
    ];
    fn label(self) -> &'static str {
        match self {
            Tool::Arrow => t("Pfeil"),
            Tool::Rect => t("Rechteck"),
            Tool::Pen => t("Stift"),
            Tool::Text => t("Text"),
            Tool::Marker => t("Markieren"),
            Tool::Blacken => t("Schwärzen"),
            Tool::Pixelate => t("Verpixeln"),
            Tool::Crop => t("Zuschneiden"),
        }
    }
    fn tip(self) -> &'static str {
        match self {
            Tool::Arrow => t("Pfeil ziehen – Spitze am Ende"),
            Tool::Rect => t("Rahmen aufziehen"),
            Tool::Pen => t("Frei zeichnen"),
            Tool::Text => t("Ins Bild klicken, Text tippen, Enter"),
            Tool::Marker => t("Halbtransparenter Leuchtstift"),
            Tool::Blacken => t("Bereich schwarz übermalen"),
            Tool::Pixelate => t("Bereich unkenntlich machen – die Pixel werden ersetzt"),
            Tool::Crop => t("Ausschnitt aufziehen, der bleiben soll"),
        }
    }
    /// Werkzeuge, die mit einem aufgezogenen Rechteck arbeiten.
    fn is_area(self) -> bool {
        matches!(self, Tool::Blacken | Tool::Pixelate | Tool::Crop)
    }
}

/// Ein Bearbeitungsschritt (Koordinaten in Pixeln des Bildes, wie es zu diesem
/// Zeitpunkt war – also nach früheren Zuschnitten).
#[derive(Clone, Debug)]
enum Op {
    Arrow { a: (f32, f32), b: (f32, f32), w: f32, c: [u8; 4] },
    Rect { a: (f32, f32), b: (f32, f32), w: f32, c: [u8; 4] },
    Pen { pts: Vec<(f32, f32)>, w: f32, c: [u8; 4] },
    Marker { pts: Vec<(f32, f32)>, w: f32, c: [u8; 4] },
    Text { at: (f32, f32), s: String, size: f32, c: [u8; 4] },
    Blacken(Rechteck),
    Pixelate(Rechteck),
    Crop(Rechteck),
}

/// Text, der gerade eingegeben wird.
struct PendingText {
    at: (f32, f32),
    s: String,
    focus: bool,
}

/// Farbfelder zum schnellen Wählen (Zeichenfarben, keine Oberflächenfarben).
const SWATCHES: [[u8; 3]; 8] = [
    [0xe5, 0x34, 0x2e], // Rot
    [0xf9, 0x73, 0x16], // Orange
    [0xfa, 0xcc, 0x15], // Gelb
    [0x16, 0xa3, 0x4a], // Grün
    [0x25, 0x63, 0xeb], // Blau
    [0x93, 0x33, 0xea], // Violett
    [0x11, 0x11, 0x11], // Schwarz
    [0xff, 0xff, 0xff], // Weiss
];

pub struct Editor {
    pub id: u64,
    base: Img,
    hist: History<Op>,
    shown: Img,
    tex: Option<egui::TextureHandle>,
    dirty: bool,
    tool: Tool,
    color: egui::Color32,
    marker_color: egui::Color32,
    width: f32,
    text_size: f32,
    drag: Option<Vec<(f32, f32)>>,
    text: Option<PendingText>,
    font: Option<ab_glyph::FontVec>,
    out_dir: String,
    jpg: bool,
    /// Seit der letzten Änderung gespeichert oder kopiert?
    kept: bool,
    msg: Option<(String, bool)>,
    ask_close: bool,
    focus_once: bool,
    /// Gespeicherte Dateien (Name, Pfad, Info) – holt die Hauptansicht ab.
    pub saved: Vec<(String, String, String)>,
    pub closed: bool,
}

impl Editor {
    pub fn new(id: u64, img: Img, out_dir: String, jpg: bool) -> Self {
        Editor {
            id,
            shown: img.clone(),
            base: img,
            hist: History::default(),
            tex: None,
            dirty: true,
            tool: Tool::Arrow,
            color: egui::Color32::from_rgb(0xe5, 0x34, 0x2e),
            marker_color: egui::Color32::from_rgb(0xfa, 0xe6, 0x3c),
            width: 4.0,
            text_size: 28.0,
            drag: None,
            text: None,
            font: load_font(),
            out_dir,
            jpg,
            kept: false,
            msg: None,
            ask_close: false,
            focus_once: true,
            saved: Vec::new(),
            closed: false,
        }
    }

    pub fn title(&self) -> String {
        tf("Bildschirmfoto bearbeiten · {w} × {h}", &[("w", &self.shown.w), ("h", &self.shown.h)])
    }

    fn render(&mut self) {
        let mut img = self.base.clone();
        for op in self.hist.items() {
            apply(&mut img, op, self.font.as_ref());
        }
        self.shown = img;
        self.dirty = true;
    }

    fn commit(&mut self, op: Op) {
        self.hist.push(op);
        self.kept = false;
        self.render();
    }

    fn undo(&mut self) {
        if self.hist.undo() {
            self.kept = false;
            self.render();
        }
    }

    fn redo(&mut self) {
        if self.hist.redo() {
            self.kept = false;
            self.render();
        }
    }

    fn draw_color(&self) -> [u8; 4] {
        let c = if self.tool == Tool::Marker { self.marker_color } else { self.color };
        [c.r(), c.g(), c.b(), 255]
    }

    fn save(&mut self, jpg: bool) -> bool {
        let ext = if jpg { "jpg" } else { "png" };
        let (y, mo, d, h, mi, s) = crate::app::local_now();
        let stem = schnapp::shot_stem(y, mo, d, h, mi, s);
        let dir = if self.out_dir.trim().is_empty() {
            PathBuf::from(crate::app::default_shot_dir())
        } else {
            PathBuf::from(&self.out_dir)
        };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.msg = Some((tf("Speichern fehlgeschlagen: {e}", &[("e", &e)]), true));
            return false;
        }
        let name = schnapp::unique_name(&stem, ext, |n| dir.join(n).exists());
        let path = dir.join(&name);
        match shot::save(&self.shown, &path, jpg) {
            Ok(()) => {
                self.kept = true;
                let info = format!("{} × {} · {}", self.shown.w, self.shown.h, ext.to_uppercase());
                self.saved.push((name.clone(), path.display().to_string(), info));
                self.msg = Some((tf("Gespeichert: {name}", &[("name", &name)]), false));
                true
            }
            Err(e) => {
                self.msg = Some((tf("Speichern fehlgeschlagen: {e}", &[("e", &e)]), true));
                false
            }
        }
    }

    /// In die Zwischenablage (auch gleich nach dem Foto, falls eingestellt).
    pub fn copy(&mut self) {
        match shot::copy_to_clipboard(&self.shown) {
            Ok(()) => {
                self.kept = true;
                self.msg = Some((t("In die Zwischenablage kopiert.").to_string(), false));
            }
            Err(e) => {
                self.msg = Some((tf("Kopieren fehlgeschlagen: {e}", &[("e", &e)]), true));
            }
        }
    }

    fn open_dir(&self) {
        let dir = if self.out_dir.trim().is_empty() { crate::app::default_shot_dir() } else { self.out_dir.clone() };
        let _ = std::fs::create_dir_all(&dir);
        shot::open_folder(&dir);
    }

    /// Text übernehmen (falls nicht leer).
    fn commit_text(&mut self) {
        if let Some(p) = self.text.take() {
            if !p.s.trim().is_empty() {
                let c = [self.color.r(), self.color.g(), self.color.b(), 255];
                self.commit(Op::Text { at: p.at, s: p.s, size: self.text_size, c });
            }
        }
    }

    /// Inhalt des Editor-Fensters; wird im Fenster-Callback aufgerufen.
    pub fn show(&mut self, ctx: &egui::Context) {
        if self.focus_once {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.focus_once = false;
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.kept {
                self.closed = true;
            } else {
                self.ask_close = true;
            }
        }

        // Tastenkürzel (nicht während der Texteingabe).
        if !ctx.wants_keyboard_input() {
            let cmd_shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
            if ctx.input_mut(|i| i.consume_key(cmd_shift, egui::Key::Z) || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Y)) {
                self.redo();
            } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
                self.undo();
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
                let jpg = self.jpg;
                self.save(jpg);
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::C)) {
                self.copy();
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.drag = None;
            }
        }

        if self.ask_close {
            self.close_bar(ctx);
        }
        self.toolbar(ctx);
        self.status_bar(ctx);
        self.canvas(ctx);
        self.text_box(ctx);
    }

    fn close_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top(egui::Id::new(("schliessen", self.id)))
            .frame(egui::Frame::none().fill(warn_soft()).inner_margin(egui::Margin::symmetric(12.0, 6.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    stil::symbol(ui, Sym::Warnung, 18.0, warn_text());
                    ui.label(egui::RichText::new(t("Das Bild ist noch nicht gespeichert.")).strong().color(warn_text()));
                    ui.add_space(8.0);
                    if stil::knopf(ui, Sym::Speichern, t("Speichern und schliessen")).clicked() {
                        let jpg = self.jpg;
                        if self.save(jpg) {
                            self.closed = true;
                        }
                    }
                    if stil::gefahr(ui, true, Sym::Papierkorb, t("Verwerfen")).clicked() {
                        self.closed = true;
                    }
                    if stil::leise(ui, true, Sym::Stift, t("Weiter bearbeiten"), "").clicked() {
                        self.ask_close = false;
                    }
                });
            });
    }

    fn toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top(egui::Id::new(("werkzeuge", self.id)))
            .frame(egui::Frame::none().fill(thema::panel()).inner_margin(egui::Margin::symmetric(12.0, 8.0)).stroke(egui::Stroke::new(1.0, thema::border())))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                // Werkzeuge (Hinweis zum gewählten Werkzeug steht unten in der Fussleiste)
                let before = self.tool;
                let mut tool = self.tool;
                let opts: Vec<(Tool, &str)> = Tool::ALL.iter().map(|w| (*w, w.label())).collect();
                if stil::wahl(ui, &mut tool, &opts) {
                    if before == Tool::Text && tool != Tool::Text {
                        self.commit_text();
                    }
                    self.tool = tool;
                    self.drag = None;
                }
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    // Farbe
                    ui.label(egui::RichText::new(t("Farbe")).size(12.0).color(thema::muted()));
                    let target = if self.tool == Tool::Marker { &mut self.marker_color } else { &mut self.color };
                    for s in SWATCHES {
                        let c = egui::Color32::from_rgb(s[0], s[1], s[2]);
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
                        let sel = *target == c;
                        ui.painter().rect_filled(r, 4.0, c);
                        ui.painter().rect_stroke(
                            r,
                            4.0,
                            egui::Stroke::new(if sel { 2.0 } else { 1.0 }, if sel { thema::text() } else { thema::border() }),
                        );
                        if resp.clicked() {
                            *target = c;
                        }
                    }
                    egui::color_picker::color_edit_button_srgba(ui, target, egui::color_picker::Alpha::Opaque)
                        .on_hover_text(t("Eigene Farbe"));
                    stil::trenner(ui);
                    if self.tool == Tool::Text {
                        ui.add(egui::Slider::new(&mut self.text_size, 10.0..=120.0).text(t("Schriftgrösse")));
                    } else {
                        ui.add(egui::Slider::new(&mut self.width, 1.0..=24.0).text(t("Strichstärke")));
                    }
                    stil::trenner(ui);
                    if stil::leise(ui, self.hist.can_undo(), Sym::Rueckgaengig, t("Rückgängig"), t("Strg + Z")).clicked() {
                        self.undo();
                    }
                    if stil::leise(ui, self.hist.can_redo(), Sym::Aktualisieren, t("Wiederholen"), t("Strg + Y")).clicked() {
                        self.redo();
                    }
                    stil::rechts(ui, |ui| {
                        if stil::leise(ui, true, Sym::OrdnerAuf, t("Ordner öffnen"), "").clicked() {
                            self.open_dir();
                        }
                        if stil::leise(ui, true, Sym::Kopieren, t("Kopieren"), t("In die Zwischenablage (Strg + C)")).clicked() {
                            self.copy();
                        }
                    });
                });
            });
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom(egui::Id::new(("fuss", self.id)))
            .frame(egui::Frame::none().fill(thema::panel()).inner_margin(egui::Margin::symmetric(14.0, 9.0)).stroke(egui::Stroke::new(1.0, thema::border())))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.horizontal(|ui| {
                    match &self.msg {
                        Some((m, true)) => {
                            ui.colored_label(warn_text(), m.as_str());
                        }
                        Some((m, false)) => {
                            ui.colored_label(stil::akzent(), m.as_str());
                        }
                        None => {
                            stil::grau(ui, self.tool.tip());
                        }
                    }
                    stil::trenner(ui);
                    let n = self.hist.items().len();
                    stil::grau(ui, &tf("{w} × {h} Pixel · {n} Schritte",
                        &[("w", &self.shown.w), ("h", &self.shown.h), ("n", &n)],
                    ));
                    stil::rechts(ui, |ui| {
                        // Hauptknopf im voreingestellten Format, das andere daneben.
                        let jpg = self.jpg;
                        let (main, other) = if jpg {
                            (t("Als JPG speichern"), t("Als PNG speichern"))
                        } else {
                            (t("Als PNG speichern"), t("Als JPG speichern"))
                        };
                        if stil::haupt(ui, true, Sym::Speichern, main).on_hover_text(t("Strg + S")).clicked() {
                            self.save(jpg);
                        }
                        if stil::knopf_text(ui, true, other).clicked() {
                            self.save(!jpg);
                        }
                    });
                });
            });
    }

    fn canvas(&mut self, ctx: &egui::Context) {
        if self.dirty {
            let ci = color_image(&self.shown);
            if let Some(tex) = self.tex.as_mut() {
                tex.set(ci, egui::TextureOptions::LINEAR);
            } else {
                self.tex = Some(ctx.load_texture(format!("foto-{}", self.id), ci, egui::TextureOptions::LINEAR));
            }
            self.dirty = false;
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(thema::bg()).inner_margin(egui::Margin::same(14.0)))
            .show(ctx, |ui| {
                let avail = ui.available_rect_before_wrap();
                let s = schnapp::fit_scale(self.shown.w, self.shown.h, avail.width(), avail.height(), 2.0);
                let size = egui::vec2(self.shown.w as f32 * s, self.shown.h as f32 * s);
                let img_rect = egui::Rect::from_center_size(avail.center(), size);
                let resp = ui.allocate_rect(avail, egui::Sense::click_and_drag());
                let painter = ui.painter_at(avail);
                // dezenter Schatten/Rand ums Bild
                painter.rect_stroke(img_rect.expand(1.0), 0.0, egui::Stroke::new(1.0, thema::border()));
                if let Some(tex) = &self.tex {
                    painter.image(
                        tex.id(),
                        img_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
                if resp.hovered() {
                    let icon = if self.tool == Tool::Text { egui::CursorIcon::Text } else { egui::CursorIcon::Crosshair };
                    ui.ctx().set_cursor_icon(icon);
                }

                let to_img = |p: egui::Pos2| ((p.x - img_rect.min.x) / s, (p.y - img_rect.min.y) / s);
                let to_scr = |q: (f32, f32)| egui::pos2(img_rect.min.x + q.0 * s, img_rect.min.y + q.1 * s);

                // Klicks
                if resp.clicked() {
                    if let Some(p) = resp.interact_pointer_pos() {
                        let q = to_img(p);
                        match self.tool {
                            Tool::Text => {
                                self.commit_text();
                                self.text = Some(PendingText { at: q, s: String::new(), focus: true });
                            }
                            Tool::Pen | Tool::Marker => {
                                let (w, c) = (self.stroke_px(), self.draw_color());
                                let op = if self.tool == Tool::Pen {
                                    Op::Pen { pts: vec![q], w, c }
                                } else {
                                    Op::Marker { pts: vec![q], w, c }
                                };
                                self.commit(op);
                            }
                            _ => {}
                        }
                    }
                }

                // Ziehen
                if resp.drag_started() && self.tool != Tool::Text {
                    let start = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos());
                    if let Some(p) = start {
                        self.drag = Some(vec![to_img(p)]);
                    }
                }
                if resp.dragged() {
                    if let (Some(pts), Some(p)) = (self.drag.as_mut(), resp.interact_pointer_pos()) {
                        let q = to_img(p);
                        match self.tool {
                            Tool::Pen | Tool::Marker => {
                                let far = pts.last().map(|l| (l.0 - q.0).abs() + (l.1 - q.1).abs() > 0.75 / s).unwrap_or(true);
                                if far {
                                    pts.push(q);
                                }
                            }
                            _ => {
                                if pts.len() < 2 {
                                    pts.push(q);
                                } else if let Some(last) = pts.last_mut() {
                                    *last = q;
                                }
                            }
                        }
                    }
                }
                if resp.drag_stopped() {
                    if let Some(pts) = self.drag.take() {
                        self.finish_drag(pts);
                    }
                }

                // Vorschau des laufenden Zugs
                if let Some(pts) = &self.drag {
                    let c = self.draw_color();
                    let col = egui::Color32::from_rgb(c[0], c[1], c[2]);
                    let w = self.stroke_px() * s;
                    let scr: Vec<egui::Pos2> = pts.iter().map(|q| to_scr(*q)).collect();
                    match self.tool {
                        Tool::Pen => {
                            painter.add(egui::Shape::line(scr, egui::Stroke::new(w, col)));
                        }
                        Tool::Marker => {
                            painter.add(egui::Shape::line(scr, egui::Stroke::new(w, col.gamma_multiply(0.45))));
                        }
                        Tool::Arrow if scr.len() >= 2 => {
                            painter.arrow(scr[0], scr[1] - scr[0], egui::Stroke::new(w, col));
                        }
                        Tool::Rect if scr.len() >= 2 => {
                            painter.rect_stroke(egui::Rect::from_two_pos(scr[0], scr[1]), 0.0, egui::Stroke::new(w, col));
                        }
                        tool if tool.is_area() && scr.len() >= 2 => {
                            let r = egui::Rect::from_two_pos(scr[0], scr[1]).intersect(img_rect);
                            match tool {
                                Tool::Blacken => {
                                    painter.rect_filled(r, 0.0, egui::Color32::BLACK);
                                }
                                Tool::Pixelate => {
                                    painter.rect_filled(r, 0.0, egui::Color32::from_black_alpha(110));
                                    painter.rect_stroke(r, 0.0, egui::Stroke::new(1.5, egui::Color32::WHITE));
                                }
                                _ => {
                                    // Zuschneiden: aussen abdunkeln
                                    let veil = egui::Color32::from_black_alpha(140);
                                    let o = img_rect;
                                    painter.rect_filled(egui::Rect::from_min_max(o.min, egui::pos2(o.max.x, r.min.y)), 0.0, veil);
                                    painter.rect_filled(egui::Rect::from_min_max(egui::pos2(o.min.x, r.max.y), o.max), 0.0, veil);
                                    painter.rect_filled(egui::Rect::from_min_max(egui::pos2(o.min.x, r.min.y), egui::pos2(r.min.x, r.max.y)), 0.0, veil);
                                    painter.rect_filled(egui::Rect::from_min_max(egui::pos2(r.max.x, r.min.y), egui::pos2(o.max.x, r.max.y)), 0.0, veil);
                                    painter.rect_stroke(r, 0.0, egui::Stroke::new(1.5, egui::Color32::WHITE));
                                }
                            }
                        }
                        _ => {}
                    }
                }

                // Vorschau des Texts, der gerade getippt wird
                if let Some(p) = &self.text {
                    let pos = to_scr(p.at);
                    let shown = if p.s.is_empty() { "|" } else { p.s.as_str() };
                    painter.text(
                        pos,
                        egui::Align2::LEFT_TOP,
                        shown,
                        egui::FontId::proportional((self.text_size * s).max(4.0)),
                        self.color,
                    );
                }
            });
    }

    /// Eingabefeld für den Text (schwebend unter der Klickstelle).
    fn text_box(&mut self, ctx: &egui::Context) {
        let Some(p) = self.text.as_mut() else { return };
        let mut commit = false;
        let mut cancel = false;
        egui::Window::new(t("Text"))
            .id(egui::Id::new(("textfeld", self.id)))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -48.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut p.s)
                            .hint_text(t("Text eingeben"))
                            .desired_width(280.0),
                    );
                    if p.focus {
                        r.request_focus();
                        p.focus = false;
                    }
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        commit = true;
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        cancel = true;
                    }
                    if stil::knopf(ui, Sym::Haken, t("Einfügen")).clicked() {
                        commit = true;
                    }
                    if stil::leise(ui, true, Sym::Kreuz, t("Abbrechen"), "").clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel {
            self.text = None;
        } else if commit {
            self.commit_text();
        }
    }

    /// Strichstärke in Bildpixeln (der Leuchtstift ist breiter).
    fn stroke_px(&self) -> f32 {
        if self.tool == Tool::Marker {
            self.width * 4.0
        } else {
            self.width
        }
    }

    fn finish_drag(&mut self, pts: Vec<(f32, f32)>) {
        let (w, c) = (self.stroke_px(), self.draw_color());
        let (iw, ih) = (self.shown.w, self.shown.h);
        let two = |pts: &[(f32, f32)]| -> Option<((f32, f32), (f32, f32))> {
            match (pts.first(), pts.last()) {
                (Some(a), Some(b)) if pts.len() >= 2 => Some((*a, *b)),
                _ => None,
            }
        };
        let area = |pts: &[(f32, f32)]| -> Option<Rechteck> {
            let (a, b) = two(pts)?;
            let r = schnapp::norm_rect(a.0, a.1, b.0, b.1, iw, ih)?;
            if r.2 >= 2 && r.3 >= 2 {
                Some(r)
            } else {
                None
            }
        };
        let op = match self.tool {
            Tool::Pen => Some(Op::Pen { pts, w, c }),
            Tool::Marker => Some(Op::Marker { pts, w, c }),
            Tool::Arrow => two(&pts).map(|(a, b)| Op::Arrow { a, b, w, c }),
            Tool::Rect => two(&pts).map(|(a, b)| Op::Rect { a, b, w, c }),
            Tool::Blacken => area(&pts).map(Op::Blacken),
            Tool::Pixelate => area(&pts).map(Op::Pixelate),
            Tool::Crop => area(&pts).map(Op::Crop),
            Tool::Text => None,
        };
        if let Some(op) = op {
            self.commit(op);
        }
    }
}

/// Wendet einen Schritt auf das Bild an.
fn apply(img: &mut Img, op: &Op, font: Option<&ab_glyph::FontVec>) {
    match op {
        Op::Arrow { a, b, w, c } => schnapp::draw_arrow(img, *a, *b, *w, *c),
        Op::Rect { a, b, w, c } => schnapp::draw_rect_outline(img, *a, *b, *w, *c),
        Op::Pen { pts, w, c } => schnapp::draw_polyline(img, pts, *w, *c, Blend::Normal),
        Op::Marker { pts, w, c } => schnapp::draw_polyline(img, pts, *w, *c, Blend::Multiply),
        Op::Text { at, s, size, c } => {
            if let Some(f) = font {
                draw_text(img, f, *at, *size, s, *c);
            }
        }
        Op::Blacken(r) => schnapp::fill_rect(img, *r, [0, 0, 0, 255]),
        Op::Pixelate(r) => schnapp::pixelate(img, *r, schnapp::pixel_block(r.2, r.3)),
        Op::Crop(r) => {
            if let Some(c) = schnapp::crop(img, *r) {
                *img = c;
            }
        }
    }
}

/// Schrift aus den eingebauten egui-Schriften (dieselbe wie in der Oberfläche).
fn load_font() -> Option<ab_glyph::FontVec> {
    let defs = egui::FontDefinitions::default();
    let data = defs
        .font_data
        .get("Ubuntu-Light")
        .or_else(|| defs.font_data.values().next())?;
    ab_glyph::FontVec::try_from_vec(data.font.to_vec()).ok()
}

/// Rastert Text mit ab_glyph und mischt ihn ins Bild (obere linke Ecke `at`).
fn draw_text(img: &mut Img, font: &ab_glyph::FontVec, at: (f32, f32), size: f32, text: &str, c: [u8; 4]) {
    use ab_glyph::{point, Font, GlyphId, PxScale, ScaleFont};
    let scale = PxScale::from(size.max(1.0));
    let sf = font.as_scaled(scale);
    let mut x = at.0;
    let mut baseline = at.1 + sf.ascent();
    let line_h = sf.height() + sf.line_gap();
    let mut prev: Option<GlyphId> = None;
    for ch in text.chars() {
        if ch == '\n' {
            x = at.0;
            baseline += line_h;
            prev = None;
            continue;
        }
        let id = font.glyph_id(ch);
        if let Some(p) = prev {
            x += sf.kern(p, id);
        }
        let glyph = id.with_scale_and_position(scale, point(x, baseline));
        x += sf.h_advance(id);
        prev = Some(id);
        if let Some(og) = font.outline_glyph(glyph) {
            let b = og.px_bounds();
            let bw = (b.max.x - b.min.x).max(0.0) as u32;
            let bh = (b.max.y - b.min.y).max(0.0) as u32;
            if bw == 0 || bh == 0 || bw > 4096 || bh > 4096 {
                continue;
            }
            let mut cov = vec![0u8; bw as usize * bh as usize];
            og.draw(|gx, gy, v| {
                let i = gy as usize * bw as usize + gx as usize;
                if let Some(slot) = cov.get_mut(i) {
                    *slot = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                }
            });
            schnapp::blend_coverage(img, b.min.x as i32, b.min.y as i32, bw, bh, &cov, c);
        }
    }
}

/// RGBA-Bild → egui-Bild.
pub fn color_image(img: &Img) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied([img.w as usize, img.h as usize], &img.px)
}

// Farben im Editor je Design (hell/dunkel).
fn warn_text() -> egui::Color32 {
    thema::pick(egui::Color32::from_rgb(0xc6, 0x28, 0x28), egui::Color32::from_rgb(0xf8, 0x71, 0x71))
}
fn warn_soft() -> egui::Color32 {
    thema::pick(egui::Color32::from_rgb(0xfd, 0xec, 0xec), egui::Color32::from_rgb(0x3a, 0x22, 0x26))
}
