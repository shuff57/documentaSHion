//! verSHion's house theme, shared so documentaSHion reads as the same family: the cream/brown
//! paper palette with a chalkboard-green accent and highlighter-yellow marker, Source Sans and
//! Source Serif, Phosphor icons, and the widget chrome. Values follow verSHion's `src/theme.rs`.
use eframe::egui::{
    self, Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke,
    TextStyle, ThemePreference,
};
use std::{cell::Cell, sync::Arc};

#[derive(Clone, Copy)]
pub struct Palette {
    /// Desk: the whole-app background under the faint graph-paper grid.
    pub cream: Color32,
    /// The left dock.
    pub rail: Color32,
    /// Cards, windows, popovers.
    pub card: Color32,
    /// Inputs and inset areas.
    pub field: Color32,
    /// Hairline borders around cards.
    pub border: Color32,
    /// Stronger dividers.
    pub rule: Color32,
    /// Headings and emphasised text.
    pub ink: Color32,
    /// Body text.
    pub text: Color32,
    /// Captions and counters.
    pub muted: Color32,
    pub accent: Color32,
    pub accent_faded: Color32,
    pub on_accent: Color32,
    /// The active dock item and attention markers. Always dark ink on top.
    pub highlighter: Color32,
    pub hover_bg: Color32,
    pub red: Color32,
    /// The tray a page sits on.
    pub table: Color32,
    pub grid_line: Color32,
}

pub const fn light_palette() -> Palette {
    Palette {
        cream: Color32::from_rgb(0xf7, 0xf4, 0xed),
        rail: Color32::from_rgb(0x33, 0x2a, 0x24),
        card: Color32::from_rgb(0xff, 0xfd, 0xf8),
        field: Color32::from_rgb(0xf3, 0xee, 0xe3),
        border: Color32::from_rgb(0xe0, 0xd3, 0xc2),
        rule: Color32::from_rgb(0xbd, 0xa9, 0x95),
        ink: Color32::from_rgb(0x33, 0x2a, 0x24),
        text: Color32::from_rgb(0x5e, 0x4c, 0x40),
        muted: Color32::from_rgb(0x7a, 0x63, 0x50),
        accent: Color32::from_rgb(0x2f, 0x61, 0x44),
        accent_faded: Color32::from_rgb(0xdf, 0xe9, 0xd4),
        on_accent: Color32::from_rgb(0xf7, 0xfb, 0xf3),
        highlighter: Color32::from_rgb(0xf3, 0xdc, 0x6a),
        hover_bg: Color32::from_rgb(0xef, 0xe8, 0xdb),
        red: Color32::from_rgb(0xa3, 0x3b, 0x2e),
        table: Color32::from_rgb(0xe8, 0xdf, 0xce),
        grid_line: Color32::from_rgba_premultiplied(3, 2, 2, 14),
    }
}

pub const fn dark_palette() -> Palette {
    Palette {
        cream: Color32::from_rgb(0x21, 0x1c, 0x18),
        rail: Color32::from_rgb(0x14, 0x10, 0x0d),
        card: Color32::from_rgb(0x2b, 0x25, 0x20),
        field: Color32::from_rgb(0x21, 0x1c, 0x18),
        border: Color32::from_rgb(0x4b, 0x3f, 0x35),
        rule: Color32::from_rgb(0x6d, 0x5c, 0x4b),
        ink: Color32::from_rgb(0xf2, 0xe8, 0xdd),
        text: Color32::from_rgb(0xd3, 0xc2, 0xb2),
        muted: Color32::from_rgb(0xa9, 0x93, 0x7e),
        accent: Color32::from_rgb(0x9d, 0xcb, 0xa0),
        accent_faded: Color32::from_rgb(0x2d, 0x3b, 0x2c),
        on_accent: Color32::from_rgb(0x0e, 0x20, 0x12),
        highlighter: Color32::from_rgb(0xe9, 0xcf, 0x55),
        hover_bg: Color32::from_rgb(0x38, 0x30, 0x28),
        red: Color32::from_rgb(0xe0, 0x82, 0x6e),
        table: Color32::from_rgb(0x16, 0x12, 0x10),
        grid_line: Color32::from_rgba_premultiplied(2, 2, 2, 11),
    }
}

thread_local! {
    static DARK: Cell<bool> = const { Cell::new(false) };
}

pub fn is_dark() -> bool {
    DARK.with(Cell::get)
}

fn pal() -> Palette {
    if is_dark() { dark_palette() } else { light_palette() }
}

/// Switches palette and restyles the running app.
pub fn set_dark(dark: bool, ctx: &Context) {
    DARK.with(|d| d.set(dark));
    ctx.set_theme(if dark { ThemePreference::Dark } else { ThemePreference::Light });
    ctx.all_styles_mut(apply_style);
    ctx.request_repaint();
}

pub fn cream() -> Color32 { pal().cream }
pub fn rail() -> Color32 { pal().rail }
pub fn card() -> Color32 { pal().card }
pub fn field() -> Color32 { pal().field }
pub fn border() -> Color32 { pal().border }
pub fn rule() -> Color32 { pal().rule }
pub fn ink() -> Color32 { pal().ink }
pub fn text() -> Color32 { pal().text }
pub fn muted() -> Color32 { pal().muted }
pub fn accent() -> Color32 { pal().accent }
pub fn accent_faded() -> Color32 { pal().accent_faded }
pub fn on_accent() -> Color32 { pal().on_accent }
pub fn highlighter() -> Color32 { pal().highlighter }
pub fn hover_bg() -> Color32 { pal().hover_bg }
pub fn red() -> Color32 { pal().red }
pub fn table() -> Color32 { pal().table }
/// Dark ink on the highlighter, in both modes.
pub fn on_highlighter() -> Color32 { Color32::from_rgb(0x33, 0x2a, 0x24) }
/// Dock labels and icons.
pub fn dock_text() -> Color32 { Color32::from_rgb(0xe9, 0xdf, 0xd2) }
/// The 3:1 control edge for outlined buttons and fields.
pub fn edge() -> Color32 {
    if is_dark() { Color32::from_rgb(0x85, 0x72, 0x5f) } else { Color32::from_rgb(0x9c, 0x86, 0x72) }
}
/// Amber for "check this" marks (changed numbers, [unclear], unsure OCR).
pub fn amber() -> Color32 {
    if is_dark() { Color32::from_rgb(0xe0, 0xb0, 0x4c) } else { Color32::from_rgb(0xb0, 0x7a, 0x12) }
}
pub fn amber_wash() -> Color32 {
    if is_dark() { Color32::from_rgb(0x5c, 0x47, 0x1c) } else { Color32::from_rgb(0xf8, 0xe2, 0xa8) }
}

/// The faint 24 px graph-paper grid across `rect`.
pub fn paint_grid(painter: &egui::Painter, rect: egui::Rect) {
    let step = 24.0;
    let stroke = Stroke::new(1.0, pal().grid_line);
    let mut x = (rect.left() / step).floor() * step;
    while x <= rect.right() {
        painter.vline(x, rect.y_range(), stroke);
        x += step;
    }
    let mut y = (rect.top() / step).floor() * step;
    while y <= rect.bottom() {
        painter.hline(rect.x_range(), y, stroke);
        y += step;
    }
}

// ---------------------------------------------------------------- fonts and icons

pub fn serif() -> FontFamily { FontFamily::Name("ds-serif".into()) }
pub fn serif_semibold() -> FontFamily { FontFamily::Name("ds-serif-semibold".into()) }
pub fn bold() -> FontFamily { FontFamily::Name("ds-sans-bold".into()) }
pub fn semibold() -> FontFamily { FontFamily::Name("ds-sans-semibold".into()) }

pub const ICON_FAMILY: &str = "ds-phosphor";
pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(ICON_FAMILY.into()))
}

// Only these icons are compiled in. An icon missing here draws as an empty box; the test
// `every_icon_used_is_in_the_subset` catches it.
egui_phosphor::subset! {
    pub mod icons {
        use regular::{
            ANGLE, ARROW_CLOCKWISE, ARROW_COUNTER_CLOCKWISE, ARROW_LEFT, ARROW_RIGHT, ARROW_U_UP_LEFT,
            BOOKS, CARET_DOWN, CHECK, CIRCLE_HALF, CORNERS_OUT, CROP, EXPORT, FILE_DASHED, FILE_IMAGE,
            FILE_PDF, FILE_TEXT, GEAR, IMAGES, INFO, KEY, LOCK_SIMPLE, MAGNIFYING_GLASS,
            MAGNIFYING_GLASS_MINUS, MAGNIFYING_GLASS_PLUS, MOON, PENCIL_SIMPLE_LINE, PLUS, SCAN,
            SLIDERS_HORIZONTAL, SPARKLE, SUN, TEXT_AA, TRASH, UPLOAD_SIMPLE, USB, WARNING, X,
        };
    }
}
pub use icons::regular as icon;

fn load_fonts() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    let put = |defs: &mut FontDefinitions, name: &str, bytes: &'static [u8]| {
        defs.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
    };
    put(&mut defs, "SourceSans3-Regular", include_bytes!("../../assets/SourceSans3-Regular.ttf"));
    put(&mut defs, "SourceSans3-Semibold", include_bytes!("../../assets/SourceSans3-Semibold.ttf"));
    put(&mut defs, "SourceSans3-Bold", include_bytes!("../../assets/SourceSans3-Bold.ttf"));
    put(&mut defs, "SourceSerif4-Regular", include_bytes!("../../assets/SourceSerif4-Regular.ttf"));
    put(&mut defs, "SourceSerif4-Semibold", include_bytes!("../../assets/SourceSerif4-Semibold.ttf"));
    // egui's own faces (emoji, symbols) stay behind ours for glyphs the Source faces lack.
    let fallback = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let family = |list: &[&str]| -> Vec<String> {
        list.iter().map(|s| s.to_string()).chain(fallback.iter().cloned()).collect()
    };
    defs.families.insert(FontFamily::Proportional, family(&["SourceSans3-Regular"]));
    for (name, list) in [
        ("ds-sans-semibold", &["SourceSans3-Semibold", "SourceSans3-Regular"][..]),
        ("ds-sans-bold", &["SourceSans3-Bold", "SourceSans3-Semibold"]),
        ("ds-serif", &["SourceSerif4-Regular", "SourceSans3-Regular"]),
        ("ds-serif-semibold", &["SourceSerif4-Semibold", "SourceSerif4-Regular"]),
    ] {
        defs.families.insert(FontFamily::Name(name.into()), family(list));
    }
    egui_phosphor::add_font_bytes_to_fonts(&mut defs, ICON_FAMILY, &icons::regular::FONT);
    // Every family reaches the icon face, so an icon inside any label draws.
    for fonts in defs.families.values_mut() {
        if !fonts.iter().any(|f| f == ICON_FAMILY) {
            fonts.insert(fonts.len().min(1), ICON_FAMILY.into());
        }
    }
    defs
}

pub fn setup(ctx: &Context, dark: bool) {
    ctx.set_fonts(load_fonts());
    set_dark(dark, ctx);
}

fn apply_style(style: &mut egui::Style) {
    let p = pal();
    let v = &mut style.visuals;
    v.dark_mode = is_dark();
    v.panel_fill = p.cream;
    v.window_fill = p.card;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(16);
    v.menu_corner_radius = CornerRadius::same(12);
    v.window_shadow = egui::Shadow { offset: [0, 6], blur: 24, spread: 0, color: Color32::from_black_alpha(if is_dark() { 90 } else { 28 }) };
    v.popup_shadow = egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(if is_dark() { 80 } else { 22 }) };
    v.faint_bg_color = p.cream;
    v.extreme_bg_color = p.field;
    v.text_edit_bg_color = Some(p.field);
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.muted;
    v.error_fg_color = p.red;
    v.selection.bg_fill = p.accent_faded;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    let w = &mut v.widgets;
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    for (state, fill, stroke) in [
        (&mut w.inactive, p.card, p.border),
        (&mut w.hovered, p.hover_bg, edge()),
        (&mut w.active, p.accent_faded, p.accent),
        (&mut w.open, p.hover_bg, p.border),
    ] {
        state.weak_bg_fill = fill;
        state.bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, stroke);
        state.fg_stroke = Stroke::new(1.0, p.ink);
        state.corner_radius = CornerRadius::same(12);
        state.expansion = 0.0;
    }
    // Slider rails and checkbox wells.
    w.inactive.bg_fill = p.field;
    w.noninteractive.corner_radius = CornerRadius::same(12);

    style.text_styles.clear();
    let sans = FontFamily::Proportional;
    style.text_styles.insert(TextStyle::Body, FontId::new(14.0, sans.clone()));
    style.text_styles.insert(TextStyle::Button, FontId::new(13.5, bold()));
    style.text_styles.insert(TextStyle::Small, FontId::new(12.0, sans.clone()));
    style.text_styles.insert(TextStyle::Heading, FontId::new(22.0, serif()));
    style.text_styles.insert(TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace));
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 8.0);
    style.spacing.interact_size.y = 34.0;
    style.spacing.slider_rail_height = 6.0;
    style.spacing.scroll = egui::style::ScrollStyle::thin();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_icon_used_is_in_the_subset() {
        let this = include_str!("theme.rs");
        let start = this.find("use regular::{").unwrap() + "use regular::{".len();
        let subset: Vec<&str> = this[start..start + this[start..].find('}').unwrap()]
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = vec![];
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                let src = std::fs::read_to_string(&p).unwrap();
                for (i, _) in src.match_indices("icon::") {
                    let name: String = src[i + 6..]
                        .chars()
                        .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
                        .collect();
                    if !name.is_empty() && name != "FONT" && !subset.contains(&name.as_str()) {
                        missing.push(name);
                    }
                }
            }
        }
        assert!(missing.is_empty(), "add to theme::icons: {missing:?}");
    }
    #[test]
    fn fonts_register_every_family() {
        let defs = load_fonts();
        for fam in [serif(), serif_semibold(), bold(), semibold()] {
            let list = &defs.families[&fam];
            assert!(list.iter().all(|f| defs.font_data.contains_key(f)), "{fam:?}");
        }
    }
}
