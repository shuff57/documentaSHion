//! The shared controls, after verSHion's `kit.rs` and `pageshell.rs`: three button tiers, icon
//! buttons, cards, section labels, pills, a switch and a segmented control. Everything is
//! painter-drawn so it looks the same on every screen.
use super::theme::{self, icon};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Frame, Margin, Pos2, Rect, Response, RichText,
    Sense, Stroke, StrokeKind, Ui, Vec2,
};

/// The type scale, in points.
pub mod ty {
    pub const TITLE: f32 = 30.0;
    pub const SECTION: f32 = 22.0;
    pub const PANEL: f32 = 19.0;
    pub const CARD: f32 = 17.0;
    pub const BODY: f32 = 14.0;
    pub const META: f32 = 13.0;
    pub const LABEL: f32 = 11.5;
    pub const BUTTON: f32 = 15.0;
    pub const BUTTON_SM: f32 = 13.5;
}

pub fn glyph(painter: &egui::Painter, center: Pos2, icon: &str, size: f32, color: Color32) {
    painter.text(center, Align2::CENTER_CENTER, icon, theme::icon_font(size), color);
}

fn focus_ring(ui: &Ui, rect: Rect, radius: u8) {
    ui.painter().rect_stroke(
        rect.expand(1.0),
        CornerRadius::same(radius + 1),
        Stroke::new(2.0, theme::accent()),
        StrokeKind::Outside,
    );
}

/// A control that can't be used yet: neutral grey at full strength, not egui's half fade.
fn unavailable() -> (Color32, Color32, Color32) {
    if theme::is_dark() {
        (Color32::from_rgb(0x46, 0x42, 0x3e), Color32::from_rgb(0xb5, 0xb0, 0xaa), Color32::from_rgb(0x5c, 0x57, 0x52))
    } else {
        (Color32::from_rgb(0xe6, 0xe4, 0xe1), Color32::from_rgb(0x6e, 0x6a, 0x66), Color32::from_rgb(0xc8, 0xc5, 0xc1))
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Tier {
    /// The screen's one main action: solid accent.
    Primary,
    /// Card fill, control edge, ink bold label.
    Secondary,
    /// No fill, control edge, ink semibold label.
    Tertiary,
}

/// A button of one tier, `h` tall, with an optional leading icon.
pub fn button(ui: &mut Ui, tier: Tier, icon: Option<&str>, label: &str, h: f32) -> Response {
    let enabled = ui.is_enabled();
    let accent = theme::accent();
    let toward = if theme::is_dark() { Color32::WHITE } else { Color32::BLACK };
    let (rest, hover, press, stroke, fg, family) = match tier {
        Tier::Primary => (
            accent,
            accent.lerp_to_gamma(toward, 0.12),
            accent.lerp_to_gamma(toward, 0.22),
            Stroke::NONE,
            theme::on_accent(),
            theme::bold(),
        ),
        Tier::Secondary => (theme::card(), theme::hover_bg(), theme::accent_faded(), Stroke::new(1.0, theme::edge()), theme::ink(), theme::bold()),
        Tier::Tertiary => (Color32::TRANSPARENT, theme::hover_bg(), theme::accent_faded(), Stroke::new(1.0, theme::edge()), theme::ink(), theme::semibold()),
    };
    let (rest, stroke, fg) = if enabled {
        (rest, stroke, fg)
    } else {
        let (f, t, e) = unavailable();
        (f, Stroke::new(1.0, e), t)
    };
    let (size, pad) = if h >= 38.0 { (ty::BUTTON, 18.0) } else { (ty::BUTTON_SM, 13.0) };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), FontId::new(size, family), fg);
    let (icon_w, gap) = match (icon, label.is_empty()) {
        (Some(_), false) => (16.0, 7.0),
        (Some(_), true) => (16.0, 0.0),
        _ => (0.0, 0.0),
    };
    let w = pad * 2.0 + icon_w + gap + galley.size().x;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w.ceil(), h), sense);
    let fill = if !enabled {
        rest
    } else if resp.is_pointer_button_down_on() {
        press
    } else if resp.hovered() {
        hover
    } else {
        rest
    };
    ui.painter().rect(rect, CornerRadius::same(12), fill, stroke, StrokeKind::Inside);
    let mut x = rect.left() + pad;
    if let Some(i) = icon {
        glyph(ui.painter(), Pos2::new(x + icon_w / 2.0, rect.center().y), i, 16.0, fg);
        x += icon_w + gap;
    }
    ui.painter().galley(Pos2::new(x, rect.center().y - galley.size().y / 2.0), galley, fg);
    if resp.has_focus() {
        focus_ring(ui, rect, 12);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

pub fn primary(ui: &mut Ui, icon: Option<&str>, label: &str) -> Response {
    button(ui, Tier::Primary, icon, label, 40.0)
}
pub fn secondary(ui: &mut Ui, icon: Option<&str>, label: &str) -> Response {
    button(ui, Tier::Secondary, icon, label, 36.0)
}
pub fn tertiary(ui: &mut Ui, icon: Option<&str>, label: &str) -> Response {
    button(ui, Tier::Tertiary, icon, label, 34.0)
}

/// A square icon button. `on` shows it pressed in (accent wash, accent glyph) for toggles.
pub fn icon_button(ui: &mut Ui, glyph_s: &str, side: f32, tip: &str, on: bool) -> Response {
    let enabled = ui.is_enabled();
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(side), sense);
    let (fill, fg) = if !enabled {
        (Color32::TRANSPARENT, theme::muted().gamma_multiply(0.5))
    } else if resp.is_pointer_button_down_on() || on {
        (theme::accent_faded(), if on { theme::accent() } else { theme::ink() })
    } else if resp.hovered() {
        (theme::hover_bg(), theme::ink())
    } else {
        (Color32::TRANSPARENT, theme::ink())
    };
    ui.painter().rect_filled(rect, CornerRadius::same(10), fill);
    glyph(ui.painter(), rect.center(), glyph_s, (side * 0.5).clamp(14.0, 20.0), fg);
    if resp.has_focus() {
        focus_ring(ui, rect, 10);
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, on, tip));
    let resp = if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
    resp.on_hover_text(tip)
}

/// A thin vertical divider inside a tool pill.
pub fn tool_divider(ui: &mut Ui, h: f32) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(9.0, h), Sense::hover());
    ui.painter().vline(r.center().x, r.shrink2(Vec2::new(0.0, 8.0)).y_range(), Stroke::new(1.0, theme::border()));
}

/// A card: 16 px corners, hairline border, soft shadow.
pub fn card_frame(pad: i8) -> Frame {
    Frame::new()
        .fill(theme::card())
        .stroke(Stroke::new(1.0, theme::border()))
        .corner_radius(CornerRadius::same(16))
        .shadow(egui::Shadow {
            offset: [0, 2],
            blur: 10,
            spread: 0,
            color: Color32::from_black_alpha(if theme::is_dark() { 60 } else { 10 }),
        })
        .inner_margin(Margin::same(pad))
}

/// A section label: bold caps, hair-spaced, muted.
pub fn caps(text: &str) -> RichText {
    let spaced: String = text.to_uppercase().chars().flat_map(|c| [c, '\u{200A}']).collect();
    RichText::new(spaced).family(theme::bold()).size(ty::LABEL).color(theme::muted())
}
pub fn label(ui: &mut Ui, text: &str) {
    ui.label(caps(text));
}

pub fn heading(text: &str, size: f32) -> RichText {
    RichText::new(text).family(theme::serif()).size(size).color(theme::ink())
}
pub fn body(text: impl Into<String>) -> RichText {
    RichText::new(text).size(ty::BODY).color(theme::text())
}
pub fn meta(text: impl Into<String>) -> RichText {
    RichText::new(text).size(ty::META).color(theme::muted())
}

/// A small rounded chip, painted at a fixed height so it never stretches to the row.
pub fn pill(ui: &mut Ui, text: &str, fill: Color32, fg: Color32) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::new(11.0, theme::bold()), fg);
    let (r, resp) = ui.allocate_exact_size(galley.size() + Vec2::new(16.0, 6.0), Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::same(255), fill);
    ui.painter().galley(r.min + Vec2::new(8.0, 3.0), galley, fg);
    resp
}
pub fn marker_pill(ui: &mut Ui, text: &str) -> Response {
    pill(ui, text, theme::highlighter(), theme::on_highlighter())
}
pub fn quiet_pill(ui: &mut Ui, text: &str) -> Response {
    pill(ui, text, theme::field(), theme::muted())
}

/// An on/off switch with its label to the right. Returns the response of the whole row.
pub fn switch(ui: &mut Ui, on: &mut bool, text: &str) -> Response {
    let enabled = ui.is_enabled();
    let font = FontId::new(ty::BODY, egui::FontFamily::Proportional);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, if enabled { theme::ink() } else { theme::muted() });
    let track = Vec2::new(36.0, 20.0);
    let gap = if text.is_empty() { 0.0 } else { 10.0 };
    let size = Vec2::new(track.x + gap + galley.size().x, track.y.max(galley.size().y).max(28.0));
    let (rect, mut resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.12);
    let tr = Rect::from_min_size(Pos2::new(rect.left(), rect.center().y - track.y / 2.0), track);
    let off_fill = if theme::is_dark() { theme::rule() } else { theme::border() };
    let fill = if enabled { off_fill.lerp_to_gamma(theme::accent(), t) } else { unavailable().0 };
    ui.painter().rect_filled(tr, CornerRadius::same(10), fill);
    let knob_x = egui::lerp(tr.left() + 10.0..=tr.right() - 10.0, t);
    ui.painter().circle_filled(Pos2::new(knob_x, tr.center().y), 7.5, theme::card());
    if resp.hovered() && enabled {
        ui.painter().rect_stroke(tr, CornerRadius::same(10), Stroke::new(1.0, theme::edge()), StrokeKind::Outside);
    }
    if resp.has_focus() {
        focus_ring(ui, tr, 10);
    }
    ui.painter().galley(Pos2::new(tr.right() + gap, rect.center().y - galley.size().y / 2.0), galley, theme::ink());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, *on, text));
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

/// A segmented control. Returns true when the choice changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let h = 34.0;
    let font = FontId::new(ty::BUTTON_SM, theme::semibold());
    let widths: Vec<f32> = options
        .iter()
        .map(|(_, l)| ui.painter().layout_no_wrap(l.to_string(), font.clone(), theme::ink()).size().x + 24.0)
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 6.0;
    let (frame, _) = ui.allocate_exact_size(Vec2::new(total, h), Sense::hover());
    ui.painter().rect(frame, CornerRadius::same(12), theme::field(), Stroke::new(1.0, theme::border()), StrokeKind::Inside);
    let mut x = frame.left() + 3.0;
    let mut changed = false;
    for ((v, l), w) in options.iter().zip(widths) {
        let seg = Rect::from_min_size(Pos2::new(x, frame.top() + 3.0), Vec2::new(w, h - 6.0));
        x += w;
        let resp = ui.interact(seg, ui.id().with(("seg", l)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        let on = *value == *v;
        if on {
            ui.painter().rect(seg, CornerRadius::same(9), theme::card(), Stroke::new(1.0, theme::border()), StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(seg, CornerRadius::same(9), theme::hover_bg());
        }
        let (f, c) = if on { (FontId::new(ty::BUTTON_SM, theme::bold()), theme::accent()) } else { (font.clone(), theme::text()) };
        ui.painter().text(seg.center(), Align2::CENTER_CENTER, *l, f, c);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, on, *l));
        if resp.clicked() && !on {
            *value = *v;
            changed = true;
        }
    }
    changed
}

/// A note set into a card: an accent disc with an icon beside one wrapped line.
pub fn note(ui: &mut Ui, glyph_s: &str, text: &str, tone: Color32) {
    Frame::new()
        .fill(theme::field())
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                glyph(ui.painter(), r.center(), glyph_s, 17.0, tone);
                ui.add(egui::Label::new(body(text).size(ty::META)).wrap());
            });
        });
}

/// The labelled single-line field used in forms: label above, 40 px rounded input.
pub fn field(ui: &mut Ui, title: &str, text: &mut String, hint: &str, password: bool) -> Response {
    ui.label(RichText::new(title).family(theme::semibold()).size(ty::META).color(theme::text()));
    ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .password(password)
            .margin(Margin::symmetric(12, 10))
            .desired_width(ui.available_width().min(460.0)),
    )
}

/// The search box: magnifier, rounded field.
pub fn search(ui: &mut Ui, text: &mut String, hint: &str) -> Response {
    let mut out = None;
    Frame::new()
        .fill(theme::card())
        .stroke(Stroke::new(1.0, theme::border()))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().interact_size.y = 20.0;
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon::MAGNIFYING_GLASS).font(theme::icon_font(16.0)).color(theme::muted()));
                out = Some(ui.add(
                    egui::TextEdit::singleline(text)
                        .frame(Frame::new())
                        .margin(Margin::ZERO)
                        .hint_text(hint)
                        .desired_width(ui.available_width()),
                ));
            });
        });
    out.unwrap()
}

/// An empty area's card: an icon disc, a serif line, a sentence and its buttons.
/// Returns the index of the button pressed.
pub fn empty_state(ui: &mut Ui, glyph_s: &str, title: &str, text: &str, actions: &[(Option<&str>, &str, Tier)]) -> Option<usize> {
    let mut pressed = None;
    card_frame(28).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical_centered(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 32.0, theme::accent_faded());
            glyph(ui.painter(), r.center(), glyph_s, 30.0, theme::accent());
            ui.add_space(10.0);
            ui.label(heading(title, ty::SECTION));
            ui.add(egui::Label::new(body(text).color(theme::muted())).wrap());
            ui.add_space(14.0);
            let widths: f32 = actions.len() as f32 * 150.0;
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width() - widths) / 2.0).max(0.0));
                for (n, (i, l, t)) in actions.iter().enumerate() {
                    if button(ui, *t, *i, l, 40.0).clicked() {
                        pressed = Some(n);
                    }
                }
            });
        });
    });
    pressed
}

/// A divider line inside a card.
pub fn rule(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 9.0), Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, theme::border()));
}

/// A menu button: the button opens a popup anchored under it.
pub fn menu_button(ui: &mut Ui, tier: Tier, glyph_s: Option<&str>, label: &str, add: impl FnOnce(&mut Ui)) -> Response {
    let r = button(ui, tier, glyph_s, label, if tier == Tier::Primary { 40.0 } else { 36.0 });
    egui::Popup::menu(&r).show(|ui| {
        ui.set_min_width(200.0);
        add(ui)
    });
    r
}

/// A row inside a popup menu: icon and label, full width.
pub fn menu_item(ui: &mut Ui, glyph_s: &str, label: &str) -> Response {
    let w = ui.available_width().max(200.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 34.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(8), theme::hover_bg());
    }
    glyph(ui.painter(), Pos2::new(rect.left() + 18.0, rect.center().y), glyph_s, 17.0, theme::ink());
    ui.painter().text(
        Pos2::new(rect.left() + 36.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::new(ty::BODY, theme::semibold()),
        theme::ink(),
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A labelled crop/trim slider: name, slider, value as a percentage.
pub fn slider_row(ui: &mut Ui, name: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>) -> Response {
    ui.horizontal(|ui| {
        ui.add_sized([54.0, 24.0], egui::Label::new(body(name).size(ty::META)));
        // The handle's size follows the control height; 34 px would make a 17 px knob.
        ui.spacing_mut().interact_size.y = 18.0;
        ui.spacing_mut().slider_width = (ui.available_width() - 52.0).max(80.0);
        let r = ui.add(egui::Slider::new(value, range).show_value(false));
        ui.add_sized([40.0, 24.0], egui::Label::new(meta(format!("{:.1}%", *value))));
        r
    })
    .inner
}
