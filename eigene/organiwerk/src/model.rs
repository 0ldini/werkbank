//! Dokument von Organiwerk: Formen, Verbinder, Anordnen, Speichern (JSON) und SVG-Export.
//! Nur Standardbibliothek (und das eigene `json`), damit alles ohne Oberfläche getestet werden kann.

use std::collections::{HashMap, HashSet};

use crate::json::{self, Json};

pub type P = (f32, f32);

// ================================================================ Formen

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Rect,
    Rounded,
    Ellipse,
    Diamond,
    /// Karte für Personen/Stellen: erste Zeile = Name, weitere Zeilen = Funktion.
    Org,
    Text,
    /// Rahmen um mehrere Formen (Abteilung, Team); liegt hinten.
    Group,
}

impl Kind {
    pub const ALL: [Kind; 7] = [Kind::Org, Kind::Rounded, Kind::Rect, Kind::Ellipse, Kind::Diamond, Kind::Text, Kind::Group];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Rect => "rect",
            Kind::Rounded => "rounded",
            Kind::Ellipse => "ellipse",
            Kind::Diamond => "diamond",
            Kind::Org => "org",
            Kind::Text => "text",
            Kind::Group => "group",
        }
    }
    pub fn parse(s: &str) -> Kind {
        match s {
            "rounded" => Kind::Rounded,
            "ellipse" => Kind::Ellipse,
            "diamond" => Kind::Diamond,
            "org" => Kind::Org,
            "text" => Kind::Text,
            "group" => Kind::Group,
            _ => Kind::Rect,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Kind::Rect => "Rechteck",
            Kind::Rounded => "Abgerundet",
            Kind::Ellipse => "Ellipse",
            Kind::Diamond => "Raute",
            Kind::Org => "Person / Stelle",
            Kind::Text => "Text",
            Kind::Group => "Gruppe",
        }
    }
}

pub const PALETTE: [u32; 9] = [0xFFFFFF, 0xDCE8FA, 0xD5EFE3, 0xFBEFC9, 0xFAD9C0, 0xF6D0CF, 0xE4DAF5, 0xE3E8EB, 0x2B3A47];

#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub id: u64,
    pub kind: Kind,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub text: String,
    /// Füllfarbe 0xRRGGBB.
    pub fill: u32,
    /// Schriftgrösse in Dokumenteinheiten.
    pub font: f32,
}

impl Shape {
    pub fn center(&self) -> P {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    /// Liegt der Punkt in der Form? Bei Gruppen nur auf Titelleiste und Rand.
    pub fn contains(&self, p: P) -> bool {
        let (px, py) = p;
        let inside = px >= self.x && px <= self.right() && py >= self.y && py <= self.bottom();
        if !inside {
            return false;
        }
        match self.kind {
            Kind::Ellipse => {
                let (cx, cy) = self.center();
                let (rx, ry) = (self.w / 2.0, self.h / 2.0);
                let (dx, dy) = ((px - cx) / rx.max(0.001), (py - cy) / ry.max(0.001));
                dx * dx + dy * dy <= 1.0
            }
            Kind::Diamond => {
                let (cx, cy) = self.center();
                (px - cx).abs() / (self.w / 2.0).max(0.001) + (py - cy).abs() / (self.h / 2.0).max(0.001) <= 1.0
            }
            Kind::Group => py <= self.y + 28.0 || px <= self.x + 6.0 || px >= self.right() - 6.0 || py >= self.bottom() - 6.0,
            _ => true,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub id: u64,
    pub a: u64,
    pub b: u64,
    /// Rechtwinklig statt gerade.
    pub elbow: bool,
    pub arrow: bool,
    pub label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    HCenter,
    Right,
    Top,
    VCenter,
    Bottom,
    DistributeH,
    DistributeV,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Doc {
    pub shapes: Vec<Shape>,
    pub edges: Vec<Edge>,
    pub next_id: u64,
}

// ================================================================ Bearbeiten

impl Doc {
    pub fn new_id(&mut self) -> u64 {
        let used = self.shapes.iter().map(|s| s.id).chain(self.edges.iter().map(|e| e.id)).max().unwrap_or(0);
        self.next_id = self.next_id.max(used) + 1;
        self.next_id
    }

    pub fn shape(&self, id: u64) -> Option<&Shape> {
        self.shapes.iter().find(|s| s.id == id)
    }
    pub fn shape_mut(&mut self, id: u64) -> Option<&mut Shape> {
        self.shapes.iter_mut().find(|s| s.id == id)
    }
    pub fn edge(&self, id: u64) -> Option<&Edge> {
        self.edges.iter().find(|e| e.id == id)
    }
    pub fn edge_mut(&mut self, id: u64) -> Option<&mut Edge> {
        self.edges.iter_mut().find(|e| e.id == id)
    }

    pub fn add_shape(&mut self, kind: Kind, x: f32, y: f32) -> u64 {
        let id = self.new_id();
        let (w, h, fill, text) = match kind {
            Kind::Rect => (140.0, 70.0, 0xDCE8FA, "Text"),
            Kind::Rounded => (140.0, 70.0, 0xD5EFE3, "Text"),
            Kind::Ellipse => (140.0, 80.0, 0xFBEFC9, "Text"),
            Kind::Diamond => (150.0, 100.0, 0xFAD9C0, "Entscheid?"),
            Kind::Org => (170.0, 64.0, 0xDCE8FA, "Name\nFunktion"),
            Kind::Text => (120.0, 30.0, 0xFFFFFF, "Text"),
            Kind::Group => (320.0, 220.0, 0xF4F6F8, "Abteilung"),
        };
        self.shapes.push(Shape { id, kind, x: x - w / 2.0, y: y - h / 2.0, w, h, text: text.to_string(), fill, font: 14.0 });
        id
    }

    pub fn remove_shapes(&mut self, ids: &[u64]) {
        self.shapes.retain(|s| !ids.contains(&s.id));
        self.edges.retain(|e| !ids.contains(&e.a) && !ids.contains(&e.b));
    }

    pub fn remove_edge(&mut self, id: u64) {
        self.edges.retain(|e| e.id != id);
    }

    pub fn connect(&mut self, a: u64, b: u64, elbow: bool, arrow: bool) -> Option<u64> {
        if a == b || self.shape(a).is_none() || self.shape(b).is_none() || self.edges.iter().any(|e| e.a == a && e.b == b) {
            return None;
        }
        let id = self.new_id();
        self.edges.push(Edge { id, a, b, elbow, arrow, label: String::new() });
        Some(id)
    }

    pub fn children(&self, id: u64) -> Vec<u64> {
        self.edges.iter().filter(|e| e.a == id).map(|e| e.b).collect()
    }
    pub fn parent(&self, id: u64) -> Option<u64> {
        self.edges.iter().find(|e| e.b == id).map(|e| e.a)
    }

    /// Neue Karte unter `parent`, verbunden (ohne Pfeil, wie im Organigramm).
    pub fn add_child(&mut self, parent: u64) -> Option<u64> {
        let p = self.shape(parent)?.clone();
        let kind = if p.kind == Kind::Group || p.kind == Kind::Text { Kind::Org } else { p.kind };
        let kids = self.children(parent);
        let x = kids.iter().filter_map(|k| self.shape(*k)).map(|k| k.right() + 30.0).fold(p.x, f32::max);
        let id = self.add_shape(kind, 0.0, 0.0);
        let (fill, w, h, font) = (p.fill, p.w, p.h, p.font);
        let s = self.shape_mut(id)?;
        s.x = x;
        s.y = p.bottom() + 60.0;
        s.w = w;
        s.h = h;
        s.fill = fill;
        s.font = font;
        s.text = if kind == Kind::Org { "Name\nFunktion".into() } else { "Text".into() };
        self.connect(parent, id, true, false);
        Some(id)
    }

    /// Neue Karte neben `id` (gleiche Ebene). Ohne Vorgesetzte(n) einfach rechts daneben.
    pub fn add_sibling(&mut self, id: u64) -> Option<u64> {
        match self.parent(id) {
            Some(p) => self.add_child(p),
            None => {
                let s = self.shape(id)?.clone();
                let n = self.add_shape(s.kind, 0.0, 0.0);
                let t = self.shape_mut(n)?;
                t.x = s.right() + 30.0;
                t.y = s.y;
                t.w = s.w;
                t.h = s.h;
                t.fill = s.fill;
                t.font = s.font;
                Some(n)
            }
        }
    }

    /// Kopiert Formen (samt Verbindern dazwischen), leicht versetzt. Gibt die neuen Kennungen zurück.
    pub fn duplicate(&mut self, ids: &[u64]) -> Vec<u64> {
        let mut map: HashMap<u64, u64> = HashMap::new();
        let originals: Vec<Shape> = self.shapes.iter().filter(|s| ids.contains(&s.id)).cloned().collect();
        for s in originals {
            let nid = self.new_id();
            let mut c = s.clone();
            c.id = nid;
            c.x += 24.0;
            c.y += 24.0;
            map.insert(s.id, nid);
            self.shapes.push(c);
        }
        let es: Vec<Edge> = self.edges.iter().filter(|e| map.contains_key(&e.a) && map.contains_key(&e.b)).cloned().collect();
        for e in es {
            let nid = self.new_id();
            self.edges.push(Edge { id: nid, a: map[&e.a], b: map[&e.b], ..e });
        }
        let mut out: Vec<u64> = map.values().copied().collect();
        out.sort();
        out
    }

    pub fn bounds(&self, ids: &[u64]) -> Option<(f32, f32, f32, f32)> {
        let mut it = self.shapes.iter().filter(|s| ids.is_empty() || ids.contains(&s.id));
        let first = it.next()?;
        let mut b = (first.x, first.y, first.right(), first.bottom());
        for s in it {
            b = (b.0.min(s.x), b.1.min(s.y), b.2.max(s.right()), b.3.max(s.bottom()));
        }
        Some(b)
    }

    pub fn align(&mut self, ids: &[u64], how: Align) {
        let sel: Vec<Shape> = self.shapes.iter().filter(|s| ids.contains(&s.id)).cloned().collect();
        if sel.len() < 2 {
            return;
        }
        let min_x = sel.iter().map(|s| s.x).fold(f32::MAX, f32::min);
        let max_r = sel.iter().map(|s| s.right()).fold(f32::MIN, f32::max);
        let min_y = sel.iter().map(|s| s.y).fold(f32::MAX, f32::min);
        let max_b = sel.iter().map(|s| s.bottom()).fold(f32::MIN, f32::max);
        match how {
            Align::Left | Align::HCenter | Align::Right | Align::Top | Align::VCenter | Align::Bottom => {
                for s in self.shapes.iter_mut().filter(|s| ids.contains(&s.id)) {
                    match how {
                        Align::Left => s.x = min_x,
                        Align::Right => s.x = max_r - s.w,
                        Align::HCenter => s.x = (min_x + max_r) / 2.0 - s.w / 2.0,
                        Align::Top => s.y = min_y,
                        Align::Bottom => s.y = max_b - s.h,
                        Align::VCenter => s.y = (min_y + max_b) / 2.0 - s.h / 2.0,
                        _ => {}
                    }
                }
            }
            Align::DistributeH | Align::DistributeV => {
                let horizontal = how == Align::DistributeH;
                let mut order: Vec<(u64, f32, f32)> = sel.iter().map(|s| if horizontal { (s.id, s.x, s.w) } else { (s.id, s.y, s.h) }).collect();
                order.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
                let total: f32 = order.iter().map(|o| o.2).sum();
                let span = if horizontal { max_r - min_x } else { max_b - min_y };
                let gap = (span - total) / (order.len() as f32 - 1.0);
                let mut pos = if horizontal { min_x } else { min_y };
                for (id, _, size) in order {
                    if let Some(s) = self.shape_mut(id) {
                        if horizontal {
                            s.x = pos;
                        } else {
                            s.y = pos;
                        }
                    }
                    pos += size + gap;
                }
            }
        }
    }

    // ------------------------------------------------------------ Treffer

    /// Oberste Form unter dem Punkt (Gruppen zählen zuletzt).
    pub fn hit_shape(&self, p: P) -> Option<u64> {
        self.shapes
            .iter()
            .rev()
            .filter(|s| s.kind != Kind::Group)
            .find(|s| s.contains(p))
            .or_else(|| self.shapes.iter().rev().filter(|s| s.kind == Kind::Group).find(|s| s.contains(p)))
            .map(|s| s.id)
    }

    pub fn hit_edge(&self, p: P, tol: f32) -> Option<u64> {
        self.edges.iter().rev().find(|e| self.edge_points(e).map_or(false, |pts| pts.windows(2).any(|w| dist_to_segment(p, w[0], w[1]) <= tol))).map(|e| e.id)
    }

    pub fn shapes_in_rect(&self, r: (f32, f32, f32, f32)) -> Vec<u64> {
        let (x0, y0, x1, y1) = (r.0.min(r.2), r.1.min(r.3), r.0.max(r.2), r.1.max(r.3));
        self.shapes.iter().filter(|s| s.x >= x0 && s.y >= y0 && s.right() <= x1 && s.bottom() <= y1).map(|s| s.id).collect()
    }

    // ------------------------------------------------------------ Verbinder

    pub fn edge_points(&self, e: &Edge) -> Option<Vec<P>> {
        Some(route(self.shape(e.a)?, self.shape(e.b)?, e.elbow))
    }

    // ------------------------------------------------------------ Organigramm anordnen

    /// Ordnet verbundene Formen als Baum an (Vorgesetzte oben, mittig über ihren Untergebenen).
    /// `only`: nur die Bäume unterhalb dieser Formen; leer = alle Bäume des Dokuments.
    pub fn layout_tree(&mut self, only: &[u64], hgap: f32, vgap: f32) {
        let has_parent: HashSet<u64> = self.edges.iter().map(|e| e.b).collect();
        let has_child: HashSet<u64> = self.edges.iter().map(|e| e.a).collect();
        let roots: Vec<u64> = if only.is_empty() {
            self.shapes.iter().map(|s| s.id).filter(|id| has_child.contains(id) && !has_parent.contains(id)).collect()
        } else {
            only.iter().copied().filter(|id| has_child.contains(id) && self.parent(*id).map_or(true, |p| !only.contains(&p))).collect()
        };
        if roots.is_empty() {
            return;
        }
        // Bäume aufbauen (wer zuerst kommt, behält das Kind; schützt vor Schleifen)
        let mut seen: HashSet<u64> = HashSet::new();
        let mut kids: HashMap<u64, Vec<u64>> = HashMap::new();
        let mut level: HashMap<u64, usize> = HashMap::new();
        let mut trees: Vec<u64> = Vec::new();
        for r in &roots {
            if !seen.insert(*r) {
                continue;
            }
            trees.push(*r);
            level.insert(*r, 0);
            let mut queue = vec![*r];
            let mut qi = 0;
            while qi < queue.len() {
                let n = queue[qi];
                qi += 1;
                let mut list = Vec::new();
                for c in self.children(n) {
                    if seen.insert(c) {
                        list.push(c);
                        level.insert(c, level[&n] + 1);
                        queue.push(c);
                    }
                }
                kids.insert(n, list);
            }
        }
        let size = |doc: &Doc, id: u64| doc.shape(id).map(|s| (s.w, s.h)).unwrap_or((0.0, 0.0));
        // Ebenenhöhen
        let max_level = level.values().copied().max().unwrap_or(0);
        let mut level_h = vec![0.0f32; max_level + 1];
        for (id, l) in &level {
            level_h[*l] = level_h[*l].max(size(self, *id).1);
        }
        let mut level_y = vec![0.0f32; max_level + 1];
        for l in 1..=max_level {
            level_y[l] = level_y[l - 1] + level_h[l - 1] + vgap;
        }
        // Breiten
        fn width(doc: &Doc, kids: &HashMap<u64, Vec<u64>>, id: u64, hgap: f32, memo: &mut HashMap<u64, f32>) -> f32 {
            let own = doc.shape(id).map(|s| s.w).unwrap_or(0.0);
            let list = kids.get(&id).cloned().unwrap_or_default();
            let mut total = 0.0;
            for (i, c) in list.iter().enumerate() {
                total += width(doc, kids, *c, hgap, memo);
                if i > 0 {
                    total += hgap;
                }
            }
            let w = own.max(total);
            memo.insert(id, w);
            w
        }
        let mut memo: HashMap<u64, f32> = HashMap::new();
        for r in &trees {
            width(self, &kids, *r, hgap, &mut memo);
        }
        // Plätze (lokal, ab x = 0)
        let mut pos: HashMap<u64, (f32, f32)> = HashMap::new();
        fn place(doc: &Doc, kids: &HashMap<u64, Vec<u64>>, memo: &HashMap<u64, f32>, level: &HashMap<u64, usize>, level_y: &[f32], id: u64, left: f32, hgap: f32, pos: &mut HashMap<u64, (f32, f32)>) {
            let sub = memo[&id];
            let own = doc.shape(id).map(|s| s.w).unwrap_or(0.0);
            let list = kids.get(&id).cloned().unwrap_or_default();
            if list.is_empty() {
                pos.insert(id, (left + (sub - own) / 2.0, level_y[level[&id]]));
                return;
            }
            let total: f32 = list.iter().map(|c| memo[c]).sum::<f32>() + hgap * (list.len() as f32 - 1.0);
            let mut x = left + (sub - total) / 2.0;
            for c in &list {
                place(doc, kids, memo, level, level_y, *c, x, hgap, pos);
                x += memo[c] + hgap;
            }
            // mittig über dem ersten und letzten Untergebenen
            let centre = |c: u64| pos[&c].0 + doc.shape(c).map(|s| s.w).unwrap_or(0.0) / 2.0;
            let mid = (centre(list[0]) + centre(*list.last().unwrap())) / 2.0;
            pos.insert(id, (mid - own / 2.0, level_y[level[&id]]));
        }
        let mut cursor = 0.0;
        for r in &trees {
            place(self, &kids, &memo, &level, &level_y, *r, cursor, hgap, &mut pos);
            cursor += memo[r] + hgap * 2.0;
        }
        // Verankerung: obere linke Ecke der bisherigen Wurzeln
        let ox = trees.iter().filter_map(|r| self.shape(*r)).map(|s| s.x).fold(f32::MAX, f32::min);
        let oy = trees.iter().filter_map(|r| self.shape(*r)).map(|s| s.y).fold(f32::MAX, f32::min);
        let lx = trees.iter().map(|r| pos[r].0).fold(f32::MAX, f32::min);
        for (id, (x, y)) in pos {
            if let Some(s) = self.shape_mut(id) {
                s.x = ox + (x - lx);
                s.y = oy + y;
            }
        }
    }
}

fn dist_to_segment(p: P, a: P, b: P) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 <= 0.0 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) };
    let (qx, qy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - qx).powi(2) + (p.1 - qy).powi(2)).sqrt()
}

/// Weg eines Verbinders von `a` nach `b`: an den zugewandten Seiten, optional rechtwinklig.
pub fn route(a: &Shape, b: &Shape, elbow: bool) -> Vec<P> {
    let (ac, bc) = (a.center(), b.center());
    let below = b.y >= a.bottom();
    let above = b.bottom() <= a.y;
    let right_of = b.x >= a.right();
    let left_of = b.right() <= a.x;
    if below || above || !(right_of || left_of) {
        let down = if below { true } else if above { false } else { bc.1 >= ac.1 };
        let p0 = (ac.0, if down { a.bottom() } else { a.y });
        let p3 = (bc.0, if down { b.y } else { b.bottom() });
        if !elbow || (p0.0 - p3.0).abs() < 0.5 {
            return vec![p0, p3];
        }
        let my = (p0.1 + p3.1) / 2.0;
        vec![p0, (p0.0, my), (p3.0, my), p3]
    } else {
        let p0 = (if right_of { a.right() } else { a.x }, ac.1);
        let p3 = (if right_of { b.x } else { b.right() }, bc.1);
        if !elbow || (p0.1 - p3.1).abs() < 0.5 {
            return vec![p0, p3];
        }
        let mx = (p0.0 + p3.0) / 2.0;
        vec![p0, (mx, p0.1), (mx, p3.1), p3]
    }
}

// ================================================================ Speichern

impl Doc {
    pub fn to_json_string(&self) -> String {
        let num = |x: f32| Json::Num(x as f64);
        json::obj(vec![
            ("v", json::n(1)),
            ("next_id", json::n(self.next_id as i64)),
            (
                "shapes",
                Json::Arr(
                    self.shapes
                        .iter()
                        .map(|s| {
                            json::obj(vec![
                                ("id", json::n(s.id as i64)),
                                ("k", json::s(s.kind.as_str())),
                                ("x", num(s.x)),
                                ("y", num(s.y)),
                                ("w", num(s.w)),
                                ("h", num(s.h)),
                                ("t", json::s(&s.text)),
                                ("f", json::n(s.fill as i64)),
                                ("fs", num(s.font)),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "edges",
                Json::Arr(
                    self.edges
                        .iter()
                        .map(|e| {
                            json::obj(vec![
                                ("id", json::n(e.id as i64)),
                                ("a", json::n(e.a as i64)),
                                ("b", json::n(e.b as i64)),
                                ("e", json::b(e.elbow)),
                                ("ar", json::b(e.arrow)),
                                ("l", json::s(&e.label)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
        .to_string()
    }

    pub fn from_json_str(t: &str) -> Result<Doc, String> {
        let j = Json::parse(t)?;
        if j.get("shapes").as_arr().is_empty() && j.get("edges").as_arr().is_empty() && j.get("v").is_null() {
            return Err("Das ist keine Organiwerk-Datei.".into());
        }
        let f = |j: &Json, k: &str, d: f32| match j.get(k) {
            Json::Num(n) if n.is_finite() => *n as f32,
            _ => d,
        };
        let mut doc = Doc { next_id: j.get("next_id").as_i64().unwrap_or(0).max(0) as u64, ..Default::default() };
        for s in j.get("shapes").as_arr() {
            let Some(id) = s.get("id").as_i64() else { continue };
            doc.shapes.push(Shape {
                id: id as u64,
                kind: Kind::parse(s.get("k").as_str().unwrap_or("")),
                x: f(s, "x", 0.0),
                y: f(s, "y", 0.0),
                w: f(s, "w", 140.0).clamp(10.0, 5000.0),
                h: f(s, "h", 70.0).clamp(10.0, 5000.0),
                text: s.get("t").str_or_empty(),
                fill: s.get("f").as_i64().unwrap_or(0xFFFFFF).clamp(0, 0xFFFFFF) as u32,
                font: f(s, "fs", 14.0).clamp(6.0, 120.0),
            });
        }
        for e in j.get("edges").as_arr() {
            let (Some(id), Some(a), Some(b)) = (e.get("id").as_i64(), e.get("a").as_i64(), e.get("b").as_i64()) else { continue };
            if doc.shape(a as u64).is_some() && doc.shape(b as u64).is_some() {
                doc.edges.push(Edge { id: id as u64, a: a as u64, b: b as u64, elbow: e.get("e").as_bool(), arrow: e.get("ar").as_bool(), label: e.get("l").str_or_empty() });
            }
        }
        let used = doc.shapes.iter().map(|s| s.id).chain(doc.edges.iter().map(|e| e.id)).max().unwrap_or(0);
        doc.next_id = doc.next_id.max(used);
        Ok(doc)
    }

    /// Kleines Beispiel zum Ausprobieren.
    pub fn example() -> Doc {
        let mut d = Doc::default();
        let mk = |d: &mut Doc, name: &str, fill: u32| {
            let id = d.add_shape(Kind::Org, 0.0, 0.0);
            let s = d.shape_mut(id).unwrap();
            s.text = name.to_string();
            s.fill = fill;
            id
        };
        let gl = mk(&mut d, "Anna Meier\nGeschäftsleitung", 0xDCE8FA);
        let a = mk(&mut d, "Beat Keller\nLeiter Verkauf", 0xD5EFE3);
        let b = mk(&mut d, "Carla Rossi\nLeiterin Produktion", 0xFBEFC9);
        let c = mk(&mut d, "David Frei\nLeiter Finanzen", 0xE4DAF5);
        let a1 = mk(&mut d, "Eva Roth\nInnendienst", 0xFFFFFF);
        let a2 = mk(&mut d, "Felix Wyss\nAussendienst", 0xFFFFFF);
        for (p, k) in [(gl, a), (gl, b), (gl, c), (a, a1), (a, a2)] {
            d.connect(p, k, true, false);
        }
        d.layout_tree(&[], 30.0, 60.0);
        d
    }
}

// ================================================================ SVG

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn hex(c: u32) -> String {
    format!("#{:06X}", c & 0xFFFFFF)
}

/// Dunkle Schrift auf hellem Grund, helle auf dunklem.
pub fn text_color(fill: u32) -> u32 {
    let (r, g, b) = (((fill >> 16) & 255) as f32, ((fill >> 8) & 255) as f32, (fill & 255) as f32);
    if 0.299 * r + 0.587 * g + 0.114 * b < 110.0 {
        0xFFFFFF
    } else {
        0x151B1F
    }
}

impl Doc {
    pub fn to_svg(&self) -> String {
        let (x0, y0, x1, y1) = self.bounds(&[]).unwrap_or((0.0, 0.0, 200.0, 100.0));
        let m = 24.0;
        let (vx, vy, vw, vh) = (x0 - m, y0 - m, x1 - x0 + 2.0 * m, y1 - y0 + 2.0 * m);
        let mut o = String::new();
        o.push_str(&format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{vw:.0}\" height=\"{vh:.0}\" viewBox=\"{vx:.1} {vy:.1} {vw:.1} {vh:.1}\" font-family=\"Segoe UI, Helvetica, Arial, sans-serif\">\n"
        ));
        o.push_str("<defs><marker id=\"pfeil\" viewBox=\"0 0 10 10\" refX=\"9\" refY=\"5\" markerWidth=\"8\" markerHeight=\"8\" orient=\"auto-start-reverse\"><path d=\"M0,0 L10,5 L0,10 z\" fill=\"#3A4650\"/></marker></defs>\n");
        o.push_str(&format!("<rect x=\"{vx:.1}\" y=\"{vy:.1}\" width=\"{vw:.1}\" height=\"{vh:.1}\" fill=\"#FFFFFF\"/>\n"));
        let order = self.shapes.iter().filter(|s| s.kind == Kind::Group).chain(self.shapes.iter().filter(|s| s.kind != Kind::Group));
        let mut shapes_svg = String::new();
        for s in order {
            let fill = hex(s.fill);
            let stroke = "#3A4650";
            match s.kind {
                Kind::Rect => shapes_svg.push_str(&format!("<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"1.5\"/>\n", s.x, s.y, s.w, s.h)),
                Kind::Rounded | Kind::Org => shapes_svg.push_str(&format!("<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" rx=\"10\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"1.5\"/>\n", s.x, s.y, s.w, s.h)),
                Kind::Ellipse => {
                    let (cx, cy) = s.center();
                    shapes_svg.push_str(&format!("<ellipse cx=\"{cx:.1}\" cy=\"{cy:.1}\" rx=\"{:.1}\" ry=\"{:.1}\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"1.5\"/>\n", s.w / 2.0, s.h / 2.0));
                }
                Kind::Diamond => {
                    let (cx, cy) = s.center();
                    shapes_svg.push_str(&format!(
                        "<polygon points=\"{cx:.1},{:.1} {:.1},{cy:.1} {cx:.1},{:.1} {:.1},{cy:.1}\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"1.5\"/>\n",
                        s.y, s.right(), s.bottom(), s.x
                    ));
                }
                Kind::Group => shapes_svg.push_str(&format!("<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" rx=\"8\" fill=\"{fill}\" stroke=\"#8E9AA3\" stroke-width=\"1.5\" stroke-dasharray=\"6 4\"/>\n", s.x, s.y, s.w, s.h)),
                Kind::Text => {}
            }
            let tc = hex(if s.kind == Kind::Text || s.kind == Kind::Group { 0x151B1F } else { text_color(s.fill) });
            let lines: Vec<&str> = s.text.lines().collect();
            if lines.is_empty() {
                continue;
            }
            if s.kind == Kind::Group {
                shapes_svg.push_str(&format!("<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"{:.1}\" font-weight=\"600\" fill=\"{tc}\">{}</text>\n", s.x + 12.0, s.y + 12.0 + s.font, s.font, esc(&s.text.replace('\n', " "))));
                continue;
            }
            let lh = s.font * 1.25;
            let total = lh * lines.len() as f32;
            let (cx, cy) = s.center();
            let top = cy - total / 2.0 + lh * 0.8;
            for (i, l) in lines.iter().enumerate() {
                let (size, weight) = if s.kind == Kind::Org { if i == 0 { (s.font * 1.05, "600") } else { (s.font * 0.9, "400") } } else { (s.font, "400") };
                shapes_svg.push_str(&format!("<text x=\"{cx:.1}\" y=\"{:.1}\" font-size=\"{size:.1}\" font-weight=\"{weight}\" text-anchor=\"middle\" fill=\"{tc}\">{}</text>\n", top + lh * i as f32, esc(l)));
            }
        }
        let mut edges_svg = String::new();
        for e in &self.edges {
            let Some(pts) = self.edge_points(e) else { continue };
            let d: Vec<String> = pts.iter().enumerate().map(|(i, p)| format!("{}{:.1},{:.1}", if i == 0 { "M" } else { "L" }, p.0, p.1)).collect();
            edges_svg.push_str(&format!("<path d=\"{}\" fill=\"none\" stroke=\"#3A4650\" stroke-width=\"1.5\"{}/>\n", d.join(" "), if e.arrow { " marker-end=\"url(#pfeil)\"" } else { "" }));
            if !e.label.is_empty() {
                let (mx, my) = polyline_mid(&pts);
                edges_svg.push_str(&format!("<text x=\"{mx:.1}\" y=\"{:.1}\" font-size=\"12\" text-anchor=\"middle\" fill=\"#3A4650\" stroke=\"#FFFFFF\" stroke-width=\"4\" paint-order=\"stroke\">{}</text>\n", my - 4.0, esc(&e.label)));
            }
        }
        // Gruppen unten, dann Verbinder, dann übrige Formen: einfacher: Formen vor Verbindern, Verbinder enden an den Rändern
        o.push_str(&shapes_svg);
        o.push_str(&edges_svg);
        o.push_str("</svg>\n");
        o
    }
}

pub fn polyline_mid(pts: &[P]) -> P {
    let total: f32 = pts.windows(2).map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt()).sum();
    let mut left = total / 2.0;
    for w in pts.windows(2) {
        let len = ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt();
        if left <= len && len > 0.0 {
            let t = left / len;
            return (w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t);
        }
        left -= len;
    }
    pts.last().copied().unwrap_or((0.0, 0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_rundreise() {
        let d = Doc::example();
        assert_eq!(d.shapes.len(), 6);
        assert_eq!(d.edges.len(), 5);
        let back = Doc::from_json_str(&d.to_json_string()).unwrap();
        assert_eq!(back.shapes.len(), 6);
        assert_eq!(back.edges.len(), 5);
        assert_eq!(back.shapes[0].text, d.shapes[0].text);
        assert!((back.shapes[3].x - d.shapes[3].x).abs() < 0.01);
        assert!(Doc::from_json_str("{}").is_err());
        assert!(Doc::from_json_str("kaputt").is_err());
        // Verbinder zu fehlenden Formen fallen weg
        let t = r#"{"v":1,"shapes":[{"id":1,"k":"rect","x":0,"y":0,"w":10,"h":10}],"edges":[{"id":2,"a":1,"b":9}]}"#;
        assert!(Doc::from_json_str(t).unwrap().edges.is_empty());
    }

    #[test]
    fn baum_anordnen() {
        let mut d = Doc::example();
        let ids: Vec<u64> = d.shapes.iter().map(|s| s.id).collect();
        let (gl, a, b, c, a1, a2) = (ids[0], ids[1], ids[2], ids[3], ids[4], ids[5]);
        // durcheinanderbringen
        d.shape_mut(a1).unwrap().x = 900.0;
        d.shape_mut(c).unwrap().y = -50.0;
        d.layout_tree(&[], 30.0, 60.0);
        let s = |id| d.shape(id).unwrap().clone();
        // gleiche Ebene → gleiche y; Ebenen mit Abstand
        assert_eq!(s(a).y, s(b).y);
        assert_eq!(s(b).y, s(c).y);
        assert_eq!(s(a1).y, s(a2).y);
        assert!((s(a).y - (s(gl).bottom() + 60.0)).abs() < 0.01);
        assert!((s(a1).y - (s(a).bottom() + 60.0)).abs() < 0.01);
        // Vorgesetzte mittig über den Untergebenen
        let mid = (s(a).center().0 + s(c).center().0) / 2.0;
        assert!((s(gl).center().0 - mid).abs() < 0.01);
        let mid2 = (s(a1).center().0 + s(a2).center().0) / 2.0;
        assert!((s(a).center().0 - mid2).abs() < 0.01);
        // nichts überlappt auf einer Ebene
        assert!(s(b).x >= s(a).right() + 29.9 && s(c).x >= s(b).right() + 29.9);
        assert!(s(a2).x >= s(a1).right() + 29.9);
    }

    #[test]
    fn baum_mit_schleife_haengt_nicht() {
        let mut d = Doc::default();
        let a = d.add_shape(Kind::Rect, 0.0, 0.0);
        let b = d.add_shape(Kind::Rect, 200.0, 0.0);
        let c = d.add_shape(Kind::Rect, 400.0, 0.0);
        d.connect(a, b, true, false);
        d.connect(b, c, true, false);
        d.connect(c, a, true, false); // Kreis: keine Wurzel
        d.layout_tree(&[], 20.0, 40.0);
        d.layout_tree(&[a], 20.0, 40.0); // mit Auswahl: kein Absturz
        assert_eq!(d.shapes.len(), 3);
    }

    #[test]
    fn kind_untergebene_und_nachbarn() {
        let mut d = Doc::default();
        let boss = d.add_shape(Kind::Org, 0.0, 0.0);
        let k1 = d.add_child(boss).unwrap();
        let k2 = d.add_child(boss).unwrap();
        assert!(d.shape(k2).unwrap().x > d.shape(k1).unwrap().right());
        let sib = d.add_sibling(k1).unwrap();
        assert_eq!(d.parent(sib), Some(boss));
        let loner = d.add_shape(Kind::Rect, 500.0, 500.0);
        let n = d.add_sibling(loner).unwrap();
        assert!(d.parent(n).is_none());
        assert!(d.connect(boss, k1, true, false).is_none()); // doppelt
        assert!(d.connect(boss, boss, true, false).is_none()); // selbst
        d.remove_shapes(&[boss]);
        assert!(d.edges.is_empty());
    }

    #[test]
    fn verbinder_wege() {
        let mut d = Doc::default();
        let a = d.add_shape(Kind::Rect, 100.0, 50.0);
        let b = d.add_shape(Kind::Rect, 300.0, 250.0);
        let (sa, sb) = (d.shape(a).unwrap().clone(), d.shape(b).unwrap().clone());
        let p = route(&sa, &sb, true);
        assert_eq!(p.len(), 4);
        assert_eq!(p[0].1, sa.bottom());
        assert_eq!(p[3].1, sb.y);
        assert_eq!(p[1].1, p[2].1);
        assert_eq!(route(&sa, &sb, false).len(), 2);
        // nebeneinander → waagrecht
        let c = d.add_shape(Kind::Rect, 600.0, 50.0);
        let sc = d.shape(c).unwrap().clone();
        let h = route(&sa, &sc, true);
        assert_eq!(h[0].0, sa.right());
        assert_eq!(h.last().unwrap().0, sc.x);
        // Treffer auf der Linie
        let e = d.connect(a, b, true, true).unwrap();
        let mid = polyline_mid(&p);
        assert_eq!(d.hit_edge(mid, 3.0), Some(e));
        assert_eq!(d.hit_edge((0.0, 0.0), 3.0), None);
    }

    #[test]
    fn treffer_formen() {
        let mut d = Doc::default();
        let g = d.add_shape(Kind::Group, 200.0, 200.0);
        let r = d.add_shape(Kind::Rect, 200.0, 200.0);
        let e = d.add_shape(Kind::Ellipse, 700.0, 700.0);
        assert_eq!(d.hit_shape((200.0, 200.0)), Some(r)); // Form vor Gruppe
        let gs = d.shape(g).unwrap().clone();
        assert_eq!(d.hit_shape((gs.x + 3.0, gs.bottom() - 3.0)), Some(g)); // Rand der Gruppe
        let es = d.shape(e).unwrap().clone();
        assert_eq!(d.hit_shape((es.x + 2.0, es.y + 2.0)), None); // Ecke der Ellipse ist leer
        assert_eq!(d.hit_shape(es.center()), Some(e));
        assert_eq!(d.shapes_in_rect((es.x - 1.0, es.y - 1.0, es.right() + 1.0, es.bottom() + 1.0)), vec![e]);
    }

    #[test]
    fn ausrichten_und_kopieren() {
        let mut d = Doc::default();
        let a = d.add_shape(Kind::Rect, 100.0, 100.0);
        let b = d.add_shape(Kind::Rect, 300.0, 180.0);
        let c = d.add_shape(Kind::Rect, 800.0, 90.0);
        d.align(&[a, b, c], Align::Top);
        let y = d.shape(a).unwrap().y;
        assert!(d.shapes.iter().all(|s| s.y == y));
        d.align(&[a, b, c], Align::DistributeH);
        let (sa, sb, sc) = (d.shape(a).unwrap().clone(), d.shape(b).unwrap().clone(), d.shape(c).unwrap().clone());
        assert!(((sb.x - sa.right()) - (sc.x - sb.right())).abs() < 0.01);
        d.connect(a, b, true, true);
        let copies = d.duplicate(&[a, b]);
        assert_eq!(copies.len(), 2);
        assert_eq!(d.edges.len(), 2);
        assert_eq!(d.shapes.len(), 5);
    }

    #[test]
    fn svg_ausgabe() {
        let mut d = Doc::example();
        d.shapes[0].text = "A & <B>\nZeile".into();
        let svg = d.to_svg();
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("A &amp; &lt;B&gt;"));
        assert_eq!(svg.matches("fill=\"none\"").count(), 5);
        assert!(svg.trim_end().ends_with("</svg>"));
        assert_eq!(text_color(0x2B3A47), 0xFFFFFF);
        assert_eq!(text_color(0xDCE8FA), 0x151B1F);
        assert!(Doc::default().to_svg().contains("<svg"));
    }
}
