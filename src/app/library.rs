//! The Library screen: every saved project as a card with its first page.
use super::*;
use crate::ui::kit::{Tier, ty};
use egui::{Align, Layout, Rect, Sense};

const CARD_W: f32 = 220.0;
const CARD_H: f32 = 264.0;

impl App {
    pub(super) fn library_screen(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let busy = self.job.is_some();
        let mut open = None;
        let mut new = false;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(kit::meta(format!("{} saved {}", self.library.len(), if self.library.len() == 1 { "project" } else { "projects" })));
                ui.label(kit::heading("Library", ty::TITLE));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    if kit::primary(ui, Some(icon::PLUS), "New project").clicked() {
                        new = true;
                    }
                });
            });
        });
        ui.add_space(10.0);
        ui.allocate_ui(Vec2::new(ui.available_width().min(520.0), 40.0), |ui| {
            kit::search(ui, &mut self.search, "Search titles and transcriptions");
        });
        ui.add_space(16.0);
        let q = self.search.to_lowercase();
        let mut entries: Vec<&Project> = self
            .library
            .iter()
            .filter(|p| {
                p.name.to_lowercase().contains(&q)
                    || p.pages.iter().any(|p| p.text.to_lowercase().contains(&q) || p.rewrite.to_lowercase().contains(&q))
            })
            .collect();
        // Newest first: project ids are creation timestamps.
        entries.sort_by(|a, b| b.id.len().cmp(&a.id.len()).then(b.id.cmp(&a.id)));
        if self.library.is_empty() {
            let w = ui.available_width().min(620.0);
            ui.allocate_ui(Vec2::new(w, 0.0), |ui| {
                if kit::empty_state(
                    ui,
                    icon::BOOKS,
                    "Nothing saved yet",
                    "Every scan becomes a project here, searchable by its title and its text.",
                    &[(Some(icon::SCAN), "Start scanning", Tier::Primary)],
                )
                .is_some()
                {
                    self.screen = Screen::Document;
                }
            });
            return;
        }
        if entries.is_empty() {
            ui.label(kit::body(format!("Nothing matches “{}”.", self.search)));
            return;
        }
        let entries: Vec<Project> = entries.into_iter().cloned().collect();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let gap = 16.0;
            let cols = ((ui.available_width() + gap) / (CARD_W + gap)).floor().max(1.0) as usize;
            for row in entries.chunks(cols) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for p in row {
                        if self.project_card(ui, ctx, p, busy) {
                            open = Some(p.clone());
                        }
                    }
                });
                ui.add_space(gap - 8.0);
            }
        });
        if new {
            self.save();
            self.project = Project::new();
            self.selected = 0;
            self.removed.clear();
            self.invalidate();
            self.screen = Screen::Document;
        }
        if let Some(p) = open {
            if p.id != self.project.id {
                self.save();
                self.project = p;
                self.selected = 0;
                self.removed.clear();
                self.invalidate();
            }
            self.screen = Screen::Document;
        }
    }

    /// One project: its first page on a tray, the title in serif, and a meta line.
    fn project_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, p: &Project, busy: bool) -> bool {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(CARD_W, CARD_H), if busy { Sense::hover() } else { Sense::click() });
        let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
        let lift = rect.translate(Vec2::new(0.0, -2.0 * hover));
        ui.painter().add(
            egui::Shadow {
                offset: [0, 2 + (4.0 * hover) as i8],
                blur: 10 + (8.0 * hover) as u8,
                spread: 0,
                color: Color32::from_black_alpha(if theme::is_dark() { 60 } else { 12 }),
            }
            .as_shape(lift, CornerRadius::same(16)),
        );
        let current = p.id == self.project.id;
        ui.painter().rect(
            lift,
            CornerRadius::same(16),
            theme::card(),
            Stroke::new(if current { 1.5 } else { 1.0 }, if current { theme::accent() } else { theme::border() }),
            StrokeKind::Inside,
        );
        let tray = Rect::from_min_size(lift.min + Vec2::new(10.0, 10.0), Vec2::new(CARD_W - 20.0, 160.0));
        ui.painter().rect_filled(tray, CornerRadius::same(10), theme::table());
        if let Some(page) = p.pages.first() {
            let key = format!("lib-{}", page.render_key());
            if !self.thumbs.contains_key(&key) {
                if let Ok(img) = page.render_within(Some(420)) {
                    self.thumbs.insert(key.clone(), texture(ctx, &key, img.thumbnail(240, 300)));
                }
            }
            if let Some(t) = self.thumbs.get(&key) {
                let s = t.size_vec2();
                let slot = tray.shrink(12.0);
                let r = Rect::from_center_size(slot.center(), s * (slot.width() / s.x).min(slot.height() / s.y));
                ui.painter().add(
                    egui::Shadow { offset: [0, 1], blur: 4, spread: 0, color: Color32::from_black_alpha(40) }.as_shape(r, CornerRadius::same(2)),
                );
                ui.painter().image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
        } else {
            kit::glyph(ui.painter(), tray.center(), icon::FILE_DASHED, 34.0, theme::muted());
        }
        let title = ui.painter().layout(
            p.name.clone(),
            egui::FontId::new(ty::CARD, theme::serif()),
            theme::ink(),
            CARD_W - 28.0,
        );
        let title_pos = Pos2::new(lift.left() + 14.0, tray.bottom() + 12.0);
        ui.painter().with_clip_rect(Rect::from_min_size(title_pos, Vec2::new(CARD_W - 28.0, 44.0))).galley(title_pos, title, theme::ink());
        let read = p.pages.iter().filter(|p| !p.text.is_empty()).count();
        let meta = format!(
            "{} {} · {} read",
            p.pages.len(),
            if p.pages.len() == 1 { "page" } else { "pages" },
            read
        );
        ui.painter().text(
            Pos2::new(lift.left() + 14.0, lift.bottom() - 14.0),
            egui::Align2::LEFT_BOTTOM,
            meta,
            egui::FontId::new(ty::META, egui::FontFamily::Proportional),
            theme::muted(),
        );
        if current {
            let galley = ui.painter().layout_no_wrap("Open".into(), egui::FontId::new(11.0, theme::bold()), theme::on_highlighter());
            let r = Rect::from_min_size(tray.min + Vec2::new(8.0, 8.0), galley.size() + Vec2::new(16.0, 6.0));
            ui.painter().rect_filled(r, CornerRadius::same(255), theme::highlighter());
            ui.painter().galley(r.min + Vec2::new(8.0, 3.0), galley, theme::on_highlighter());
        }
        resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
    }
}
