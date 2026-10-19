//! Oberfläche (egui): links Inhalt und Darstellung wählen, rechts Vorschau
//! und Export (PNG/SVG).

use crate::qr::{self, Content, Ec, Style, SwissQr, WifiEnc};
use crate::save::save_bytes;
use crate::sprache::t;
use crate::stil::{self, Sym};
use crate::thema;

#[derive(PartialEq, Clone, Copy)]
enum Kind {
    Text,
    Wifi,
    Email,
    Contact,
    Swiss,
}

pub struct App {
    kind: Kind,
    text: String,
    ssid: String,
    wpass: String,
    wenc: WifiEnc,
    whidden: bool,
    eto: String,
    esubj: String,
    ebody: String,
    cname: String,
    cphone: String,
    cemail: String,
    corg: String,
    sw: SwissQr,
    st: Style,

    last_key: String,
    tex: Option<egui::TextureHandle>,
    err: Option<String>,
}

impl Default for App {
    fn default() -> Self {
        App {
            kind: Kind::Text,
            text: "https://oldini.ch".to_owned(),
            ssid: String::new(),
            wpass: String::new(),
            wenc: WifiEnc::Wpa,
            whidden: false,
            eto: String::new(),
            esubj: String::new(),
            ebody: String::new(),
            cname: String::new(),
            cphone: String::new(),
            cemail: String::new(),
            corg: String::new(),
            sw: SwissQr {
                currency: "CHF".into(),
                cr_country: "CH".into(),
                ref_type: "NON".into(),
                ..Default::default()
            },
            st: Style::default(),
            last_key: String::new(),
            tex: None,
            err: None,
        }
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        for (_, f) in style.text_styles.iter_mut() {
            f.size *= 1.05;
        }
        cc.egui_ctx.set_style(style);
        // Im Browser (wasm) gibt es weder Umgebungsvariable noch Datei – dort bleibt es hell.
        thema::apply(&cc.egui_ctx);
        thema::titelleiste(cc, [0xE0, 0x7A, 0x1F]);
        stil::init(&cc.egui_ctx, [0xE0, 0x7A, 0x1F]);
        Self::default()
    }

    fn content(&self) -> Content {
        match self.kind {
            Kind::Text => Content::Text(self.text.clone()),
            Kind::Wifi => Content::Wifi {
                ssid: self.ssid.clone(),
                pass: self.wpass.clone(),
                enc: self.wenc,
                hidden: self.whidden,
            },
            Kind::Email => Content::Email {
                to: self.eto.clone(),
                subject: self.esubj.clone(),
                body: self.ebody.clone(),
            },
            Kind::Contact => Content::Contact {
                name: self.cname.clone(),
                phone: self.cphone.clone(),
                email: self.cemail.clone(),
                org: self.corg.clone(),
            },
            Kind::Swiss => Content::Swiss(Box::new(self.sw.clone())),
        }
    }

    /// Stil mit den für Swiss QR nötigen Vorgaben (M + Schweizer Kreuz).
    fn effective_style(&self) -> Style {
        let mut s = self.st.clone();
        if self.kind == Kind::Swiss {
            s.ec = Ec::M;
            s.cross = true;
        } else {
            s.cross = false;
        }
        s
    }

    fn key(&self, payload: &str, st: &Style) -> String {
        format!(
            "{payload}|{:?}{:?}{}{}{}{}",
            st.fg, st.bg, st.scale, st.quiet, st.ec as u8, st.cross as u8
        )
    }

    fn refresh(&mut self, ctx: &egui::Context, payload: &str, st: &Style) {
        let key = self.key(payload, st);
        if key == self.last_key && (self.tex.is_some() || self.err.is_some()) {
            return;
        }
        self.last_key = key;
        match qr::to_rgba(payload, st) {
            Ok((raw, size)) => {
                let img =
                    egui::ColorImage::from_rgba_unmultiplied([size as usize, size as usize], &raw);
                self.tex = Some(ctx.load_texture("qr", img, egui::TextureOptions::NEAREST));
                self.err = None;
            }
            Err(e) => {
                self.err = Some(e);
                self.tex = None;
            }
        }
    }
}

fn kind_label(k: Kind) -> &'static str {
    match k {
        Kind::Text => t("Text / Link"),
        Kind::Wifi => t("WLAN-Zugang"),
        Kind::Email => t("E-Mail"),
        Kind::Contact => t("Kontakt (vCard)"),
        Kind::Swiss => t("Schweizer QR-Rechnung"),
    }
}

fn kind_sym(k: Kind) -> Sym {
    match k {
        Kind::Text => Sym::Link,
        Kind::Wifi => Sym::Schluessel,
        Kind::Email => Sym::Mail,
        Kind::Contact => Sym::Datei,
        Kind::Swiss => Sym::Schild,
    }
}

/// Einzeiliges Textfeld über die ganze Breite mit Beschriftung darüber.
fn txt(ui: &mut egui::Ui, label: &str, v: &mut String) {
    stil::feld(ui, label, |ui| {
        ui.add(egui::TextEdit::singleline(v).desired_width(f32::INFINITY));
    });
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let payload = self.content().payload();
        let eff = self.effective_style();
        self.refresh(ctx, &payload, &eff);
        let ok = self.err.is_none() && self.tex.is_some();

        // ---------------- Kopfleiste
        stil::kopfleiste(ctx, |ui| {
            stil::titel(ui, "Codewerk");
            stil::trenner(ui);
            let cur = self.kind;
            stil::menue(ui, true, kind_sym(cur), kind_label(cur), |ui| {
                for k in [Kind::Text, Kind::Wifi, Kind::Email, Kind::Contact, Kind::Swiss] {
                    if stil::eintrag(ui, true, kind_sym(k), kind_label(k)).clicked() {
                        self.kind = k;
                    }
                }
            });
            stil::rechts(ui, |ui| {
                stil::grau(ui, t("QR-Codes erstellen"));
            });
        });

        // ---------------- Fussleiste
        stil::fussleiste(ctx, |ui| {
            ui.horizontal(|ui| {
                match &self.err {
                    Some(e) => {
                        ui.label(egui::RichText::new(e).size(13.0).color(stil::rot()));
                    }
                    None => {
                        stil::grau(ui, &format!("{} · {} {}", kind_label(self.kind), payload.chars().count(), t("Zeichen")));
                    }
                }
                stil::rechts(ui, |ui| {
                    if stil::haupt(ui, ok, Sym::Download, t("Als PNG speichern")).clicked() {
                        if let Ok(bytes) = qr::to_png(&payload, &eff) {
                            save_bytes("qr-code.png", &bytes);
                        }
                    }
                    if stil::knopf_an(ui, ok, Sym::Download, t("Als SVG speichern")).clicked() {
                        if let Ok(s) = qr::to_svg(&payload, &eff) {
                            save_bytes("qr-code.svg", s.as_bytes());
                        }
                    }
                });
            });
        });

        // ---------------- rechts: Inhalt und Darstellung
        stil::seitenleiste(ctx, 340.0, |ui| {
            stil::karte(ui, "inhalt", kind_sym(self.kind), t("Inhalt"), true, |ui| {
                stil::feld(ui, t("Art"), |ui| {
                    let opts: Vec<(Kind, &str)> = [Kind::Text, Kind::Wifi, Kind::Email, Kind::Contact, Kind::Swiss].iter().map(|k| (*k, kind_label(*k))).collect();
                    stil::liste(ui, "kind", &mut self.kind, &opts);
                });
                self.content_fields(ui);
            });
            stil::karte(ui, "darstellung", Sym::Raster, t("Darstellung"), true, |ui| {
                ui.horizontal(|ui| {
                    ui.label(t("Farbe:"));
                    ui.color_edit_button_srgb(&mut self.st.fg);
                    ui.add_space(8.0);
                    ui.label(t("Hintergrund:"));
                    ui.color_edit_button_srgb(&mut self.st.bg);
                });
                stil::feld(ui, t("Grösse:"), |ui| {
                    ui.add(egui::Slider::new(&mut self.st.scale, 2..=30).suffix(t(" px/Modul")));
                });
                stil::feld(ui, t("Rand:"), |ui| {
                    ui.add(egui::Slider::new(&mut self.st.quiet, 0..=10).suffix(t(" Module")));
                });
                stil::feld(ui, t("Fehlerkorrektur:"), |ui| {
                    if self.kind == Kind::Swiss {
                        stil::grau(ui, t("M + Schweizer Kreuz (vorgeschrieben)"));
                    } else {
                        let opts: Vec<(Ec, &str)> = [Ec::L, Ec::M, Ec::Q, Ec::H].iter().map(|e| (*e, e.label())).collect();
                        stil::liste(ui, "ec", &mut self.st.ec, &opts);
                    }
                });
            });
        });

        // ---------------- Mitte: Vorschau
        stil::mitte(ctx, |ui| {
            if let Some(err) = &self.err {
                stil::leer(ui, Sym::Warnung, t("Kein QR-Code"), err);
            } else if let Some(tex) = &self.tex {
                ui.vertical_centered(|ui| {
                    let avail = ui.available_width().min(ui.available_height() - 40.0);
                    let d = avail.clamp(120.0, 520.0);
                    ui.add_space(((ui.available_height() - d) / 2.0).max(10.0));
                    ui.image(egui::load::SizedTexture::new(tex.id(), egui::vec2(d, d)));
                });
            }
        });
    }
}
impl App {
    fn content_fields(&mut self, ui: &mut egui::Ui) {
        match self.kind {
            Kind::Text => {
                stil::feld(ui, t("Text oder Link:"), |ui| {
                    ui.add(egui::TextEdit::multiline(&mut self.text).desired_rows(4).desired_width(f32::INFINITY));
                });
            }
            Kind::Wifi => {
                txt(ui, t("Netzname (SSID):"), &mut self.ssid);
                txt(ui, t("Passwort:"), &mut self.wpass);
                stil::feld(ui, t("Verschlüsselung:"), |ui| {
                    let opts = [(WifiEnc::Wpa, "WPA/WPA2"), (WifiEnc::Wep, "WEP"), (WifiEnc::None, t("offen"))];
                    stil::liste(ui, "wenc", &mut self.wenc, &opts);
                });
                stil::schalter(ui, &mut self.whidden, t("Verstecktes Netz:"));
            }
            Kind::Email => {
                txt(ui, t("An:"), &mut self.eto);
                txt(ui, t("Betreff:"), &mut self.esubj);
                stil::feld(ui, t("Text:"), |ui| {
                    ui.add(egui::TextEdit::multiline(&mut self.ebody).desired_rows(3).desired_width(f32::INFINITY));
                });
            }
            Kind::Contact => {
                txt(ui, t("Name:"), &mut self.cname);
                txt(ui, t("Firma:"), &mut self.corg);
                txt(ui, t("Telefon:"), &mut self.cphone);
                txt(ui, t("E-Mail:"), &mut self.cemail);
            }
            Kind::Swiss => self.swiss_fields(ui),
        }
    }

    fn swiss_fields(&mut self, ui: &mut egui::Ui) {
        let sw = &mut self.sw;
        txt(ui, t("IBAN / QR-IBAN:"), &mut sw.iban);
        stil::feld(ui, t("Betrag:"), |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut sw.amount).desired_width(110.0));
                egui::ComboBox::from_id_salt("ccy")
                    .selected_text(if sw.currency.is_empty() { "CHF" } else { sw.currency.as_str() })
                    .width(70.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut sw.currency, "CHF".into(), "CHF");
                        ui.selectable_value(&mut sw.currency, "EUR".into(), "EUR");
                    });
            });
        });

        ui.add_space(4.0);
        ui.label(egui::RichText::new(t("Empfänger")).strong());
        txt(ui, t("Name:"), &mut sw.cr_name);
        ui.columns(2, |c| {
            txt(&mut c[0], t("Strasse:"), &mut sw.cr_street);
            txt(&mut c[1], t("Nr.:"), &mut sw.cr_bno);
        });
        ui.columns(2, |c| {
            txt(&mut c[0], t("PLZ / Ort:"), &mut sw.cr_zip);
            txt(&mut c[1], " ", &mut sw.cr_city);
        });
        txt(ui, t("Land:"), &mut sw.cr_country);

        ui.add_space(4.0);
        ui.label(egui::RichText::new(t("Zahlungspflichtiger (optional)")).strong());
        txt(ui, t("Name:"), &mut sw.db_name);
        ui.columns(2, |c| {
            txt(&mut c[0], t("Strasse:"), &mut sw.db_street);
            txt(&mut c[1], t("Nr.:"), &mut sw.db_bno);
        });
        ui.columns(2, |c| {
            txt(&mut c[0], t("PLZ / Ort:"), &mut sw.db_zip);
            txt(&mut c[1], " ", &mut sw.db_city);
        });
        txt(ui, t("Land:"), &mut sw.db_country);

        ui.add_space(4.0);
        ui.label(egui::RichText::new(t("Referenz & Mitteilung")).strong());
        stil::feld(ui, t("Referenztyp:"), |ui| {
            let opts = [("NON", t("keine (NON)")), ("QRR", t("QR-Referenz (QRR)")), ("SCOR", "Creditor Reference (SCOR)")];
            let mut cur: &str = match sw.ref_type.as_str() {
                "QRR" => "QRR",
                "SCOR" => "SCOR",
                _ => "NON",
            };
            if stil::liste(ui, "rtype", &mut cur, &opts) {
                sw.ref_type = cur.to_string();
            }
        });
        if sw.ref_type != "NON" {
            txt(ui, t("Referenz:"), &mut sw.reference);
        }
        txt(ui, t("Mitteilung:"), &mut sw.message);
    }
}
