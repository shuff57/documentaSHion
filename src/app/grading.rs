//! The grading part of the inspector: the answer key, the rebuilt copy awaiting review, and the
//! suggested marks with the teacher's overrides.
use super::*;
use crate::{
    marking,
    ui::kit::{Tier, ty},
};
use egui::{Align, Layout, Margin, Rect, RichText, Sense};

/// What a mark row changed this frame.
#[derive(Default)]
struct Edit {
    changed: bool,
    /// The change starts a new undo step (not the middle of a drag).
    step: bool,
    /// A drag is still under way; save when it ends.
    dragging: bool,
}

impl App {
    pub(super) fn grading(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, busy: bool) {
        self.blank_copy(ui, ctx, busy);
        self.answer_key(ui, ctx, busy);
        self.copy_review(ui);
        self.marks(ui, ctx, busy);
    }

    /// The blank copy of the worksheet the student's writing is read against.
    fn blank_copy(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, busy: bool) {
        kit::rule(ui);
        ui.horizontal(|ui| {
            kit::label(ui, "Blank copy");
            if let Some(b) = &self.project.blank {
                let lines: usize = b.pages.iter().map(|p| p.lines.len()).sum();
                kit::quiet_pill(ui, &format!("{} page{} · {lines} printed lines", b.pages.len(), if b.pages.len() == 1 { "" } else { "s" }));
            }
        });
        let has_key = self.datalab().is_some();
        match &self.project.blank {
            None => {
                ui.label(kit::meta(
                    "With a blank copy of the worksheet, AI grade reads the student's writing with Datalab: each page is lined up with the blank, the student's name blanked before it's sent, and only what the student added kept.",
                ));
            }
            Some(b) if b.pages.iter().all(|p| p.names.is_empty()) => {
                kit::note(ui, icon::WARNING, "No Name line was found on the blank, so there's no telling where names are written: pages won't be sent. Add the page with the Name line.", theme::amber());
            }
            Some(_) => {
                ui.label(kit::meta("AI grade reads each page against this blank with Datalab. The Name line is blanked before a page is sent. Add the other side if the worksheet has two."));
            }
        }
        if !has_key {
            kit::note(ui, icon::INFO, "Needs a Datalab key: Settings → AI provider → Student work.", theme::accent());
        }
        let mut read = None;
        let mut import = false;
        let mut remove = false;
        ui.add_enabled_ui(!busy && has_key, |ui| {
            ui.horizontal_wrapped(|ui| {
                let more = self.project.blank.is_some();
                kit::menu_button(ui, Tier::Secondary, Some(icon::SCAN), if more { "Add a page" } else { "From a page" }, |ui| {
                    for i in 0..self.project.pages.len() {
                        if kit::menu_item(ui, icon::FILE_TEXT, &format!("Page {}", i + 1)).clicked() {
                            read = Some(i);
                        }
                    }
                })
                .on_hover_text("Use a scanned page of this document as the blank");
                if kit::tertiary(ui, Some(icon::UPLOAD_SIMPLE), "Import…").on_hover_text("Read the blank from images or a PDF").clicked() {
                    import = true;
                }
                if more && kit::tertiary(ui, Some(icon::X), "Remove").clicked() {
                    remove = true;
                }
            });
        });
        if let Some(i) = read {
            let p = self.project.pages[i].clone();
            self.read_blank(ctx, vec![p], vec![]);
        } else if import {
            if let Some(paths) = rfd::FileDialog::new().add_filter("Blank pages", &["png", "jpg", "jpeg", "tif", "tiff", "pnm", "pdf"]).pick_files() {
                self.read_blank(ctx, vec![], paths);
            }
        } else if remove {
            self.project.blank = None;
            self.status = "Blank copy removed · AI grade reads pages with the vision model".into();
            self.save();
        }
    }

    fn answer_key(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, busy: bool) {
        kit::rule(ui);
        ui.horizontal(|ui| {
            kit::label(ui, "Answer key");
            if !self.project.key.is_empty() {
                let total: f32 = self.project.key.iter().map(|k| k.points).sum();
                kit::quiet_pill(ui, &format!("{} questions · {} pts", self.project.key.len(), points(total)));
            }
        });
        let mut save = None;
        let mut read = None;
        let mut import = false;
        if let Some(draft) = &mut self.key_draft {
            ui.add_enabled(
                !busy,
                egui::TextEdit::multiline(draft)
                    .margin(Margin::symmetric(12, 10))
                    .desired_rows(6)
                    .desired_width(f32::INFINITY)
                    .hint_text("1. 13\n2. $x = 4$ [2]\n3. slope = -2 (3 pts)"),
            );
            let items = marking::parse_key(draft);
            let total: f32 = items.iter().map(|k| k.points).sum();
            ui.label(kit::meta(format!(
                "{} questions · {} points. One per line: number, answer, then points as [2]. Math in $…$.",
                items.len(),
                points(total)
            )));
            ui.add_enabled_ui(!busy, |ui| {
                ui.horizontal(|ui| {
                    if kit::primary(ui, Some(icon::CHECK), if items.is_empty() { "Remove key" } else { "Save key" }).clicked() {
                        save = Some(items);
                    }
                    if kit::secondary(ui, Some(icon::X), "Cancel").clicked() {
                        self.key_draft = None;
                    }
                });
            });
        } else {
            ui.label(kit::meta(if self.project.key.is_empty() {
                "No key: AI grade solves each question and marks against its own answers. Add a key to mark against yours."
            } else {
                "AI grade marks every page of this document against this key."
            }));
            ui.add_enabled_ui(!busy, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let label = if self.project.key.is_empty() { "Type key" } else { "Edit key" };
                    if kit::secondary(ui, Some(icon::PENCIL_SIMPLE_LINE), label).clicked() {
                        self.key_draft = Some(marking::format_key(&self.project.key));
                    }
                    kit::menu_button(ui, Tier::Secondary, Some(icon::SCAN), "From a page", |ui| {
                        for i in 0..self.project.pages.len() {
                            if kit::menu_item(ui, icon::FILE_TEXT, &format!("Page {}", i + 1)).clicked() {
                                read = Some(i);
                            }
                        }
                    })
                    .on_hover_text("Read the key from a scanned page of this document");
                    if kit::tertiary(ui, Some(icon::UPLOAD_SIMPLE), "Import…").on_hover_text("Read the key from images or a PDF").clicked() {
                        import = true;
                    }
                });
            });
        }
        let (s, api) = (self.settings.clone(), self.key.clone());
        if let Some(i) = read {
            let p = self.project.pages[i].clone();
            self.launch(ctx, move |_| Event::AnswerKey(marking::read_key(&p, &s, &api)));
        } else if import {
            if let Some(paths) = rfd::FileDialog::new()
                .add_filter("Key pages", &["png", "jpg", "jpeg", "tif", "tiff", "pnm", "pdf"])
                .pick_files()
            {
                self.launch(ctx, move |c| Event::AnswerKey(marking::read_key_files(paths, &s, &api, &c)));
            }
        } else if let Some(items) = save {
            let n = items.len();
            self.project.key = items;
            self.key_draft = None;
            let copy = self.pending.is_some() || self.project.pages.get(self.selected).is_some_and(|p| p.rebuild_current().is_some());
            self.status = match (n, copy) {
                (0, _) => "Answer key removed · AI grade will solve the questions itself".into(),
                (n, true) => format!("Answer key saved · {n} questions · Mark again to apply it"),
                (n, false) => format!("Answer key saved · {n} questions"),
            };
            self.save();
        }
    }

    /// The pending rebuilt copy, with Accept and Reject, or a note on the accepted one.
    fn copy_review(&mut self, ui: &mut egui::Ui) {
        let mut action = 0;
        if let Some(r) = &self.pending {
            kit::rule(ui);
            ui.horizontal(|ui| {
                kit::label(ui, "Rebuilt page");
                kit::marker_pill(ui, "AI grade");
            });
            let hand = r.blocks.iter().filter(|b| b.handwritten).count();
            let unsure = r.unsure();
            ui.label(kit::meta(format!(
                "{} parts · {hand} in the student's writing{}",
                r.blocks.len(),
                if unsure > 0 { format!(" · {unsure} to check") } else { String::new() }
            )));
            if unsure > 0 {
                kit::note(ui, icon::WARNING, "Shaded parts are ones the model was unsure of, or couldn't read. Point at a part to see it outlined on the scan.", theme::amber());
            }
            if r.model.starts_with("Datalab") {
                kit::note(ui, icon::INFO, "Read by Datalab against the blank copy. Each mark below shows the student's writing from the scan beside how it was read.", theme::accent());
            } else if !r.anchored {
                kit::note(ui, icon::INFO, "Positions are the model's estimate. With Tesseract installed, AI grade lines the copy up with the printed text.", theme::accent());
            }
            ui.label(kit::meta(format!("Blue text is the student's writing · {}", r.model)));
            ui.horizontal(|ui| {
                if kit::primary(ui, Some(icon::CHECK), "Accept").clicked() {
                    action = 1;
                }
                if kit::secondary(ui, Some(icon::X), "Reject").clicked() {
                    action = 2;
                }
            });
        } else if let Some(p) = self.project.pages.get(self.selected)
            && p.rebuild.is_some()
            && p.rebuild_current().is_none()
        {
            kit::note(ui, icon::INFO, "This page was rotated, straightened or cropped after it was rebuilt. Use AI grade again to line the copy up.", theme::accent());
        }
        if action == 1 {
            let r = self.pending.take();
            if let Some(p) = self.project.pages.get_mut(self.selected) {
                p.checkpoint();
                p.rebuild = r;
            }
            self.status = "Rebuilt copy and marks saved · Undo in the tool bar takes them back".into();
            self.save();
        } else if action == 2 {
            self.pending = None;
        }
    }

    /// The suggested marks, each one open to the teacher's override.
    fn marks(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, busy: bool) {
        let Some(page) = self.project.pages.get(self.selected) else { return };
        let pending = self.pending.is_some();
        let Some(copy) = self.pending.as_ref().or_else(|| page.rebuild_current()) else { return };
        let grade = copy.grade.clone();
        kit::rule(ui);
        ui.horizontal(|ui| {
            kit::label(ui, "Marks");
            if grade.as_ref().is_some_and(|g| g.solved) {
                kit::marker_pill(ui, "AI-solved");
            }
            if let Some(g) = &grade {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let (s, m) = g.score();
                    ui.label(kit::heading(&format!("{} / {}", points(s), points(m)), ty::PANEL));
                });
            }
        });
        let mut again = false;
        let Some(mut g) = grade else {
            ui.label(kit::meta("Not marked yet."));
            if ui.add_enabled_ui(!busy, |ui| kit::secondary(ui, Some(icon::EXAM), "Mark answers")).inner.clicked() {
                self.remark(ctx);
            }
            return;
        };
        let count = |v: Verdict| g.marks.iter().filter(|m| m.verdict == v).count();
        let mut summary = vec![];
        for (v, word) in [(Verdict::Right, "right"), (Verdict::Partial, "partial"), (Verdict::Wrong, "wrong"), (Verdict::Blank, "blank")] {
            let n = count(v);
            if n > 0 {
                summary.push(format!("{n} {word}"));
            }
        }
        let unsure = g.unsure();
        if unsure > 0 {
            summary.push(format!("{unsure} to check"));
        }
        ui.label(kit::meta(summary.join(" · ")));
        if g.solved {
            kit::note(ui, icon::INFO, "No answer key: the model solved these questions itself. Check its answers before trusting the marks.", theme::accent());
        }
        if unsure > 0 {
            kit::note(ui, icon::WARNING, &format!("{unsure} flagged: the handwriting or the mark is in doubt for {}. Check against the scan.", if unsure == 1 { "this one" } else { "these" }), theme::amber());
        }
        let look = g.marks.iter().filter(|m| m.unsure() || matches!(m.verdict, Verdict::Wrong | Verdict::Partial)).count();
        if look > 0 && look < g.marks.len() {
            ui.checkbox(&mut self.marks_look, format!("Only the {look} that need a look"));
        }
        let only = self.marks_look && look > 0;
        let mut edit = Edit::default();
        let blocks = copy.blocks.clone();
        let tex = self.texture.as_ref().map(|t| t.1.clone());
        let cache = &mut self.copy_cache;
        let mut hover = None;
        ui.add_enabled_ui(!busy, |ui| {
            for (i, m) in g.marks.iter_mut().enumerate() {
                if only && !(m.unsure() || matches!(m.verdict, Verdict::Wrong | Verdict::Partial)) {
                    continue;
                }
                let (e, hot) = mark_row(ui, i, m, g.solved, &blocks, tex.as_ref(), cache);
                if hot {
                    hover = Some(i);
                }
                edit.changed |= e.changed;
                edit.step |= e.step;
                edit.dragging |= e.dragging;
            }
            ui.add_space(4.0);
            again = kit::secondary(ui, Some(icon::ARROW_CLOCKWISE), "Mark again")
                .on_hover_text("Mark this copy again with the current answer key")
                .clicked();
        });
        self.mark_hover = hover;
        if !g.model.is_empty() {
            ui.label(kit::meta(format!("Suggested by {}. Click a mark to change it; point at one to see it on the scan.", g.model)));
        }
        if edit.changed {
            if pending {
                if let Some(r) = &mut self.pending {
                    r.grade = Some(g);
                }
            } else {
                let p = &mut self.project.pages[self.selected];
                let before = p.before_edit();
                if let Some(r) = &mut p.rebuild {
                    r.grade = Some(g);
                }
                if edit.step {
                    p.push_history(before);
                }
            }
        }
        if !pending && (edit.changed || edit.step) && !edit.dragging {
            self.save();
        }
        if again {
            self.remark(ctx);
        }
    }
}

/// One question's mark: verdict, points, the student's writing as scanned and as read, the
/// expected answer and the model's note. Also whether the pointer is on it.
fn mark_row(ui: &mut egui::Ui, i: usize, m: &mut Mark, solved: bool, blocks: &[Block], tex: Option<&egui::TextureHandle>, cache: &mut crate::pagecopy::Cache) -> (Edit, bool) {
    let mut e = Edit::default();
    let frame = egui::Frame::new()
        .fill(theme::field())
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(12, 10))
        .stroke(if m.unsure() { Stroke::new(1.5, theme::amber()) } else { Stroke::NONE })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.push_id(("mark", i), |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Q{}", m.question)).family(theme::bold()).size(ty::BODY).color(theme::ink()));
                    if m.unsure() {
                        let (r, resp) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                        kit::glyph(ui.painter(), r.center(), icon::WARNING, 15.0, theme::amber());
                        resp.on_hover_text(format!("The model was {:.0}% sure of this mark", m.conf * 100.0));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(kit::meta(format!("of {}", points(m.max))));
                        let r = ui.add(egui::DragValue::new(&mut m.points).range(0.0..=m.max).speed(0.05).max_decimals(2));
                        if r.changed() {
                            m.overridden = true;
                            m.verdict = if m.points <= 0.0 {
                                if m.verdict == Verdict::Blank { Verdict::Blank } else { Verdict::Wrong }
                            } else if m.points >= m.max {
                                Verdict::Right
                            } else {
                                Verdict::Partial
                            };
                            e.changed = true;
                            e.step |= r.drag_started() || !r.dragged();
                        }
                        e.step |= r.drag_started();
                        e.dragging |= r.dragged();
                        if r.drag_stopped() {
                            e.changed = true;
                        }
                    });
                });
                let mut v = m.verdict;
                if kit::segmented(ui, &mut v, &[(Verdict::Right, "Right"), (Verdict::Partial, "Partial"), (Verdict::Wrong, "Wrong"), (Verdict::Blank, "Blank")]) {
                    m.points = marking::points_for(v, m.points, m.max);
                    m.verdict = v;
                    m.overridden = true;
                    e.changed = true;
                    e.step = true;
                }
                // The student's writing as scanned, to check the reading against.
                let area = m.blocks.iter().filter_map(|&b| blocks.get(b)).map(|b| b.bbox).reduce(|a, b| {
                    let (x0, y0) = (a[0].min(b[0]), a[1].min(b[1]));
                    [x0, y0, (a[0] + a[2]).max(b[0] + b[2]) - x0, (a[1] + a[3]).max(b[1] + b[3]) - y0]
                });
                let typed = |ui: &mut egui::Ui, cache: &mut crate::pagecopy::Cache, label: &str, text: &str, student: bool| {
                    ui.label(kit::meta(label));
                    if text.trim().is_empty() {
                        ui.label(kit::meta("—"));
                    } else {
                        crate::pagecopy::slip(ui, cache, text, student, 15.0);
                    }
                };
                ui.horizontal_top(|ui| {
                    if let (Some([x, y, w, h]), Some(tex)) = (area, tex) {
                        let pad = 6.0;
                        let uv = Rect::from_min_max(Pos2::new((x - pad).max(0.0) / 1000.0, (y - pad).max(0.0) / 1000.0), Pos2::new((x + w + pad).min(1000.0) / 1000.0, (y + h + pad).min(1000.0) / 1000.0));
                        let px = tex.size_vec2() * uv.size();
                        let k = (ui.available_width() * 0.45 / px.x.max(1.0)).min(110.0 / px.y.max(1.0)).min(1.5);
                        let (r, resp) = ui.allocate_exact_size(px * k, Sense::hover());
                        ui.painter().rect_filled(r.expand(1.0), CornerRadius::same(4), Color32::WHITE);
                        ui.painter().image(tex.id(), r, uv, Color32::WHITE);
                        resp.on_hover_text("The student's writing, from the scan");
                    }
                    ui.vertical(|ui| {
                        typed(ui, cache, "Read as", &m.answer, true);
                        typed(ui, cache, if solved { "AI's answer" } else { "Key" }, &m.expected, false);
                    });
                });
                // The two readers disagreed about the writing: both readings, to judge by the scan.
                let mut seen = vec![];
                for b in m.blocks.iter().filter_map(|&i| blocks.get(i)).filter(|b| !b.alt.is_empty()) {
                    // The pieces of one stack share its second reading.
                    if seen.contains(&&b.alt) {
                        continue;
                    }
                    seen.push(&b.alt);
                    kit::note(ui, icon::WARNING, "The two readings of this writing differ. Check the scan.", theme::amber());
                    let first: Vec<&str> = blocks.iter().filter(|o| o.alt == b.alt).map(|o| o.text.as_str()).collect();
                    typed(ui, cache, "Datalab read", &first.join("\n"), true);
                    typed(ui, cache, "Second reading", &b.alt, true);
                }
                if !m.note.trim().is_empty() {
                    ui.add(egui::Label::new(kit::body(&m.note).size(ty::META)).wrap());
                }
            });
        });
    let hot = ui.rect_contains_pointer(frame.response.rect);
    (e, hot)
}
