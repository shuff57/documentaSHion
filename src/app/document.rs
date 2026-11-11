//! The Document screen: page cards on the left, the page on a tray in the middle with a floating
//! tool pill, and the review inspector on the right.
use super::*;
use crate::ui::kit::{Tier, ty};
use egui::{Align, Layout, Margin, Rect, RichText, Sense, UiBuilder};

/// Header and empty-state actions, run after drawing so menus don't hold borrows.
#[derive(Clone, Copy, PartialEq)]
enum Act {
    Scan,
    AddImages,
    AddPdf,
    ExportPdf,
    ExportImages,
    ExportText,
    Restore,
    RemoveBlank,
    Settings,
}

/// Page edits from the tool pill and crop card.
#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Rotate,
    Straighten,
    Contrast,
    Undo,
    Earlier,
    Later,
    Remove,
}

const PILL_H: f32 = 44.0;

impl App {
    pub(super) fn document(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let busy = self.job.is_some();
        let mut act = None;
        self.header(ui, busy, &mut act);
        ui.add_space(16.0);
        if self.project.pages.is_empty() {
            self.empty(ui, busy, &mut act);
        } else {
            let area = ui.available_rect_before_wrap();
            let gap = 16.0;
            let left_w = 188.0;
            let right_w = (area.width() * 0.3).clamp(320.0, 420.0);
            let left = Rect::from_min_size(area.min, Vec2::new(left_w, area.height()));
            let right = Rect::from_min_max(Pos2::new(area.right() - right_w, area.top()), area.max);
            let center = Rect::from_min_max(Pos2::new(left.right() + gap, area.top()), Pos2::new(right.left() - gap, area.bottom()));
            ui.scope_builder(UiBuilder::new().max_rect(left), |ui| self.page_list(ui, ctx, busy, &mut act));
            ui.scope_builder(UiBuilder::new().max_rect(center), |ui| self.tray(ui, ctx, busy));
            ui.scope_builder(UiBuilder::new().max_rect(right), |ui| self.inspector(ui, ctx, busy));
            ui.allocate_rect(area, Sense::hover());
        }
        if let Some(a) = act {
            self.run(a, ctx);
        }
    }

    fn header(&mut self, ui: &mut egui::Ui, busy: bool, act: &mut Option<Act>) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(kit::meta(format!(
                    "Document  ·  {} {}",
                    self.project.pages.len(),
                    if self.project.pages.len() == 1 { "page" } else { "pages" }
                )));
                let title = ui.add_enabled(
                    !busy,
                    egui::TextEdit::singleline(&mut self.project.name)
                        .frame(egui::Frame::new())
                        .margin(Margin::ZERO)
                        .font(egui::FontId::new(ty::TITLE, theme::serif()))
                        .text_color(theme::ink())
                        .hint_text("Untitled scan")
                        .desired_width(560.0),
                );
                if title.changed() {
                    self.save();
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.add_enabled_ui(!busy, |ui| {
                    if kit::primary(ui, Some(icon::SCAN), "Scan").clicked() {
                        *act = Some(Act::Scan);
                    }
                    let has_pages = !self.project.pages.is_empty();
                    ui.add_enabled_ui(has_pages, |ui| {
                        kit::menu_button(ui, Tier::Secondary, Some(icon::EXPORT), "Export", |ui| {
                            if kit::menu_item(ui, icon::FILE_PDF, "Searchable PDF…").clicked() {
                                *act = Some(Act::ExportPdf);
                            }
                            if kit::menu_item(ui, icon::IMAGES, "Page images…").clicked() {
                                *act = Some(Act::ExportImages);
                            }
                            if kit::menu_item(ui, icon::FILE_TEXT, "Text files…").clicked() {
                                *act = Some(Act::ExportText);
                            }
                        });
                    });
                    kit::menu_button(ui, Tier::Tertiary, Some(icon::PLUS), "Add pages", |ui| {
                        if kit::menu_item(ui, icon::IMAGES, "Images…").clicked() {
                            *act = Some(Act::AddImages);
                        }
                        if kit::menu_item(ui, icon::FILE_PDF, "PDF…").clicked() {
                            *act = Some(Act::AddPdf);
                        }
                    });
                });
            });
        });
    }

    fn empty(&mut self, ui: &mut egui::Ui, busy: bool, act: &mut Option<Act>) {
        ui.add_space((ui.available_height() * 0.12).min(90.0));
        let w = ui.available_width().min(620.0);
        let pad = (ui.available_width() - w) / 2.0;
        ui.horizontal(|ui| {
            ui.add_space(pad);
            ui.vertical(|ui| {
                ui.set_width(w);
                ui.add_enabled_ui(!busy, |ui| {
                    match kit::empty_state(
                        ui,
                        icon::SCAN,
                        "Paper, with possibilities.",
                        "Scan an exam, a worksheet or handwritten notes, or add pages you already have. Originals stay untouched, and AI suggestions wait for your review.",
                        &[
                            (Some(icon::SCAN), "Scan", Tier::Primary),
                            (Some(icon::IMAGES), "Add images", Tier::Secondary),
                            (Some(icon::FILE_PDF), "Add a PDF", Tier::Secondary),
                        ],
                    ) {
                        Some(0) => *act = Some(Act::Scan),
                        Some(1) => *act = Some(Act::AddImages),
                        Some(2) => *act = Some(Act::AddPdf),
                        _ => {}
                    }
                    ui.add_space(12.0);
                    if self.settings.device.trim().is_empty() {
                        kit::note(ui, icon::USB, "No scanner chosen yet. Plug in the iX500 by USB, then pick it in Settings.", theme::accent());
                        if kit::button(ui, Tier::Tertiary, Some(icon::GEAR), "Open scanner settings", 32.0).clicked() {
                            *act = Some(Act::Settings);
                        }
                    }
                    if !self.removed.is_empty() && kit::tertiary(ui, Some(icon::ARROW_U_UP_LEFT), "Restore removed pages").clicked() {
                        *act = Some(Act::Restore);
                    }
                });
            });
        });
    }

    fn run(&mut self, a: Act, ctx: &egui::Context) {
        match a {
            Act::Scan => {
                if self.settings.device.trim().is_empty() {
                    self.status = "Choose your scanner in Settings first.".into();
                    self.screen = Screen::Settings;
                    self.settings_section = 0;
                    return;
                }
                let p = self.project.clone();
                let s = self.settings.clone();
                self.add_pages(ctx, s.dpi, move |c| services::scan(&p, &s, c));
            }
            Act::AddImages => {
                if let Some(paths) = rfd::FileDialog::new()
                    .add_filter("Images", &["png", "jpg", "jpeg", "tif", "tiff", "pnm"])
                    .pick_files()
                {
                    self.add_pages(ctx, 300, move |_| Ok((paths, None)));
                }
            }
            Act::AddPdf => {
                if let Some(path) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_file() {
                    self.add_pages(ctx, 300, move |c| services::import_pdf(&path, c).map(|p| (p, None)));
                }
            }
            Act::ExportPdf => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name(format!("{}.pdf", file_stem(&self.project.name)))
                    .save_file()
                {
                    self.status = match crate::pdf::export(&self.project, &path) {
                        Ok(()) => "PDF exported · its text is searchable on read pages".into(),
                        Err(e) => e,
                    };
                }
            }
            Act::ExportImages => {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    let r = self.project.pages.iter().enumerate().try_for_each(|(i, p)| {
                        p.render()?
                            .save(dir.join(format!("page-{:04}.png", i + 1)))
                            .map_err(|e| e.to_string())
                    });
                    self.status = match r {
                        Ok(()) => "Page images exported".into(),
                        Err(e) => e,
                    };
                }
            }
            Act::ExportText => {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    let r = self.project.pages.iter().enumerate().try_for_each(|(i, p)| -> Result<(), String> {
                        std::fs::write(dir.join(format!("page-{:04}-transcription.txt", i + 1)), &p.text)
                            .map_err(|e| e.to_string())?;
                        if !p.rewrite.is_empty() {
                            std::fs::write(dir.join(format!("page-{:04}-rewrite.txt", i + 1)), &p.rewrite)
                                .map_err(|e| e.to_string())?;
                        }
                        Ok(())
                    });
                    self.status = match r {
                        Ok(()) => "Transcriptions and rewrites exported".into(),
                        Err(e) => e,
                    };
                }
            }
            Act::Restore => self.restore(),
            Act::RemoveBlank => {
                let blank: Vec<usize> = (0..self.project.pages.len()).filter(|&i| self.project.pages[i].blank).collect();
                self.removed = blank.iter().rev().map(|&i| (i, self.project.pages.remove(i))).collect();
                self.removed.reverse();
                self.selected = self.selected.min(self.project.pages.len().saturating_sub(1));
                self.invalidate();
                self.save();
                self.status = format!("Removed {} blank pages · Restore brings them back", blank.len());
            }
            Act::Settings => {
                self.screen = Screen::Settings;
                self.settings_section = 0;
            }
        }
    }

    fn page_list(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, busy: bool, act: &mut Option<Act>) {
        let h = ui.available_height();
        kit::card_frame(12).show(ui, |ui| {
            ui.set_min_height(h - 26.0);
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add_space(2.0);
                kit::label(ui, "Pages");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    kit::quiet_pill(ui, &self.project.pages.len().to_string());
                });
            });
            let blank = self.project.pages.iter().filter(|p| p.blank).count();
            ui.add_enabled_ui(!busy, |ui| {
                if blank > 0 {
                    kit::note(ui, icon::FILE_DASHED, &format!("{blank} {} blank", if blank == 1 { "page looks" } else { "pages look" }), theme::amber());
                    if kit::button(ui, Tier::Secondary, Some(icon::TRASH), "Remove blank", 30.0).clicked() {
                        *act = Some(Act::RemoveBlank);
                    }
                }
                if !self.removed.is_empty() && kit::button(ui, Tier::Tertiary, Some(icon::ARROW_U_UP_LEFT), "Restore removed", 30.0).clicked() {
                    *act = Some(Act::Restore);
                }
            });
            ui.add_space(4.0);
            let item_w = ui.available_width();
            let thumb_w = item_w - 20.0;
            let item_h = thumb_w * 1.3 + 30.0;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, item_h, self.project.pages.len(), |ui, range| {
                for i in range {
                    let p = &self.project.pages[i];
                    let key = format!("thumb-{}", p.render_key());
                    if !self.thumbs.contains_key(&key) {
                        if let Ok(img) = p.render_within(Some(420)) {
                            let t = texture(ctx, &key, img.thumbnail(260, 340));
                            self.thumbs.insert(key.clone(), t);
                        }
                    }
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(item_w, item_h), if busy { Sense::hover() } else { Sense::click() });
                    let on = self.selected == i;
                    if on {
                        ui.painter().rect_filled(rect, CornerRadius::same(12), theme::accent_faded());
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, CornerRadius::same(12), theme::hover_bg());
                    }
                    let slot = Rect::from_min_size(rect.min + Vec2::new(10.0, 8.0), Vec2::new(thumb_w, thumb_w * 1.3 - 6.0));
                    if let Some(t) = self.thumbs.get(&key) {
                        let s = t.size_vec2();
                        let scale = (slot.width() / s.x).min(slot.height() / s.y);
                        let r = Rect::from_center_size(slot.center(), s * scale);
                        ui.painter().add(paper_shadow(r, 4));
                        ui.painter().image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                        ui.painter().rect_stroke(
                            r,
                            CornerRadius::same(2),
                            if on { Stroke::new(2.0, theme::accent()) } else { Stroke::new(1.0, theme::border()) },
                            StrokeKind::Outside,
                        );
                    }
                    let base = rect.bottom() - 13.0;
                    ui.painter().text(
                        Pos2::new(rect.left() + 12.0, base),
                        egui::Align2::LEFT_CENTER,
                        format!("{}", i + 1),
                        egui::FontId::new(ty::META, theme::bold()),
                        if on { theme::accent() } else { theme::ink() },
                    );
                    let (state, fill, fg) = if p.blank {
                        ("Blank?", theme::highlighter(), theme::on_highlighter())
                    } else if p.text.is_empty() {
                        ("Not read", theme::field(), theme::muted())
                    } else {
                        ("Read", theme::accent_faded(), theme::accent())
                    };
                    paint_pill(ui, Pos2::new(rect.right() - 10.0, base), state, fill, fg);
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
                        self.selected = i;
                        self.texture = None;
                        self.proposal = None;
                    }
                }
            });
        });
    }

    fn tray(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, busy: bool) {
        let tray = ui.max_rect();
        ui.painter().rect_filled(tray, CornerRadius::same(16), theme::table());
        let mut tool = None;
        let mut redraw = true;
        let mut changed = false;
        // The floating tool pill.
        let pill_w: f32 = 9.0 * 38.0 + 3.0 * 9.0 + 116.0 + 20.0;
        let pill = Rect::from_center_size(Pos2::new(tray.center().x, tray.top() + 16.0 + PILL_H / 2.0), Vec2::new(pill_w.min(tray.width() - 24.0), PILL_H));
        let page_area = Rect::from_min_max(Pos2::new(tray.left() + 20.0, pill.bottom() + 16.0), Pos2::new(tray.right() - 20.0, tray.bottom() - 46.0));

        // The page itself.
        let p = &self.project.pages[self.selected];
        let key = p.render_key();
        if self.texture.as_ref().is_none_or(|(k, _)| k != &key) {
            match p.render_within(Some(2200)) {
                Ok(img) => self.texture = Some((key.clone(), texture(ctx, &key, img))),
                Err(e) => self.status = e,
            }
        }
        if let Some((_, tex)) = &self.texture {
            let s = tex.size_vec2();
            let fit = (page_area.width() / s.x).min(page_area.height() / s.y);
            let size = s * fit * self.zoom;
            ui.scope_builder(UiBuilder::new().max_rect(page_area), |ui| {
                egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                    let avail = ui.available_size();
                    let (outer, _) = ui.allocate_exact_size(size.max(avail), Sense::hover());
                    let r = Rect::from_center_size(outer.center(), size);
                    ui.painter().add(paper_shadow(r, 10));
                    ui.painter().image(tex.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                });
            });
        }

        // Page counter and zoom along the foot of the tray.
        let foot = Pos2::new(tray.center().x, tray.bottom() - 22.0);
        let counter = format!("Page {} of {}", self.selected + 1, self.project.pages.len());
        let w = ui.painter().layout_no_wrap(counter.clone(), egui::FontId::new(11.0, theme::bold()), theme::text()).size().x + 16.0;
        paint_pill(ui, foot + Vec2::new(w / 2.0, 0.0), &counter, theme::card(), theme::text());

        ui.scope_builder(UiBuilder::new().max_rect(pill), |ui| {
            kit::card_frame(0)
                .corner_radius(CornerRadius::same(14))
                .inner_margin(Margin::symmetric(6, 4))
                .show(ui, |ui| {
                    ui.add_enabled_ui(!busy, |ui| {
                        ui.horizontal_centered(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            let p = &self.project.pages[self.selected];
                            let side = 36.0;
                            if kit::icon_button(ui, icon::ARROW_CLOCKWISE, side, "Rotate a quarter turn", false).clicked() {
                                tool = Some(Tool::Rotate);
                            }
                            let tip = if p.skew == 0.0 {
                                "Straighten · this page is already level".to_string()
                            } else if p.straighten {
                                format!("Straightened by {:.1}° · click to undo", p.skew.abs())
                            } else {
                                format!("Straighten by {:.1}°", p.skew.abs())
                            };
                            if ui.add_enabled_ui(p.skew != 0.0, |ui| kit::icon_button(ui, icon::ANGLE, side, &tip, p.straighten)).inner.clicked() {
                                tool = Some(Tool::Straighten);
                            }
                            let cropped = p.crop != [0.0; 4];
                            if kit::icon_button(ui, icon::CROP, side, if cropped { "Crop · edges trimmed" } else { "Crop" }, self.crop_open || cropped).clicked() {
                                self.crop_open = !self.crop_open;
                            }
                            if kit::icon_button(ui, icon::CIRCLE_HALF, side, if p.clean { "Contrast cleanup · on" } else { "Contrast cleanup · off" }, p.clean).clicked() {
                                tool = Some(Tool::Contrast);
                            }
                            kit::tool_divider(ui, 36.0);
                            if ui.add_enabled_ui(!p.history.is_empty(), |ui| kit::icon_button(ui, icon::ARROW_U_UP_LEFT, side, "Undo the last change to this page", false)).inner.clicked() {
                                tool = Some(Tool::Undo);
                            }
                            kit::tool_divider(ui, 36.0);
                            if ui.add_enabled_ui(self.selected > 0, |ui| kit::icon_button(ui, icon::ARROW_LEFT, side, "Move page earlier", false)).inner.clicked() {
                                tool = Some(Tool::Earlier);
                            }
                            if ui.add_enabled_ui(self.selected + 1 < self.project.pages.len(), |ui| kit::icon_button(ui, icon::ARROW_RIGHT, side, "Move page later", false)).inner.clicked() {
                                tool = Some(Tool::Later);
                            }
                            if kit::icon_button(ui, icon::TRASH, side, "Remove page · Restore brings it back", false).clicked() {
                                tool = Some(Tool::Remove);
                            }
                            kit::tool_divider(ui, 36.0);
                            if kit::icon_button(ui, icon::MAGNIFYING_GLASS_MINUS, side, "Zoom out", false).clicked() {
                                self.zoom = (self.zoom / 1.25).max(0.5);
                            }
                            ui.add_sized([44.0, 36.0], egui::Label::new(RichText::new(format!("{:.0}%", self.zoom * 100.0)).family(theme::semibold()).size(ty::META).color(theme::text())));
                            if kit::icon_button(ui, icon::MAGNIFYING_GLASS_PLUS, side, "Zoom in", false).clicked() {
                                self.zoom = (self.zoom * 1.25).min(4.0);
                            }
                            if kit::icon_button(ui, icon::CORNERS_OUT, side, "Fit the page", self.zoom == 1.0).clicked() {
                                self.zoom = 1.0;
                            }
                        });
                    });
                });
        });

        if self.crop_open {
            let card = Rect::from_min_size(Pos2::new(pill.left(), pill.bottom() + 10.0), Vec2::new(300.0, 250.0));
            ui.scope_builder(UiBuilder::new().max_rect(card), |ui| {
                kit::card_frame(16).show(ui, |ui| {
                    ui.set_width(card.width() - 34.0);
                    ui.horizontal(|ui| {
                        kit::label(ui, "Crop");
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if kit::icon_button(ui, icon::X, 26.0, "Close", false).clicked() {
                                self.crop_open = false;
                            }
                        });
                    });
                    ui.label(kit::meta("Dark scanner edges are trimmed for you. Trim more here."));
                    ui.add_enabled_ui(!busy, |ui| {
                        let p = &mut self.project.pages[self.selected];
                        for (side, name) in ["Left", "Top", "Right", "Bottom"].iter().enumerate() {
                            let i = p.crop_index(side);
                            let before = p.before_edit();
                            let mut v = p.crop[i] * 100.0;
                            let r = kit::slider_row(ui, name, &mut v, 0.0..=40.0);
                            if r.drag_started() || (r.changed() && !r.dragged()) {
                                p.push_history(before);
                            }
                            if r.changed() {
                                p.crop[i] = v / 100.0;
                                changed = true;
                                redraw &= !r.dragged();
                            }
                            if r.drag_stopped() {
                                changed = true;
                            }
                        }
                        if ui.add_enabled_ui(p.crop != [0.0; 4], |ui| kit::button(ui, Tier::Tertiary, None, "No crop", 30.0)).inner.clicked() {
                            p.checkpoint();
                            p.crop = [0.0; 4];
                            changed = true;
                        }
                    });
                });
            });
        }

        if let Some(t) = tool {
            let n = self.selected;
            let p = &mut self.project.pages[n];
            match t {
                Tool::Rotate => {
                    p.checkpoint();
                    p.rotation = (p.rotation + 1) % 4;
                }
                Tool::Straighten => {
                    p.checkpoint();
                    p.straighten = !p.straighten;
                }
                Tool::Contrast => {
                    p.checkpoint();
                    p.clean = !p.clean;
                }
                Tool::Undo => p.undo(),
                Tool::Earlier => {
                    self.project.pages.swap(n, n - 1);
                    self.selected -= 1;
                }
                Tool::Later => {
                    self.project.pages.swap(n, n + 1);
                    self.selected += 1;
                }
                Tool::Remove => {
                    self.removed = vec![(n, self.project.pages.remove(n))];
                    self.selected = n.saturating_sub(1);
                    self.status = "Page removed · Restore removed brings it back".into();
                }
            }
            changed = true;
        }
        if changed && redraw {
            self.invalidate();
            self.save();
        }
    }

    fn inspector(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, busy: bool) {
        let h = ui.available_height();
        kit::card_frame(18).show(ui, |ui| {
            ui.set_min_height(h - 38.0);
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("inspector").show(ui, |ui| {
                ui.set_width(ui.available_width() - 6.0);
                self.inspector_body(ui, ctx, busy);
            });
        });
    }

    fn inspector_body(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, busy: bool) {
        kit::label(ui, "Review");
        ui.label(kit::heading("Transcription", ty::PANEL));
        if let Some(p) = self.project.pages.get(self.selected) {
            let unsure = if p.words_current() {
                diff::place(&p.words, &p.text).iter().filter(|w| w.conf.is_some_and(|c| c < review::UNSURE)).count()
            } else {
                0
            };
            let words = diff::tokens(&p.text).len();
            ui.label(kit::meta(match (words, unsure) {
                (0, _) => "Not read yet. Read the text locally, or ask the AI.".to_string(),
                (n, 0) => format!("{n} words accepted"),
                (n, u) => format!("{n} words accepted · {u} unsure"),
            }));
        }
        ui.add_space(4.0);
        let mut task = None;
        ui.add_enabled_ui(!busy && !self.project.pages.is_empty(), |ui| {
            ui.horizontal_wrapped(|ui| {
                let read = ui
                    .add_enabled_ui(self.ocr_status.is_ok(), |ui| kit::secondary(ui, Some(icon::TEXT_AA), "Read text"))
                    .inner
                    .on_hover_text("Tesseract, on this computer")
                    .on_disabled_hover_text(self.ocr_status.clone().err().unwrap_or_default());
                if read.clicked() {
                    task = Some("OCR");
                }
                if kit::secondary(ui, Some(icon::SPARKLE), "AI correct").on_hover_text(format!("{} · {}", self.settings.provider, if self.settings.model.is_empty() { "no model set" } else { &self.settings.model })).clicked() {
                    task = Some("Correct");
                }
                if kit::tertiary(ui, Some(icon::PENCIL_SIMPLE_LINE), "Rewrite").on_hover_text("A clean rewritten version, kept separate from the source").clicked() {
                    task = Some("Rewrite");
                }
            });
        });
        if let Err(e) = &self.ocr_status {
            kit::note(ui, icon::WARNING, e, theme::amber());
        }
        match task {
            Some("OCR") => {
                if let Some(p) = self.project.pages.get(self.selected).cloned() {
                    self.launch(ctx, move |c| Event::Text("OCR".into(), services::ocr(&p, &c)));
                }
            }
            Some(t) => self.ai(ctx, t),
            None => {}
        }
        ui.add_space(6.0);
        let palette = review::Palette::new(theme::is_dark());
        let font = egui::FontId::new(ty::BODY, egui::FontFamily::Proportional);
        let mut action = 0;
        if let Some(prop) = &mut self.proposal {
            kit::rule(ui);
            ui.horizontal(|ui| {
                kit::label(ui, "Suggestion");
                kit::marker_pill(ui, match prop.kind.as_str() {
                    "OCR" => "Read locally",
                    "Rewrite" => "Rewrite",
                    _ => "AI correction",
                });
            });
            if prop.view.as_ref().is_none_or(|v| v.0 != prop.text) {
                let (mut job, summary) = review::changes(&prop.base, &prop.text, &prop.words, font.clone(), &palette);
                job.wrap.max_width = ui.available_width() - 28.0;
                prop.view = Some((prop.text.clone(), job, summary));
            }
            let (_, job, summary) = prop.view.as_ref().unwrap();
            if !prop.base.trim().is_empty() || summary.unsure + summary.unclear > 0 {
                ui.label(kit::meta(summary.describe()));
                egui::Frame::new()
                    .fill(theme::field())
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(job.clone());
                    });
                legend(ui);
            }
            ui.label(RichText::new("Edit before accepting").family(theme::semibold()).size(ty::META).color(theme::text()));
            let words = prop.words.clone();
            let (f, pal) = (font.clone(), &palette);
            let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
                let mut job = review::unsure_words(text.as_str(), &words, f.clone(), pal);
                job.wrap.max_width = wrap;
                ui.fonts_mut(|fonts| fonts.layout_job(job))
            };
            ui.add(
                egui::TextEdit::multiline(&mut prop.text)
                    .margin(Margin::symmetric(12, 10))
                    .desired_rows(8)
                    .desired_width(f32::INFINITY)
                    .layouter(&mut layouter),
            );
            ui.horizontal(|ui| {
                if kit::primary(ui, Some(icon::CHECK), "Accept").clicked() {
                    action = 1;
                }
                if kit::secondary(ui, Some(icon::X), "Reject").clicked() {
                    action = 2;
                }
            });
        }
        if action == 1 {
            let prop = self.proposal.take().unwrap();
            if let Some(p) = self.project.pages.get_mut(self.selected) {
                p.checkpoint();
                if prop.kind == "Rewrite" {
                    p.rewrite = prop.text;
                } else {
                    p.text = prop.text;
                    if !prop.words.is_empty() {
                        p.words = prop.words;
                        p.words_geometry = prop.geometry;
                    }
                }
            }
            self.status = "Accepted · Undo in the tool bar takes it back".into();
            self.save();
        } else if action == 2 {
            self.proposal = None;
        }
        let Some(p) = self.project.pages.get_mut(self.selected) else {
            return;
        };
        kit::rule(ui);
        kit::label(ui, "Accepted text");
        if !p.words.is_empty() && !p.words_current() {
            kit::note(ui, icon::INFO, "This page was rotated, straightened or cropped after it was read. Read it again so the PDF text lines up with the scan.", theme::accent());
        }
        let before = p.before_edit();
        let words = if p.words_current() { p.words.clone() } else { vec![] };
        let (f, pal) = (font.clone(), &palette);
        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
            let mut job = review::unsure_words(text.as_str(), &words, f.clone(), pal);
            job.wrap.max_width = wrap;
            ui.fonts_mut(|fonts| fonts.layout_job(job))
        };
        let mut changed = ui
            .add_enabled(
                !busy,
                egui::TextEdit::multiline(&mut p.text)
                    .margin(Margin::symmetric(12, 10))
                    .hint_text("Nothing accepted yet")
                    .desired_rows(10)
                    .desired_width(f32::INFINITY)
                    .layouter(&mut layouter),
            )
            .changed();
        if !words.is_empty() {
            ui.label(kit::meta("Underlined words: Tesseract was unsure. Check them against the page."));
        }
        ui.add_space(6.0);
        kit::label(ui, "Rewritten version");
        changed |= ui
            .add_enabled(
                !busy,
                egui::TextEdit::multiline(&mut p.rewrite)
                    .margin(Margin::symmetric(12, 10))
                    .hint_text("Kept separate from the source")
                    .desired_rows(5)
                    .desired_width(f32::INFINITY),
            )
            .changed();
        if changed {
            p.push_history(before);
            self.save();
        }
    }
}

fn legend(ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        kit::pill(ui, "added", review::Palette::new(theme::is_dark()).added_color(), theme::ink());
        kit::pill(ui, "removed", theme::field(), theme::red());
        kit::pill(ui, "check this", theme::amber_wash(), theme::ink());
    });
}

/// A soft shadow under a sheet of paper.
fn paper_shadow(r: Rect, blur: u8) -> egui::Shape {
    egui::Shadow {
        offset: [0, 2],
        blur,
        spread: 0,
        color: Color32::from_black_alpha(if theme::is_dark() { 120 } else { 40 }),
    }
    .as_shape(r, CornerRadius::same(2))
    .into()
}

/// A small painted pill whose right edge sits at `at`.
fn paint_pill(ui: &egui::Ui, at: Pos2, text: &str, fill: Color32, fg: Color32) {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), egui::FontId::new(11.0, theme::bold()), fg);
    let size = galley.size() + Vec2::new(16.0, 6.0);
    let r = Rect::from_min_size(Pos2::new(at.x - size.x, at.y - size.y / 2.0), size);
    ui.painter().rect_filled(r, CornerRadius::same(255), fill);
    ui.painter().galley(r.min + Vec2::new(8.0, 3.0), galley, fg);
}

fn file_stem(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
    let s = s.trim();
    if s.is_empty() { "document".into() } else { s.into() }
}

use crate::diff;
