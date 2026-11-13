//! The application state, background work and the window shell: the dock on the left, the
//! active screen, and the status toast. Screens live in `app/`.
mod document;
mod grading;
mod library;
mod settings;

use crate::{
    model::*,
    secrets,
    services::{self, Prepared, Reading},
    review, update,
    ui::{kit, theme, theme::icon},
};
use eframe::egui::{self, Color32, CornerRadius, Pos2, Stroke, StrokeKind, Vec2};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Screen {
    Document,
    Library,
    Settings,
}

pub(crate) enum Event {
    Devices(Result<String, String>),
    Pages {
        pages: Result<Prepared, String>,
        dpi: u32,
        /// The scanner stopped early but some pages arrived.
        warning: Option<String>,
    },
    Text(String, Result<Reading, String>),
    Key(String, Result<KeyChange, String>),
    Models(String, Result<services::Models, String>),
    /// A rebuilt copy of the page with this id, marked unless marking failed (the warning).
    Rebuild(String, Result<(Rebuild, Option<String>), String>),
    /// New marks for the copy of the page with this id.
    Marks(String, Result<Grade, String>),
    /// An answer key read from a page or file, for review.
    AnswerKey(Result<Vec<KeyItem>, String>),
    Update(Result<update::Status, String>),
    /// The new version was installed and started; this copy should close.
    Updated(Result<(), String>),
}
pub(crate) enum KeyChange {
    Loaded(Option<String>),
    Stored,
    Forgotten,
}
/// Where the API key in use came from, for Settings.
#[derive(PartialEq)]
pub(crate) enum KeyState {
    None,
    Typed,
    Saved,
    Unavailable(String),
}
/// A suggested text awaiting Accept or Reject.
pub(crate) struct Proposal {
    pub(crate) kind: String,
    pub(crate) text: String,
    /// The text it would replace, for the word-level comparison.
    pub(crate) base: String,
    pub(crate) words: Vec<Word>,
    pub(crate) geometry: String,
    /// Comparison of `base` with `text`, refreshed when `text` is edited.
    pub(crate) view: Option<(String, egui::text::LayoutJob, review::Summary)>,
}
pub(crate) struct App {
    pub(crate) project: Project,
    pub(crate) library: Vec<Project>,
    pub(crate) settings: Settings,
    pub(crate) selected: usize,
    pub(crate) search: String,
    pub(crate) status: String,
    pub(crate) screen: Screen,
    /// The last status shown and when it changed, for the status toast.
    pub(crate) status_seen: String,
    pub(crate) status_since: f64,
    pub(crate) settings_section: usize,
    pub(crate) key: String,
    pub(crate) key_state: KeyState,
    pub(crate) ocr_status: Result<String, String>,
    pub(crate) devices: Vec<String>,
    /// Models the current provider offers, with the provider they were fetched for.
    pub(crate) models: Option<(String, Result<services::Models, String>)>,
    pub(crate) update: Option<Result<update::Status, String>>,
    pub(crate) model_job: Option<Receiver<Event>>,
    pub(crate) job: Option<Receiver<Event>>,
    /// Keychain calls get their own channel: a wallet unlock prompt must not block scanning.
    pub(crate) key_job: Option<Receiver<Event>>,
    pub(crate) cancel: services::Cancel,
    pub(crate) proposal: Option<Proposal>,
    pub(crate) thumbs: std::collections::HashMap<String, egui::TextureHandle>,
    pub(crate) zoom: f32,
    /// Last removal (one page, or every blank page), restorable this session.
    pub(crate) removed: Vec<(usize, Page)>,
    pub(crate) texture: Option<(String, egui::TextureHandle)>,
    pub(crate) crop_open: bool,
    /// A rebuilt copy of the selected page awaiting Accept or Reject.
    pub(crate) pending: Option<Rebuild>,
    /// Typeset equations and fitted sizes for drawing rebuilt pages.
    pub(crate) copy_cache: crate::pagecopy::Cache,
    /// Show the rebuilt copy beside the scan when there is one.
    pub(crate) side_by_side: bool,
    /// Scroll the tray to this offset on the next frame.
    pub(crate) scroll_to: Option<Vec2>,
    /// The answer key being typed or reviewed, as text, before it is saved.
    pub(crate) key_draft: Option<String>,
}
impl App {
    /// `secrets`: look up a saved API key at start (off for headless previews).
    pub(crate) fn new(ctx: &egui::Context, secrets: bool) -> Self {
        let settings = Settings::load();
        theme::setup(ctx, settings.dark);
        let mut app = Self {
            project: Project::new(),
            library: library(),
            settings,
            selected: 0,
            search: String::new(),
            status: String::new(),
            screen: Screen::Document,
            status_seen: String::new(),
            status_since: 0.0,
            settings_section: 0,
            key: String::new(),
            key_state: KeyState::None,
            ocr_status: services::ocr_status(),
            devices: vec![],
            models: None,
            update: None,
            model_job: None,
            job: None,
            key_job: None,
            cancel: Arc::new(AtomicBool::new(false)),
            proposal: None,
            thumbs: Default::default(),
            zoom: 1.0,
            removed: vec![],
            texture: None,
            crop_open: false,
            pending: None,
            copy_cache: Default::default(),
            side_by_side: true,
            scroll_to: None,
            key_draft: None,
        };
        if secrets {
            app.load_key(ctx);
        }
        app
    }
    fn save(&mut self) {
        match self.project.save() {
            Ok(()) => self.library = library(),
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }
    fn launch(
        &mut self,
        ctx: &egui::Context,
        f: impl FnOnce(services::Cancel) -> Event + Send + 'static,
    ) {
        if self.job.is_some() {
            return;
        }
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let (tx, rx) = mpsc::channel();
        self.job = Some(rx);
        self.status = "Working…".into();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(f(cancel));
            ctx.request_repaint();
        });
    }
    /// Runs a keychain call in the background; unlocking a wallet can wait on a prompt.
    fn keychain(
        &mut self,
        ctx: &egui::Context,
        f: impl FnOnce(&str) -> Result<KeyChange, String> + Send + 'static,
    ) {
        if self.key_job.is_some() {
            self.status = "Still waiting for the keychain. Check for an unlock prompt.".into();
            return;
        }
        let account = self.settings.key_account();
        let (tx, rx) = mpsc::channel();
        self.key_job = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = f(&account);
            let _ = tx.send(Event::Key(account, r));
            ctx.request_repaint();
        });
    }
    /// Lists the provider's vision models in the background.
    fn fetch_models(&mut self, ctx: &egui::Context) {
        if self.model_job.is_some() {
            return;
        }
        let (s, key) = (self.settings.clone(), self.key.clone());
        let tag = format!("{}|{}", s.provider, s.endpoint);
        let (tx, rx) = mpsc::channel();
        self.model_job = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Event::Models(tag, services::models(&s, &key)));
            ctx.request_repaint();
        });
    }
    fn load_key(&mut self, ctx: &egui::Context) {
        self.key.clear();
        self.key_state = KeyState::None;
        if self.settings.needs_key() {
            self.keychain(ctx, |a| secrets::load(a).map(KeyChange::Loaded));
        }
    }
    fn ai(&mut self, ctx: &egui::Context, task: &str) {
        if let Some(p) = self.project.pages.get(self.selected).cloned() {
            let s = self.settings.clone();
            let key = self.key.clone();
            let task = task.to_owned();
            self.launch(ctx, move |_| {
                Event::Text(
                    task.clone(),
                    services::ai(&p, &s, &key, &task).map(Reading::text),
                )
            });
        }
    }
    /// Rebuilds the selected page as a digital copy and marks it, in the background.
    fn grade(&mut self, ctx: &egui::Context) {
        if let Some(p) = self.project.pages.get(self.selected).cloned() {
            let (s, api, key) = (self.settings.clone(), self.key.clone(), self.project.key.clone());
            self.launch(ctx, move |c| {
                let r = services::rebuild(&p, &s, &api, &c).map(|mut r| {
                    if c.load(Ordering::Relaxed) {
                        return (r, None);
                    }
                    match crate::marking::mark(&p, &r, &key, &s, &api) {
                        Ok(g) => {
                            r.grade = Some(g);
                            (r, None)
                        }
                        Err(e) => (r, Some(format!("Not marked: {e}"))),
                    }
                });
                Event::Rebuild(p.id.clone(), r)
            });
        }
    }
    /// Marks the selected page's copy again, for example after the key changed.
    fn remark(&mut self, ctx: &egui::Context) {
        let Some(p) = self.project.pages.get(self.selected).cloned() else { return };
        let Some(r) = self.pending.clone().or_else(|| p.rebuild_current().cloned()) else { return };
        let (s, api, key) = (self.settings.clone(), self.key.clone(), self.project.key.clone());
        self.launch(ctx, move |_| Event::Marks(p.id.clone(), crate::marking::mark(&p, &r, &key, &s, &api)));
    }
    /// Adds pages in the background so image analysis doesn't freeze the window.
    fn add_pages(
        &mut self,
        ctx: &egui::Context,
        dpi: u32,
        source: impl FnOnce(&AtomicBool) -> Result<(Vec<std::path::PathBuf>, Option<String>), String>
        + Send
        + 'static,
    ) {
        let cleanup = self.settings.auto_cleanup;
        self.launch(ctx, move |c| {
            let (pages, warning) = match source(&c) {
                Ok((paths, warning)) => match services::prepare(paths, cleanup, &c) {
                    Ok((pages, skipped)) => (Ok(pages), warning.or(skipped)),
                    Err(e) => (Err(e), None),
                },
                Err(e) => (Err(e), None),
            };
            Event::Pages {
                pages,
                dpi,
                warning,
            }
        });
    }
    fn poll(&mut self) {
        match self.model_job.as_ref().map(|r| r.try_recv()) {
            Some(Ok(e)) => {
                self.model_job = None;
                self.handle(e);
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => self.model_job = None,
            _ => {}
        }
        match self.key_job.as_ref().map(|r| r.try_recv()) {
            Some(Ok(e)) => {
                self.key_job = None;
                self.handle(e);
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => self.key_job = None,
            _ => {}
        }
        let event = match self.job.as_ref().map(|r| r.try_recv()) {
            Some(Ok(e)) => Some(e),
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.job = None;
                self.status =
                    "The background worker stopped unexpectedly. Your saved project is intact."
                        .into();
                None
            }
            _ => None,
        };
        let Some(e) = event else { return };
        self.job = None;
        if self.cancel.load(Ordering::Relaxed) {
            self.status =
                "Cancelled. Unaccepted AI results discarded; capture files remain saved.".into();
            return;
        }
        self.status = "Done".into();
        self.handle(e);
    }
    fn handle(&mut self, e: Event) {
        match e {
            Event::Models(tag, r) => self.models = Some((tag, r)),
            Event::Rebuild(id, Ok((r, warning))) => {
                if self.project.pages.get(self.selected).is_some_and(|p| p.id == id) {
                    let unsure = r.unsure() + r.grade.as_ref().map_or(0, Grade::unsure);
                    let score = r.grade.as_ref().map(|g| {
                        let (s, m) = g.score();
                        format!(" · {} / {}", points(s), points(m))
                    });
                    self.status = format!(
                        "Page rebuilt{}{}",
                        score.unwrap_or_else(|| format!(" · {} parts", r.blocks.len())),
                        if unsure > 0 { format!(" · {unsure} to check") } else { String::new() }
                    );
                    if let Some(w) = warning {
                        self.status = format!("{} · {w}", self.status);
                    }
                    self.pending = Some(r);
                    self.side_by_side = true;
                } else {
                    self.status = "Rebuilt a page that is no longer selected; discarded.".into();
                }
            }
            Event::Marks(id, Ok(g)) => {
                let page = self.project.pages.get(self.selected).filter(|p| p.id == id);
                let copy = self.pending.clone().or_else(|| page.and_then(Page::rebuild_current).cloned());
                match (page, copy) {
                    (Some(_), Some(mut r)) => {
                        let (s, m) = g.score();
                        self.status = format!("Marked · {} / {} · Accept to keep the new marks", points(s), points(m));
                        r.grade = Some(g);
                        self.pending = Some(r);
                    }
                    _ => self.status = "Marked a page that is no longer selected; discarded.".into(),
                }
            }
            Event::AnswerKey(Ok(items)) => {
                self.status = format!("Answer key read · {} questions · check it, then save", items.len());
                self.key_draft = Some(crate::marking::format_key(&items));
            }
            Event::Rebuild(_, Err(e)) | Event::Marks(_, Err(e)) | Event::AnswerKey(Err(e)) => self.status = e,
            Event::Update(r) => {
                self.status = match &r {
                    Ok(s) if s.behind => "An update is available.".into(),
                    Ok(_) => "You have the latest version.".into(),
                    Err(e) => e.clone(),
                };
                self.update = Some(r);
            }
            Event::Updated(Ok(())) => std::process::exit(0),
            Event::Updated(Err(e)) => self.status = e,
            Event::Devices(Ok(s)) => {
                self.devices = s
                    .lines()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect();
                if let Some(d) = self.devices.first() {
                    self.settings.device = d.clone();
                } else {
                    self.status = "No scanner found. Check USB and power.".into();
                }
            }
            Event::Pages {
                pages: Ok(pages),
                dpi,
                warning,
            } => {
                let before = self.project.pages.len();
                for (path, cleanup) in pages {
                    if let Err(e) = self.project.import(path, dpi, cleanup) {
                        self.status = e;
                    }
                }
                self.save();
                if before == 0 && !self.project.pages.is_empty() {
                    self.selected = 0;
                    self.invalidate();
                }
                let added = &self.project.pages[before..];
                let straightened = added.iter().filter(|p| p.straighten).count();
                let blank = added.iter().filter(|p| p.blank).count();
                self.status = format!("Added {} pages", added.len());
                if straightened > 0 {
                    self.status += &format!(" · straightened {straightened}");
                }
                if blank > 0 {
                    self.status += &format!(" · {blank} look blank");
                }
                if let Some(w) = warning {
                    self.status = format!("{} · {w}", self.status);
                }
            }
            Event::Text(kind, Ok(r)) => {
                let page = self.project.pages.get(self.selected);
                let base = page
                    .map(|p| if kind == "Rewrite" { &p.rewrite } else { &p.text })
                    .cloned()
                    .unwrap_or_default();
                self.proposal = Some(Proposal {
                    kind,
                    text: r.text,
                    base,
                    words: r.words,
                    geometry: r.geometry,
                    view: None,
                });
                self.status = "Suggestion ready · Review the highlighted words before accepting".into();
            }
            Event::Key(account, Ok(change)) if account == self.settings.key_account() => {
                match change {
                    KeyChange::Loaded(Some(k)) => {
                        self.key = k;
                        self.key_state = KeyState::Saved;
                        self.status = format!("Using the saved {account} key from the keychain");
                    }
                    KeyChange::Loaded(None) => self.status.clear(),
                    KeyChange::Stored => {
                        self.key_state = KeyState::Saved;
                        self.status = format!("{account} key saved to the system keychain");
                    }
                    KeyChange::Forgotten => {
                        self.key.clear();
                        self.key_state = KeyState::None;
                        self.status = format!("{account} key removed from the keychain");
                    }
                }
            }
            Event::Key(_, Ok(_)) => {}
            Event::Key(_, Err(e)) => {
                self.status = e.clone();
                self.key_state = KeyState::Unavailable(e);
            }
            Event::Devices(Err(e)) | Event::Pages { pages: Err(e), .. } | Event::Text(_, Err(e)) => {
                self.status = e
            }
        }
    }
    pub(crate) fn invalidate(&mut self) {
        self.texture = None;
        self.thumbs.clear();
        self.proposal = None;
        self.pending = None;
    }
    fn restore(&mut self) {
        if self.removed.is_empty() {
            return;
        }
        for (i, p) in std::mem::take(&mut self.removed) {
            let i = i.min(self.project.pages.len());
            self.project.pages.insert(i, p);
            self.selected = i;
        }
        self.invalidate();
        self.save();
    }
}

pub(crate) fn texture(ctx: &egui::Context, name: &str, img: image::DynamicImage) -> egui::TextureHandle {
    let img = img.to_rgba8();
    ctx.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()),
        egui::TextureOptions::LINEAR,
    )
}

const DOCK_W: f32 = 88.0;

impl App {
    pub(crate) fn draw(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.poll();
        egui::Panel::left("dock")
            .exact_size(DOCK_W + 20.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(theme::cream()).inner_margin(egui::Margin { left: 10, right: 10, top: 10, bottom: 10 }))
            .show(ui, |ui| self.dock(ui, &ctx));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::cream()).inner_margin(egui::Margin { left: 14, right: 24, top: 18, bottom: 18 }))
            .show(ui, |ui| {
                theme::paint_grid(ui.painter(), ui.max_rect().expand2(Vec2::new(24.0, 18.0)));
                match self.screen {
                    Screen::Document => self.document(ui, &ctx),
                    Screen::Library => self.library_screen(ui, &ctx),
                    Screen::Settings => self.settings_screen(ui, &ctx),
                }
            });
        self.toast(&ctx);
    }

    fn dock(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        theme::paint_grid(ui.painter(), ui.max_rect().expand(10.0));
        let rect = ui.max_rect();
        ui.painter().rect(
            rect,
            CornerRadius::same(18),
            theme::rail(),
            if theme::is_dark() { Stroke::new(1.0, theme::border()) } else { Stroke::NONE },
            StrokeKind::Inside,
        );
        let inner = rect.shrink2(Vec2::new(9.0, 12.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
        let ui = &mut child;
        ui.spacing_mut().item_spacing = Vec2::new(0.0, 4.0);
        // The wordmark tile, as verSHion's.
        let (logo, _) = ui.allocate_exact_size(Vec2::new(inner.width(), 56.0), egui::Sense::hover());
        let tile = egui::Rect::from_center_size(logo.center(), Vec2::splat(42.0));
        ui.painter().rect_filled(tile, CornerRadius::same(12), theme::accent().lerp_to_gamma(Color32::BLACK, if theme::is_dark() { 0.55 } else { 0.0 }));
        ui.painter().text(tile.center(), egui::Align2::CENTER_CENTER, "d", egui::FontId::new(26.0, theme::serif_semibold()), Color32::from_rgb(0xf7, 0xf4, 0xed));
        ui.add_space(10.0);
        for (glyph, label, target) in [
            (icon::FILE_TEXT, "Document", Screen::Document),
            (icon::BOOKS, "Library", Screen::Library),
            (icon::GEAR, "Settings", Screen::Settings),
        ] {
            if dock_item(ui, glyph, label, self.screen == target) {
                if target == Screen::Library {
                    self.save();
                }
                self.screen = target;
            }
        }
        let floor = (ui.available_height() - 60.0).max(0.0);
        ui.add_space(floor);
        let dark = theme::is_dark();
        let (glyph, label) = if dark { (icon::SUN, "Light") } else { (icon::MOON, "Dark") };
        if dock_item(ui, glyph, label, false) {
            self.settings.dark = !dark;
            theme::set_dark(!dark, ctx);
            let _ = self.settings.save();
            self.proposal.iter_mut().for_each(|p| p.view = None);
        }
    }

    /// Busy work and recent messages float at the foot of the window, like verSHion's toasts.
    fn toast(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if self.status != self.status_seen {
            self.status_seen = self.status.clone();
            self.status_since = now;
        }
        let busy = self.job.is_some();
        let age = now - self.status_since;
        if !busy && (age > 6.0 || self.status.is_empty()) {
            return;
        }
        if !busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        let alpha = if busy { 1.0 } else { ((6.0 - age) / 0.6).clamp(0.0, 1.0) as f32 };
        egui::Area::new(egui::Id::new("status-toast"))
            .anchor(egui::Align2::CENTER_BOTTOM, Vec2::new(DOCK_W / 2.0, -22.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_opacity(alpha);
                kit::card_frame(0)
                    .inner_margin(egui::Margin::symmetric(16, 8))
                    .corner_radius(CornerRadius::same(255))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if busy {
                                ui.add(egui::Spinner::new().size(16.0).color(theme::accent()));
                            } else {
                                kit::glyph(ui.painter(), ui.cursor().min + Vec2::new(9.0, 13.0), icon::CHECK, 16.0, theme::accent());
                                ui.add_space(20.0);
                            }
                            ui.label(kit::body(&self.status).color(theme::ink()));
                            if busy && kit::button(ui, kit::Tier::Tertiary, None, "Cancel", 28.0).clicked() {
                                self.cancel.store(true, Ordering::Relaxed);
                                self.status = "Cancelling…".into();
                            }
                        });
                    });
            });
    }
}

/// One dock item: icon over a small label; the active one wears the highlighter.
fn dock_item(ui: &mut egui::Ui, glyph: &str, label: &str, active: bool) -> bool {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 56.0), egui::Sense::click());
    if active {
        ui.painter().rect_filled(rect, CornerRadius::same(13), theme::highlighter());
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(13), theme::dock_text().gamma_multiply(0.12));
    }
    let col = if active { theme::on_highlighter() } else { theme::dock_text() };
    kit::glyph(ui.painter(), Pos2::new(rect.center().x, rect.top() + 21.0), glyph, 21.0, col);
    ui.painter().text(
        Pos2::new(rect.center().x, rect.bottom() - 11.0),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::new(10.5, theme::bold()),
        col,
    );
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, active, label));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        self.draw(ui);
    }
}
