//! Oberfläche (egui): links Inhalt und Darstellung wählen, rechts Vorschau
//! und Export (PNG/SVG).

use crate::qr::{self, Content, Ec, Style, SwissQr, WifiEnc};
use crate::save::save_bytes;
use crate::sprache::t;
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

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let payload = self.content().payload();
        let eff = self.effective_style();
        self.refresh(ctx, &payload, &eff);

        egui::SidePanel::left("eingabe")
            .resizable(false)
            .min_width(340.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(8.0);
                    ui.heading("Codewerk");
                    ui.label(egui::RichText::new(t("QR-Codes erstellen")).weak());
                    ui.add_space(10.0);

                    ui.label(t("Inhalt"));
                    egui::ComboBox::from_id_salt("kind")
                        .selected_text(match self.kind {
                            Kind::Text => t("Text / Link"),
                            Kind::Wifi => t("WLAN-Zugang"),
                            Kind::Email => t("E-Mail"),
                            Kind::Contact => t("Kontakt (vCard)"),
                            Kind::Swiss => t("Schweizer QR-Rechnung"),
                        })
                        .width(300.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.kind, Kind::Text, t("Text / Link"));
                            ui.selectable_value(&mut self.kind, Kind::Wifi, t("WLAN-Zugang"));
                            ui.selectable_value(&mut self.kind, Kind::Email, t("E-Mail"));
                            ui.selectable_value(&mut self.kind, Kind::Contact, t("Kontakt (vCard)"));
                            ui.selectable_value(
                                &mut self.kind,
                                Kind::Swiss,
                                t("Schweizer QR-Rechnung"),
                            );
                        });
                    ui.add_space(8.0);

                    self.content_fields(ui);

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(6.0);
                    ui.label(t("Darstellung"));
                    grid(ui, "style", |ui| {
                        ui.label(t("Farbe:"));
                        ui.horizontal(|ui| {
                            ui.color_edit_button_srgb(&mut self.st.fg);
                            ui.label(t("Hintergrund:"));
                            ui.color_edit_button_srgb(&mut self.st.bg);
                        });
                        ui.end_row();
                        ui.label(t("Grösse:"));
                        ui.add(egui::Slider::new(&mut self.st.scale, 2..=30).suffix(t(" px/Modul")));
                        ui.end_row();
                        ui.label(t("Rand:"));
                        ui.add(egui::Slider::new(&mut self.st.quiet, 0..=10).suffix(t(" Module")));
                        ui.end_row();
                        if self.kind == Kind::Swiss {
                            ui.label(t("Fehlerkorrektur:"));
                            ui.label(egui::RichText::new(t("M + Schweizer Kreuz (vorgeschrieben)")).weak());
                            ui.end_row();
                        } else {
                            ui.label(t("Fehlerkorrektur:"));
                            egui::ComboBox::from_id_salt("ec")
                                .selected_text(self.st.ec.label())
                                .show_ui(ui, |ui| {
                                    for ec in [Ec::L, Ec::M, Ec::Q, Ec::H] {
                                        ui.selectable_value(&mut self.st.ec, ec, ec.label());
                                    }
                                });
                            ui.end_row();
                        }
                    });
                    ui.add_space(10.0);
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(10.0);
            ui.vertical_centered(|ui| {
                if let Some(err) = &self.err {
                    ui.add_space(40.0);
                    ui.colored_label(thema::pick(egui::Color32::from_rgb(0xc0, 0x39, 0x2b), egui::Color32::from_rgb(0xf8, 0x71, 0x71)), err);
                } else if let Some(tex) = &self.tex {
                    let avail = ui.available_width().min(ui.available_height() - 90.0);
                    let d = avail.clamp(120.0, 440.0);
                    ui.image(egui::load::SizedTexture::new(tex.id(), egui::vec2(d, d)));
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.add_space((ui.available_width() - 260.0).max(0.0) / 2.0);
                        if ui
                            .add_sized([120.0, 34.0], egui::Button::new(t("Als PNG speichern")))
                            .clicked()
                        {
                            if let Ok(bytes) = qr::to_png(&payload, &eff) {
                                save_bytes("qr-code.png", &bytes);
                            }
                        }
                        ui.add_space(8.0);
                        if ui
                            .add_sized([120.0, 34.0], egui::Button::new(t("Als SVG speichern")))
                            .clicked()
                        {
                            if let Ok(s) = qr::to_svg(&payload, &eff) {
                                save_bytes("qr-code.svg", s.as_bytes());
                            }
                        }
                    });
                }
            });
        });
    }
}

impl App {
    fn content_fields(&mut self, ui: &mut egui::Ui) {
        match self.kind {
            Kind::Text => {
                ui.label(t("Text oder Link:"));
                ui.add(
                    egui::TextEdit::multiline(&mut self.text)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY),
                );
            }
            Kind::Wifi => {
                grid(ui, "wifi", |ui| {
                    ui.label(t("Netzname (SSID):"));
                    ui.text_edit_singleline(&mut self.ssid);
                    ui.end_row();
                    ui.label(t("Passwort:"));
                    ui.text_edit_singleline(&mut self.wpass);
                    ui.end_row();
                    ui.label(t("Verschlüsselung:"));
                    egui::ComboBox::from_id_salt("wenc")
                        .selected_text(match self.wenc {
                            WifiEnc::Wpa => "WPA/WPA2",
                            WifiEnc::Wep => "WEP",
                            WifiEnc::None => t("offen"),
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.wenc, WifiEnc::Wpa, "WPA/WPA2");
                            ui.selectable_value(&mut self.wenc, WifiEnc::Wep, "WEP");
                            ui.selectable_value(&mut self.wenc, WifiEnc::None, t("offen"));
                        });
                    ui.end_row();
                    ui.label(t("Verstecktes Netz:"));
                    ui.checkbox(&mut self.whidden, "");
                    ui.end_row();
                });
            }
            Kind::Email => {
                grid(ui, "email", |ui| {
                    ui.label(t("An:"));
                    ui.text_edit_singleline(&mut self.eto);
                    ui.end_row();
                    ui.label(t("Betreff:"));
                    ui.text_edit_singleline(&mut self.esubj);
                    ui.end_row();
                });
                ui.label(t("Text:"));
                ui.add(
                    egui::TextEdit::multiline(&mut self.ebody)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                );
            }
            Kind::Contact => {
                grid(ui, "contact", |ui| {
                    ui.label(t("Name:"));
                    ui.text_edit_singleline(&mut self.cname);
                    ui.end_row();
                    ui.label(t("Firma:"));
                    ui.text_edit_singleline(&mut self.corg);
                    ui.end_row();
                    ui.label(t("Telefon:"));
                    ui.text_edit_singleline(&mut self.cphone);
                    ui.end_row();
                    ui.label(t("E-Mail:"));
                    ui.text_edit_singleline(&mut self.cemail);
                    ui.end_row();
                });
            }
            Kind::Swiss => self.swiss_fields(ui),
        }
    }

    fn swiss_fields(&mut self, ui: &mut egui::Ui) {
        let sw = &mut self.sw;
        grid(ui, "swiss_head", |ui| {
            ui.label(t("IBAN / QR-IBAN:"));
            ui.text_edit_singleline(&mut sw.iban);
            ui.end_row();
            ui.label(t("Betrag:"));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut sw.amount).desired_width(90.0));
                egui::ComboBox::from_id_salt("ccy")
                    .selected_text(if sw.currency.is_empty() { "CHF" } else { sw.currency.as_str() })
                    .width(70.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut sw.currency, "CHF".into(), "CHF");
                        ui.selectable_value(&mut sw.currency, "EUR".into(), "EUR");
                    });
            });
            ui.end_row();
        });

        ui.add_space(6.0);
        ui.label(egui::RichText::new(t("Empfänger")).strong());
        grid(ui, "swiss_cr", |ui| {
            ui.label(t("Name:"));
            ui.text_edit_singleline(&mut sw.cr_name);
            ui.end_row();
            ui.label(t("Strasse:"));
            ui.text_edit_singleline(&mut sw.cr_street);
            ui.end_row();
            ui.label(t("Nr.:"));
            ui.text_edit_singleline(&mut sw.cr_bno);
            ui.end_row();
            ui.label(t("PLZ / Ort:"));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut sw.cr_zip).desired_width(70.0));
                ui.text_edit_singleline(&mut sw.cr_city);
            });
            ui.end_row();
            ui.label(t("Land:"));
            ui.add(egui::TextEdit::singleline(&mut sw.cr_country).desired_width(50.0));
            ui.end_row();
        });

        ui.add_space(6.0);
        ui.label(egui::RichText::new(t("Zahlungspflichtiger (optional)")).strong());
        grid(ui, "swiss_db", |ui| {
            ui.label(t("Name:"));
            ui.text_edit_singleline(&mut sw.db_name);
            ui.end_row();
            ui.label(t("Strasse:"));
            ui.text_edit_singleline(&mut sw.db_street);
            ui.end_row();
            ui.label(t("Nr.:"));
            ui.text_edit_singleline(&mut sw.db_bno);
            ui.end_row();
            ui.label(t("PLZ / Ort:"));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut sw.db_zip).desired_width(70.0));
                ui.text_edit_singleline(&mut sw.db_city);
            });
            ui.end_row();
            ui.label(t("Land:"));
            ui.add(egui::TextEdit::singleline(&mut sw.db_country).desired_width(50.0));
            ui.end_row();
        });

        ui.add_space(6.0);
        ui.label(egui::RichText::new(t("Referenz & Mitteilung")).strong());
        grid(ui, "swiss_ref", |ui| {
            ui.label(t("Referenztyp:"));
            egui::ComboBox::from_id_salt("rtype")
                .selected_text(match sw.ref_type.as_str() {
                    "QRR" => t("QR-Referenz (QRR)"),
                    "SCOR" => "Creditor Reference (SCOR)",
                    _ => t("keine (NON)"),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut sw.ref_type, "NON".into(), t("keine (NON)"));
                    ui.selectable_value(&mut sw.ref_type, "QRR".into(), t("QR-Referenz (QRR)"));
                    ui.selectable_value(&mut sw.ref_type, "SCOR".into(), "Creditor Reference (SCOR)");
                });
            ui.end_row();
            if sw.ref_type != "NON" {
                ui.label(t("Referenz:"));
                ui.text_edit_singleline(&mut sw.reference);
                ui.end_row();
            }
            ui.label(t("Mitteilung:"));
            ui.text_edit_singleline(&mut sw.message);
            ui.end_row();
        });
    }
}

fn grid(ui: &mut egui::Ui, id: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([10.0, 8.0])
        .show(ui, add);
}
