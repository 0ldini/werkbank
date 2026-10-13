//! Oberfläche (egui): links Inhalt und Darstellung wählen, rechts Vorschau
//! und Export (PNG/SVG).

use crate::qr::{self, Content, Ec, Style, WifiEnc};
use crate::save::save_bytes;

#[derive(PartialEq, Clone, Copy)]
enum Kind {
    Text,
    Wifi,
    Email,
    Contact,
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
            st: Style::default(),
            last_key: String::new(),
            tex: None,
            err: None,
        }
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // etwas grössere Standardschrift
        let mut style = (*cc.egui_ctx.style()).clone();
        for (_, f) in style.text_styles.iter_mut() {
            f.size *= 1.05;
        }
        cc.egui_ctx.set_style(style);
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
        }
    }

    fn key(&self, payload: &str) -> String {
        format!(
            "{payload}|{:?}{:?}{}{}{}",
            self.st.fg, self.st.bg, self.st.scale, self.st.quiet, self.st.ec as u8
        )
    }

    fn refresh(&mut self, ctx: &egui::Context, payload: &str) {
        let key = self.key(payload);
        if key == self.last_key && (self.tex.is_some() || self.err.is_some()) {
            return;
        }
        self.last_key = key;
        match qr::to_rgba(payload, &self.st) {
            Ok((raw, size)) => {
                let img = egui::ColorImage::from_rgba_unmultiplied([size as usize, size as usize], &raw);
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
        self.refresh(ctx, &payload);

        egui::SidePanel::left("eingabe")
            .resizable(false)
            .min_width(320.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.heading("Codewerk");
                ui.label(egui::RichText::new("QR-Codes erstellen").weak());
                ui.add_space(10.0);

                ui.label("Inhalt");
                egui::ComboBox::from_id_salt("kind")
                    .selected_text(match self.kind {
                        Kind::Text => "Text / Link",
                        Kind::Wifi => "WLAN-Zugang",
                        Kind::Email => "E-Mail",
                        Kind::Contact => "Kontakt (vCard)",
                    })
                    .width(280.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.kind, Kind::Text, "Text / Link");
                        ui.selectable_value(&mut self.kind, Kind::Wifi, "WLAN-Zugang");
                        ui.selectable_value(&mut self.kind, Kind::Email, "E-Mail");
                        ui.selectable_value(&mut self.kind, Kind::Contact, "Kontakt (vCard)");
                    });
                ui.add_space(8.0);

                match self.kind {
                    Kind::Text => {
                        ui.label("Text oder Link:");
                        ui.add(
                            egui::TextEdit::multiline(&mut self.text)
                                .desired_rows(4)
                                .desired_width(f32::INFINITY),
                        );
                    }
                    Kind::Wifi => {
                        grid(ui, "wifi", |ui| {
                            ui.label("Netzname (SSID):");
                            ui.text_edit_singleline(&mut self.ssid);
                            ui.end_row();
                            ui.label("Passwort:");
                            ui.text_edit_singleline(&mut self.wpass);
                            ui.end_row();
                            ui.label("Verschlüsselung:");
                            egui::ComboBox::from_id_salt("wenc")
                                .selected_text(match self.wenc {
                                    WifiEnc::Wpa => "WPA/WPA2",
                                    WifiEnc::Wep => "WEP",
                                    WifiEnc::None => "offen",
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut self.wenc, WifiEnc::Wpa, "WPA/WPA2");
                                    ui.selectable_value(&mut self.wenc, WifiEnc::Wep, "WEP");
                                    ui.selectable_value(&mut self.wenc, WifiEnc::None, "offen");
                                });
                            ui.end_row();
                            ui.label("Verstecktes Netz:");
                            ui.checkbox(&mut self.whidden, "");
                            ui.end_row();
                        });
                    }
                    Kind::Email => {
                        grid(ui, "email", |ui| {
                            ui.label("An:");
                            ui.text_edit_singleline(&mut self.eto);
                            ui.end_row();
                            ui.label("Betreff:");
                            ui.text_edit_singleline(&mut self.esubj);
                            ui.end_row();
                        });
                        ui.label("Text:");
                        ui.add(
                            egui::TextEdit::multiline(&mut self.ebody)
                                .desired_rows(3)
                                .desired_width(f32::INFINITY),
                        );
                    }
                    Kind::Contact => {
                        grid(ui, "contact", |ui| {
                            ui.label("Name:");
                            ui.text_edit_singleline(&mut self.cname);
                            ui.end_row();
                            ui.label("Firma:");
                            ui.text_edit_singleline(&mut self.corg);
                            ui.end_row();
                            ui.label("Telefon:");
                            ui.text_edit_singleline(&mut self.cphone);
                            ui.end_row();
                            ui.label("E-Mail:");
                            ui.text_edit_singleline(&mut self.cemail);
                            ui.end_row();
                        });
                    }
                }

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(6.0);
                ui.label("Darstellung");
                grid(ui, "style", |ui| {
                    ui.label("Farbe:");
                    ui.horizontal(|ui| {
                        ui.color_edit_button_srgb(&mut self.st.fg);
                        ui.label("Hintergrund:");
                        ui.color_edit_button_srgb(&mut self.st.bg);
                    });
                    ui.end_row();
                    ui.label("Grösse:");
                    ui.add(egui::Slider::new(&mut self.st.scale, 2..=30).suffix(" px/Modul"));
                    ui.end_row();
                    ui.label("Rand:");
                    ui.add(egui::Slider::new(&mut self.st.quiet, 0..=10).suffix(" Module"));
                    ui.end_row();
                    ui.label("Fehlerkorrektur:");
                    egui::ComboBox::from_id_salt("ec")
                        .selected_text(self.st.ec.label())
                        .show_ui(ui, |ui| {
                            for ec in [Ec::L, Ec::M, Ec::Q, Ec::H] {
                                ui.selectable_value(&mut self.st.ec, ec, ec.label());
                            }
                        });
                    ui.end_row();
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(10.0);
            ui.vertical_centered(|ui| {
                if let Some(err) = &self.err {
                    ui.add_space(40.0);
                    ui.colored_label(egui::Color32::from_rgb(0xc0, 0x39, 0x2b), err);
                } else if let Some(tex) = &self.tex {
                    let avail = ui.available_width().min(ui.available_height() - 90.0);
                    let d = avail.clamp(120.0, 440.0);
                    ui.image(egui::load::SizedTexture::new(tex.id(), egui::vec2(d, d)));
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.add_space((ui.available_width() - 260.0).max(0.0) / 2.0);
                        if ui.add_sized([120.0, 34.0], egui::Button::new("Als PNG speichern")).clicked() {
                            if let Ok(bytes) = qr::to_png(&payload, &self.st) {
                                save_bytes("qr-code.png", &bytes);
                            }
                        }
                        ui.add_space(8.0);
                        if ui.add_sized([120.0, 34.0], egui::Button::new("Als SVG speichern")).clicked() {
                            if let Ok(s) = qr::to_svg(&payload, &self.st) {
                                save_bytes("qr-code.svg", s.as_bytes());
                            }
                        }
                    });
                }
            });
        });
    }
}

fn grid(ui: &mut egui::Ui, id: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([10.0, 8.0])
        .show(ui, add);
}
