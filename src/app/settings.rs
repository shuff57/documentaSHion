//! The Settings screen: a section menu on the left and one card per section. Changes save as
//! they are made.
use super::*;
use crate::ui::kit::{Tier, ty};
use egui::{Align2, Rect, Sense};

const SECTIONS: [(&str, &str); 5] = [
    (icon::USB, "Scanner"),
    (icon::SLIDERS_HORIZONTAL, "Cleanup & OCR"),
    (icon::SPARKLE, "AI provider"),
    (icon::SUN, "Appearance"),
    (icon::DOWNLOAD_SIMPLE, "Updates"),
];
const NAV_W: f32 = 220.0;

impl App {
    pub(super) fn settings_screen(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let busy = self.job.is_some();
        let before = self.settings.clone();
        ui.label(kit::meta("Saved as you go"));
        ui.label(kit::heading("Settings", ty::TITLE));
        ui.add_space(14.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 24.0;
            ui.vertical(|ui| {
                ui.set_width(NAV_W);
                ui.spacing_mut().item_spacing.y = 4.0;
                for (i, (glyph, label)) in SECTIONS.iter().enumerate() {
                    if nav_item(ui, glyph, label, self.settings_section == i) {
                        self.settings_section = i;
                    }
                }
            });
            ui.vertical(|ui| {
                ui.set_width((ui.available_width()).min(680.0));
                kit::card_frame(24).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    match self.settings_section {
                        0 => self.scanner_section(ui, ctx, busy),
                        1 => self.cleanup_section(ui),
                        2 => self.ai_section(ui, ctx),
                        3 => self.appearance_section(ui, ctx),
                        _ => self.updates_section(ui, ctx, busy),
                    }
                });
            });
        });
        if self.settings != before {
            if let Err(e) = self.settings.save() {
                self.status = e;
            }
        }
    }

    fn scanner_section(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, busy: bool) {
        ui.label(kit::heading("Scanner", ty::SECTION));
        ui.label(kit::body("documentaSHion talks to the iX500 over USB through SANE's Fujitsu driver."));
        ui.add_space(10.0);
        kit::label(ui, "Device");
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!busy, |ui| {
                if kit::secondary(ui, Some(icon::USB), "Find scanners").clicked() {
                    self.launch(ctx, move |c| Event::Devices(services::discover(&c)));
                }
            });
            let shown = if self.settings.device.is_empty() { "None chosen".to_string() } else { self.settings.device.clone() };
            egui::ComboBox::from_id_salt("device")
                .selected_text(shown)
                .width(300.0)
                .show_ui(ui, |ui| {
                    for d in &self.devices {
                        ui.selectable_value(&mut self.settings.device, d.clone(), d);
                    }
                    if self.devices.is_empty() {
                        ui.label(kit::meta("Press Find scanners first."));
                    }
                });
        });
        if self.devices.is_empty() && self.settings.device.is_empty() {
            kit::note(ui, icon::INFO, "Plug in the iX500, open its lid so it powers on, then press Find scanners.", theme::accent());
        }
        ui.add_space(12.0);
        kit::label(ui, "Feeding");
        kit::switch(ui, &mut self.settings.duplex, "Scan both sides of each sheet");
        ui.label(kit::meta("Blank backs are spotted and can be removed in one step."));
        ui.add_space(12.0);
        kit::label(ui, "Resolution");
        let mut dpi = self.settings.dpi;
        kit::segmented(ui, &mut dpi, &[(150, "150"), (200, "200"), (300, "300 dpi"), (400, "400"), (600, "600")]);
        self.settings.dpi = dpi;
        ui.label(kit::meta("300 dpi suits text and handwriting. Higher takes longer and makes bigger files."));
    }

    fn cleanup_section(&mut self, ui: &mut egui::Ui) {
        ui.label(kit::heading("Cleanup & OCR", ty::SECTION));
        ui.add_space(6.0);
        kit::label(ui, "New pages");
        kit::switch(ui, &mut self.settings.auto_cleanup, "Clean up new pages automatically");
        ui.label(kit::meta(
            "Straightens tilted pages, trims dark scanner edges, raises contrast and spots blank pages. The original scan is kept, and each step can be switched off or undone per page.",
        ));
        ui.add_space(14.0);
        kit::label(ui, "Reading text on this computer");
        match &self.ocr_status {
            Ok(v) => {
                ui.horizontal(|ui| {
                    kit::glyph(ui.painter(), ui.cursor().min + Vec2::new(10.0, 12.0), icon::CHECK, 18.0, theme::accent());
                    ui.add_space(24.0);
                    ui.label(kit::body(format!("Ready · {v}")).color(theme::ink()));
                });
            }
            Err(e) => kit::note(ui, icon::WARNING, e, theme::amber()),
        }
        if kit::button(ui, Tier::Tertiary, None, "Check again", 30.0).clicked() {
            self.ocr_status = services::ocr_status();
        }
    }

    fn ai_section(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(kit::heading("AI provider", ty::SECTION));
        ui.label(kit::body("Used for AI correction and rewriting. The same providers as verSHion."));
        ui.add_space(8.0);
        kit::label(ui, "Provider");
        let before = self.settings.provider.clone();
        let mut provider = before.as_str();
        kit::segmented(
            ui,
            &mut provider,
            &[("Local Ollama", "Ollama"), ("Ollama Cloud", "Ollama Cloud"), ("OpenRouter", "OpenRouter"), ("Custom", "Custom")],
        );
        if provider != before {
            self.settings.provider = provider.to_string();
            self.settings.endpoint = match provider {
                "Local Ollama" => "http://127.0.0.1:11434",
                "Ollama Cloud" => "https://ollama.com/v1",
                "OpenRouter" => "https://openrouter.ai/api/v1",
                _ => "http://localhost:1234/v1",
            }
            .into();
            self.settings.model.clear();
            self.load_key(ctx);
        }
        ui.add_space(8.0);
        kit::field(ui, "Endpoint", &mut self.settings.endpoint, "https://…/v1", false);
        self.model_picker(ui, ctx);
        if self.settings.needs_key() {
            ui.add_space(8.0);
            if kit::field(ui, "API key", &mut self.key, "Paste your key", true).changed() {
                self.key_state = KeyState::Typed;
            }
            ui.horizontal(|ui| {
                ui.add_enabled_ui(self.key_job.is_none(), |ui| {
                    if ui.add_enabled_ui(!self.key.trim().is_empty(), |ui| kit::secondary(ui, Some(icon::LOCK_SIMPLE), "Save to keychain")).inner.clicked() {
                        let key = self.key.trim().to_owned();
                        self.keychain(ctx, move |a| secrets::store(a, &key).map(|_| KeyChange::Stored));
                    }
                    if kit::tertiary(ui, Some(icon::TRASH), "Forget saved key").clicked() {
                        self.keychain(ctx, |a| secrets::forget(a).map(|_| KeyChange::Forgotten));
                    }
                });
            });
            let (glyph, tone, text) = match &self.key_state {
                KeyState::Saved => (icon::LOCK_SIMPLE, theme::accent(), "Saved in the system keychain (KWallet). Loaded each time the app starts.".to_string()),
                KeyState::Typed => (icon::KEY, theme::amber(), "Used for this session only until you save it to the keychain.".to_string()),
                KeyState::None => (icon::KEY, theme::muted(), "No key saved for this provider. Environment keys also work: OLLAMA_API_KEY, OPENROUTER_API_KEY, DOCUMENTASHION_AI_KEY.".to_string()),
                KeyState::Unavailable(e) => (icon::WARNING, theme::amber(), e.clone()),
            };
            kit::note(ui, glyph, &text, tone);
        }
        ui.add_space(6.0);
        ui.label(kit::meta("AI actions send the selected page image and its text to this provider. Reading text locally never leaves the computer."));
    }

    /// A dropdown of the provider's vision models, with a refresh button and a typed fallback.
    fn model_picker(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let tag = format!("{}|{}", self.settings.provider, self.settings.endpoint);
        let fetched = matches!(&self.models, Some((t, _)) if *t == tag);
        kit::label(ui, "Vision model");
        ui.horizontal(|ui| {
            let shown = if self.settings.model.is_empty() { "Choose a model".to_string() } else { self.settings.model.clone() };
            let list: Vec<String> = match &self.models {
                Some((t, Ok(m))) if *t == tag => m.list.clone(),
                _ => vec![],
            };
            let r = egui::ComboBox::from_id_salt("model").selected_text(shown).width(340.0).show_ui(ui, |ui| {
                for m in &list {
                    ui.selectable_value(&mut self.settings.model, m.clone(), m);
                }
                if list.is_empty() {
                    ui.label(kit::meta(if self.model_job.is_some() { "Loading…" } else { "Press the refresh button to list models." }));
                }
            });
            // Opening the list the first time asks the provider for it.
            if r.response.clicked() && !fetched {
                self.fetch_models(ctx);
            }
            ui.add_enabled_ui(self.model_job.is_none(), |ui| {
                if kit::icon_button(ui, icon::ARROW_CLOCKWISE, 36.0, "Refresh the model list", false).clicked() {
                    self.fetch_models(ctx);
                }
            });
        });
        match &self.models {
            Some((t, Err(e))) if *t == tag => kit::note(ui, icon::WARNING, e, theme::amber()),
            Some((t, Ok(m))) if *t == tag => {
                let text = if m.vision {
                    format!("{} models that accept images.", m.list.len())
                } else {
                    format!("{} models. This provider doesn't say which accept images, so pick a vision one.", m.list.len())
                };
                ui.label(kit::meta(text));
            }
            _ => {}
        }
        kit::field(ui, "Or type a model name", &mut self.settings.model, "for example qwen2.5vl or gemma3", false);
    }

    fn updates_section(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, busy: bool) {
        ui.label(kit::heading("Updates", ty::SECTION));
        ui.label(kit::body("Pulls the latest version from GitHub, rebuilds it and restarts the app."));
        ui.add_space(8.0);
        let built: String = update::BUILT.chars().take(7).collect();
        ui.label(kit::meta(format!("Running version {built} · source {}", update::source().display())));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!busy, |ui| {
                if kit::secondary(ui, Some(icon::ARROW_CLOCKWISE), "Check for updates").clicked() {
                    self.launch(ctx, |c| Event::Update(update::check(&c)));
                }
                if matches!(&self.update, Some(Ok(s)) if s.behind)
                    && kit::primary(ui, Some(icon::DOWNLOAD_SIMPLE), "Update and restart").clicked()
                {
                    self.save();
                    self.launch(ctx, |c| Event::Updated(update::apply(&c)));
                }
            });
        });
        match &self.update {
            Some(Ok(s)) if s.behind => {
                kit::note(ui, icon::DOWNLOAD_SIMPLE, "An update is available. Building takes a few minutes; the app restarts when it finishes.", theme::accent());
                for c in s.changes.iter().take(8) {
                    ui.label(kit::body(format!("•  {c}")));
                }
            }
            Some(Ok(_)) => kit::note(ui, icon::CHECK, "You have the latest version.", theme::accent()),
            Some(Err(e)) => kit::note(ui, icon::WARNING, e, theme::amber()),
            None => {}
        }
        ui.add_space(6.0);
        ui.label(kit::meta("Needs git and Rust (cargo) on this computer. Your projects and settings are kept."));
    }

    fn appearance_section(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(kit::heading("Appearance", ty::SECTION));
        ui.add_space(6.0);
        kit::label(ui, "Theme");
        let mut dark = self.settings.dark;
        if kit::segmented(ui, &mut dark, &[(false, "Light"), (true, "Dark")]) {
            self.settings.dark = dark;
            theme::set_dark(dark, ctx);
            self.proposal.iter_mut().for_each(|p| p.view = None);
        }
        ui.label(kit::meta("The dock's moon and sun button switches too."));
    }
}

/// One section in the menu: the chosen one wears the accent wash and a bar on its left.
fn nav_item(ui: &mut egui::Ui, glyph: &str, label: &str, on: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(NAV_W, 44.0), Sense::click());
    if on {
        ui.painter().rect_filled(rect, CornerRadius::same(12), theme::accent_faded());
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min + Vec2::new(0.0, 11.0), Vec2::new(3.0, 22.0)),
            CornerRadius::same(2),
            theme::accent(),
        );
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(12), theme::hover_bg());
    }
    let col = if on { theme::accent() } else { theme::ink() };
    kit::glyph(ui.painter(), rect.left_center() + Vec2::new(27.0, 0.0), glyph, 18.0, col);
    ui.painter().text(
        rect.left_center() + Vec2::new(46.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        egui::FontId::new(15.0, theme::bold()),
        col,
    );
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, on, label));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}
